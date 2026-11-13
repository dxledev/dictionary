use std::collections::{BTreeSet, HashSet};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::model::{self, Entry};
use crate::urban::UrbanDef;

const STOP_WORDS: &str = "a an and are as at be been being by can could did do does each for from had has have he her him his how i if in into is it its may more most much no not of on one or other our out same she should so some such than that the their them then there these they this those through to too two under up upon us used using very was we were what when where which who will with would you your also any especially etc having often said see something someone thing usually";

#[derive(Clone)]
pub struct Profile {
    word: String,
    definitions: Vec<BTreeSet<String>>,
    parts_of_speech: BTreeSet<String>,
    phonetic: String,
}

fn tokens(text: &str) -> BTreeSet<String> {
    static STOPS: OnceLock<HashSet<&str>> = OnceLock::new();
    let stops = STOPS.get_or_init(|| STOP_WORDS.split_whitespace().collect());
    text.split(|c: char| !c.is_alphabetic())
        .map(str::to_lowercase)
        .filter(|word| word.len() > 2 && !stops.contains(word.as_str()))
        .collect()
}

impl Profile {
    pub fn new(word: &str, entry: Option<&Entry>, urban: &[UrbanDef]) -> Self {
        let mut profile = Self {
            word: word.trim().to_lowercase(),
            definitions: Vec::new(),
            parts_of_speech: BTreeSet::new(),
            phonetic: String::new(),
        };
        if let Some(entry) = entry {
            profile.phonetic = entry.phonetic.clone();
            for group in &entry.groups {
                profile.parts_of_speech.insert(group.pos.clone());
                profile.definitions.extend(
                    group.defs.iter().take(6).map(|def| tokens(&def.text)),
                );
            }
        }
        for def in urban {
            profile.definitions.push(tokens(&def.text));
            if let Some(pos) = &def.pos {
                profile.parts_of_speech.insert((*pos).to_owned());
            }
        }
        profile
    }

    fn score(&self, candidate: &Entry) -> f32 {
        let word = candidate.word.to_lowercase();
        if word == self.word
            || word.chars().count() > 40
            || word.chars().any(char::is_control)
        {
            return 0.;
        }
        let mut best = 0_f32;
        for group in &candidate.groups {
            for def in group.defs.iter().take(6) {
                let candidate_tokens = tokens(&def.text);
                for query_tokens in &self.definitions {
                    let shared =
                        query_tokens.intersection(&candidate_tokens).count();
                    let reference = query_tokens.contains(&word)
                        || candidate_tokens.contains(&self.word);
                    if shared < 2 && !reference {
                        continue;
                    }
                    let overlap = shared as f32
                        / ((query_tokens.len().max(1)
                            * candidate_tokens.len().max(1))
                            as f32)
                            .sqrt();
                    let score = overlap * 10. + if reference { 2. } else { 0. };
                    best = best.max(score);
                }
            }
        }
        if best < 1.5 {
            return 0.;
        }
        if candidate.groups.iter().any(|group| {
            !group.pos.is_empty() && self.parts_of_speech.contains(&group.pos)
        }) {
            best += 0.3;
        }
        if !self.phonetic.is_empty() && candidate.phonetic == self.phonetic {
            best += 0.2;
        }
        let prefix = self
            .word
            .chars()
            .zip(word.chars())
            .take_while(|(a, b)| a == b)
            .count();
        if prefix >= 4 {
            best += 0.2;
        }
        best
    }
}

pub fn limit() -> usize {
    std::env::var("DICTIONARY_SIMILAR_LIMIT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(12)
        .min(24)
}

fn pool_size(limit: usize) -> usize {
    std::env::var("DICTIONARY_SIMILAR_POOL")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(96)
        .clamp(limit, 192)
}

fn select_words(
    mut ranked: Vec<(f32, String)>,
    limit: usize,
    random: &mut fastrand::Rng,
) -> Vec<String> {
    let mut words = Vec::with_capacity(limit.min(ranked.len()));
    while words.len() < limit && !ranked.is_empty() {
        let total: f64 =
            ranked.iter().map(|(score, _)| f64::from(*score)).sum();
        let mut draw = random.f64() * total;
        let index = ranked
            .iter()
            .position(|(score, _)| {
                draw -= f64::from(*score);
                draw < 0.
            })
            .unwrap_or(ranked.len() - 1);
        words.push(ranked.swap_remove(index).1);
    }
    words
}

fn retain_ranked(
    ranked: &mut Vec<(f32, String)>,
    word: String,
    score: f32,
    pool_size: usize,
) {
    if pool_size == 0 {
        return;
    }
    if ranked.iter().any(|(_, existing)| existing == &word) {
        return;
    }
    if ranked.len() < pool_size {
        ranked.push((score, word));
        return;
    }
    let weakest = ranked
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            a.0.total_cmp(&b.0).then_with(|| b.1.cmp(&a.1))
        })
        .map(|(index, _)| index)
        .expect("non-empty bounded pool");
    let (weak_score, weak_word) = &ranked[weakest];
    if score
        .total_cmp(weak_score)
        .then_with(|| weak_word.cmp(&word))
        .is_gt()
    {
        ranked[weakest] = (score, word);
    }
}

pub fn find(
    profile: &Profile,
    limit: usize,
    cancelled: &AtomicBool,
) -> Vec<String> {
    if limit == 0 || profile.definitions.is_empty() {
        return Vec::new();
    }
    let mut ranked = Vec::new();
    let pool_size = pool_size(limit);
    model::scan_webster(|entry| {
        if cancelled.load(Ordering::Relaxed) {
            return false;
        }
        let score = profile.score(&entry);
        if score > 0. {
            let word = entry.word.to_lowercase();
            retain_ranked(&mut ranked, word, score, pool_size);
        }
        true
    });
    if cancelled.load(Ordering::Relaxed) {
        return Vec::new();
    }
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    select_words(ranked, limit, &mut fastrand::Rng::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Def, Group};

    fn entry(word: &str, definition: &str) -> Entry {
        Entry {
            word: word.into(),
            phonetic: String::new(),
            source: "fixture",
            groups: vec![Group {
                pos: "noun".into(),
                defs: vec![Def {
                    text: definition.into(),
                    example: None,
                }],
            }],
        }
    }

    #[test]
    fn meaning_beats_spelling_and_grammar_alone() {
        let query = entry("cat", "A small domestic animal with soft fur.");
        let profile = Profile::new("cat", Some(&query), &[]);
        assert!(
            profile.score(&entry(
                "kitten",
                "A young domestic animal with soft fur."
            )) > profile
                .score(&entry("cattle", "Farm animals raised for meat."))
        );
        assert_eq!(
            profile
                .score(&entry("CAT", "A small domestic animal with soft fur.")),
            0.
        );
        assert_eq!(
            profile.score(&entry("chair", "A thing used for sitting.")),
            0.
        );
    }

    #[test]
    fn urban_definitions_contribute_without_source_bias() {
        let urban = UrbanDef {
            word: "slang".into(),
            text: "A small domestic animal with soft fur.".into(),
            example: None,
            pos: Some("noun"),
            author: String::new(),
            date: String::new(),
        };
        let profile = Profile::new("slang", None, &[urban]);
        assert!(
            profile.score(&entry(
                "kitten",
                "A young domestic animal with soft fur."
            )) > 0.
        );
    }

    #[test]
    fn bundled_words_produce_relevant_bounded_results() {
        let entry = model::webster("happy").unwrap();
        let profile = Profile::new("happy", Some(&entry), &[]);
        let words = find(&profile, 12, &AtomicBool::new(false));
        assert!(!words.is_empty());
        for word in &words {
            assert!(profile.score(&model::webster(word).unwrap()) > 0.);
        }
        assert!(words.len() <= 12);
        assert!(!words.iter().any(|word| word == "happy"));
        assert_eq!(words.iter().collect::<BTreeSet<_>>().len(), words.len());
    }

    #[test]
    fn random_selection_varies_membership_and_favors_stronger_matches() {
        let candidates = vec![
            (10., "closest".to_owned()),
            (5., "related".to_owned()),
            (2., "looser".to_owned()),
        ];
        let selections: Vec<_> = (0..1000)
            .map(|seed| {
                select_words(
                    candidates.clone(),
                    2,
                    &mut fastrand::Rng::with_seed(seed),
                )
            })
            .collect();
        assert!(
            selections
                .iter()
                .any(|words| !words.contains(&"closest".into()))
        );
        assert!(
            selections
                .iter()
                .any(|words| words.contains(&"looser".into()))
        );
        assert!(
            selections
                .iter()
                .all(|words| words.len() == 2 && words[0] != words[1])
        );
        let count = |word: &str| {
            selections
                .iter()
                .filter(|words| words.iter().any(|w| w == word))
                .count()
        };
        assert!(count("closest") > count("related"));
        assert!(count("related") > count("looser"));
    }

    #[test]
    fn cancelled_scan_and_empty_profiles_return_nothing() {
        let query = entry("cat", "A domestic animal.");
        let profile = Profile::new("cat", Some(&query), &[]);
        assert!(find(&profile, 12, &AtomicBool::new(true)).is_empty());
        assert!(
            find(
                &Profile::new("missing", None, &[]),
                12,
                &AtomicBool::new(false)
            )
            .is_empty()
        );
    }

    #[test]
    fn bounded_pool_keeps_the_best_scores_and_lexical_ties() {
        let mut ranked = Vec::new();
        retain_ranked(&mut ranked, "zulu".into(), 1., 2);
        retain_ranked(&mut ranked, "bravo".into(), 1., 2);
        retain_ranked(&mut ranked, "alpha".into(), 1., 2);
        retain_ranked(&mut ranked, "best".into(), 2., 2);
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        assert_eq!(ranked, vec![(2., "best".into()), (1., "alpha".into())]);
    }

    #[test]
    fn bounded_pool_ignores_duplicates_already_retained() {
        let mut ranked = Vec::new();
        retain_ranked(&mut ranked, "kept".into(), 1., 2);
        retain_ranked(&mut ranked, "other".into(), 1., 2);
        retain_ranked(&mut ranked, "kept".into(), 3., 2);
        assert_eq!(ranked.len(), 2);
        assert!(
            ranked
                .iter()
                .any(|(score, word)| word == "kept" && *score == 1.)
        );
    }
}
