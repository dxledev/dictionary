//! Urban Dictionary lookups: fetch the public define API and reduce each entry
//! to the fields the dictionary UI shows.

use crate::model::{curl, encode};
use serde::Deserialize;

const API: &str = "https://api.urbandictionary.com/v0/define?term=";

/// At most this many definitions are kept.
pub const MAX_DEFS: usize = 5;

/// Infer a part of speech when an Urban Dictionary definition gives a clear
/// grammatical cue. Informal definitions often omit one, so ambiguous text
/// intentionally returns `None`.
pub fn infer_part_of_speech(text: &str) -> Option<&'static str> {
    let text = text
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | '['));
    let text = text.to_lowercase().replace(['’', '‘'], "'");
    let text = text.trim_start();

    for (label, variants) in [
        ("noun", &["noun", "n."][..]),
        ("verb", &["verb", "v."][..]),
        ("adjective", &["adjective", "adj."][..]),
        ("adverb", &["adverb", "adv."][..]),
        ("interjection", &["interjection", "interj."][..]),
    ] {
        for variant in variants {
            if let Some(rest) = text.strip_prefix(variant) {
                if rest.starts_with('/') || rest.starts_with(" or ") {
                    return None;
                }
                if rest.is_empty()
                    || rest.starts_with([':', ')', '\n', '—', '–'])
                    || (variant.ends_with('.') && rest.starts_with(' '))
                {
                    return Some(label);
                }
            }
        }
    }
    if text.contains("verb or a noun") || text.contains("noun or a verb") {
        return None;
    }
    if ["to be honest", "to be fair", "to be clear"]
        .iter()
        .any(|p| text.starts_with(p))
    {
        return None;
    }
    if let Some((register, body)) = text.split_once(';') {
        if ["truckers' lingo", "trucker lingo", "internet slang"]
            .contains(&register.trim())
        {
            return infer_part_of_speech(body);
        }
    }
    for wrapper in ["another word for ", "a synonym for ", "slang for "] {
        if let Some(complement) = text.strip_prefix(wrapper) {
            return infer_complement_pos(complement);
        }
    }
    if omitted_article_noun(text) {
        return Some("noun");
    }
    if ["in a ", "in an ", "in the ", "with "]
        .iter()
        .any(|p| text.starts_with(p))
        && [" manner", " way"].iter().any(|p| text.contains(p))
    {
        return Some("adverb");
    }
    let cues: [(&[&str], &str); 5] = [
        (
            &[
                "an interjection meaning ",
                "an exclamation used to ",
                "exclamation: ",
            ],
            "interjection",
        ),
        (&["a verb meaning ", "verb meaning ", "to "], "verb"),
        (
            &[
                "an adjective meaning ",
                "adjective meaning ",
                "describes someone who is ",
                "describes something that is ",
                "having ",
                "characterized by ",
                "of or relating to ",
                "relating to ",
                "full of ",
                "capable of ",
                "tending to ",
                "marked by ",
                "resembling ",
            ],
            "adjective",
        ),
        (&["an adverb meaning ", "adverb meaning "], "adverb"),
        (
            &[
                "a ",
                "an ",
                "the ",
                "one who ",
                "someone who ",
                "something that ",
            ],
            "noun",
        ),
    ];
    if let Some(label) = cues.into_iter().find_map(|(prefixes, label)| {
        prefixes
            .iter()
            .any(|prefix| text.starts_with(prefix))
            .then_some(label)
    }) {
        return Some(label);
    }
    relative_clause_noun(text)
}

fn relative_clause_noun(text: &str) -> Option<&'static str> {
    let clause = text.split(['.', '!', '?', ';', ':', '\n']).next()?;
    let words: Vec<_> = clause
        .split(|c: char| !c.is_ascii_alphabetic() && c != '\'')
        .filter(|word| !word.is_empty()).take(6).collect();
    let relative = words.iter().position(|word| matches!(*word, "who" | "that"))?;
    if !(1..=3).contains(&relative) || non_nominal_opener(words[0]) {
        return None;
    }
    let head = words[relative - 1];
    if !relative_predicate(words.get(relative + 1)?) {
        return None;
    }
    let noun = has_dictionary_pos(head, "noun")
        || head.strip_suffix('s').is_some_and(|singular| has_dictionary_pos(singular, "noun"));
    (noun || (words[relative] == "who" && !non_nominal_opener(head))).then_some("noun")
}

fn non_nominal_opener(word: &str) -> bool {
    matches!(word, "i" | "you" | "he" | "she" | "it" | "we" | "they" | "this" | "these"
        | "those" | "there" | "here" | "what" | "which" | "whatever" | "whoever"
        | "when" | "where" | "why" | "how" | "used" | "means" | "because")
}

fn relative_predicate(word: &str) -> bool {
    if matches!(word, "a" | "an" | "the" | "my" | "your" | "our" | "their" | "his" | "her") {
        return false;
    }
    matches!(word, "am" | "are" | "is" | "was" | "were" | "be" | "been" | "being"
        | "do" | "does" | "did" | "have" | "has" | "had" | "think" | "thinks" | "thought"
        | "make" | "makes" | "made" | "run" | "runs" | "use" | "uses" | "want" | "wants"
        | "say" | "says" | "know" | "knows" | "get" | "gets" | "go" | "goes" | "work" | "works")
        || has_dictionary_pos(word, "verb")
}

fn has_dictionary_pos(word: &str, pos: &str) -> bool {
    crate::model::webster(word).is_some_and(|entry| entry.groups.iter().any(|group| {
        group.pos == pos || (pos == "verb" && (group.pos.starts_with("v.") || group.pos.starts_with("v ")))
    }))
}

fn fallback_is_allowed(text: &str) -> bool {
    let normalized = text.to_lowercase().replace(['’', '‘'], "'");
    let normalized = normalized
        .trim_start_matches(|c: char| matches!(c, '"' | '\'' | '(' | '['));
    let tokens: Vec<_> = normalized
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '.')
        .map(|token| token.trim_end_matches('.'))
        .filter(|token| !token.is_empty())
        .collect();
    let labels = [
        "noun",
        "verb",
        "adjective",
        "adverb",
        "interjection",
        "n",
        "v",
        "adj",
        "adv",
        "interj",
    ];
    let connectors = [
        "or", "and", "a", "an", "both", "either", "maybe", "possibly",
    ];
    let positions: Vec<_> = tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| labels.contains(token))
        .map(|(i, _)| i)
        .collect();
    !positions.windows(2).any(|pair| {
        let between = &tokens[pair[0] + 1..pair[1]];
        between.iter().all(|token| connectors.contains(token))
    })
}

fn omitted_article_noun(text: &str) -> bool {
    let mut words = text.split_whitespace();
    let first = words.next().unwrap_or_default();
    let body = if [
        "fatherless",
        "young",
        "old",
        "long",
        "short",
        "large",
        "small",
    ]
    .contains(&first)
    {
        text.strip_prefix(first).unwrap_or(text).trim_start()
    } else {
        text
    };
    [
        "being who ",
        "person who ",
        "strip of ",
        "piece of ",
        "ability to ",
    ]
    .iter()
    .any(|cue| body.starts_with(cue))
}

fn infer_complement_pos(complement: &str) -> Option<&'static str> {
    let complement = complement.trim_start();
    let nominal = complement.split('/').any(|part| {
        [
            "how good you are with ",
            "how good someone is with ",
            "how well you ",
        ]
        .iter()
        .any(|cue| part.trim_start().starts_with(cue))
    });
    if nominal {
        return Some("noun");
    }
    if ["another word for ", "a synonym for ", "slang for "]
        .iter()
        .any(|cue| complement.starts_with(cue))
    {
        return None;
    }
    infer_part_of_speech(complement)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrbanDef {
    pub word: String,
    pub text: String,
    pub example: Option<String>,
    pub pos: Option<&'static str>,
    pub author: String,
    pub date: String,
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    list: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    definition: Option<String>,
    #[serde(default)]
    example: Option<String>,
    #[serde(default)]
    word: Option<String>,
    #[serde(default)]
    written_on: Option<String>,
}

pub fn urban(word: &str) -> Result<Vec<UrbanDef>, String> {
    if word.is_empty() {
        return Ok(vec![]);
    }
    let body = curl(&format!("{API}{}", encode(&word.to_lowercase())))?;
    let mut defs = parse(&body)?;
    let requests: Vec<_> = defs
        .iter()
        .filter(|def| def.pos.is_none() && fallback_is_allowed(&def.text))
        .map(|def| crate::urban_pos::Request {
            word: if def.word.is_empty() {
                word.to_owned()
            } else {
                def.word.clone()
            },
            example: def.example.clone().unwrap_or_default(),
        })
        .collect();
    let results = crate::urban_pos::classify(&requests);
    let mut results = results.into_iter();
    for def in defs
        .iter_mut()
        .filter(|def| def.pos.is_none() && fallback_is_allowed(&def.text))
    {
        def.pos = results.next().flatten();
    }
    known_pos_first(&mut defs);
    Ok(defs)
}

fn known_pos_first(defs: &mut [UrbanDef]) {
    defs.sort_by_key(|def| def.pos.is_none());
}

pub fn parse(body: &str) -> Result<Vec<UrbanDef>, String> {
    let resp: Response =
        serde_json::from_str(body).map_err(|e| e.to_string())?;
    Ok(resp
        .list
        .into_iter()
        .filter_map(|e| {
            let text = clean(&e.definition.unwrap_or_default());
            if text.is_empty() {
                return None;
            }
            let example = clean(&e.example.unwrap_or_default());
            let pos = infer_part_of_speech(&text);
            Some(UrbanDef {
                word: e.word.unwrap_or_default(),
                text,
                example: (!example.is_empty()).then_some(example),
                pos,
                author: e.author.unwrap_or_default(),
                date: e
                    .written_on
                    .unwrap_or_default()
                    .get(..10)
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .take(MAX_DEFS)
        .collect())
}

/// Strips Urban's bracket link markup, normalizes line endings, and collapses
/// runs of blank lines to one blank line.
pub fn clean(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(normalized.len());
    let mut newlines = 0;
    for c in normalized.chars() {
        if c == '[' || c == ']' {
            continue;
        }
        if c == '\n' {
            newlines += 1;
            if newlines > 2 {
                continue;
            }
        } else {
            newlines = 0;
        }
        out.push(c);
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{"list":[
        {"author":"alice","definition":"[Another word] for [spitting game]\r\n\r\n\r\nok",
         "example":"Person 1: hi [there]\r\nbye","word":"Rizz","written_on":"2022-03-31T10:36:17.000Z","thumbs_up":3},
        {"author":"bob","definition":"   ","example":"skipped","word":"Empty","written_on":"2020-01-01"}
    ]}"#;

    #[test]
    fn parses_and_cleans_fixture() {
        let defs = parse(FIXTURE).unwrap();
        assert_eq!(defs.len(), 1);
        let d = &defs[0];
        assert_eq!(d.word, "Rizz");
        assert_eq!(d.text, "Another word for spitting game\n\nok");
        assert_eq!(d.example.as_deref(), Some("Person 1: hi there\nbye"));
        assert_eq!(d.author, "alice");
        assert_eq!(d.date, "2022-03-31");
    }

    #[test]
    fn empty_list_gives_empty_vec() {
        assert_eq!(parse(r#"{"list":[]}"#).unwrap(), vec![]);
    }

    #[test]
    fn invalid_json_is_err() {
        assert!(parse("not json").is_err());
    }

    #[test]
    fn truncates_to_max_defs_in_api_order() {
        let entries: Vec<String> = (0..7)
            .map(|i| format!(r#"{{"definition":"def {i}","word":"w{i}","written_on":"2021-05-06T00:00:00.000Z"}}"#))
            .collect();
        let body = format!(r#"{{"list":[{}]}}"#, entries.join(","));
        let defs = parse(&body).unwrap();
        assert_eq!(defs.len(), MAX_DEFS);
        assert_eq!(defs[0].word, "w0");
        assert_eq!(defs[MAX_DEFS - 1].word, "w4");
        assert_eq!(defs[0].example, None);
    }

    #[test]
    fn puts_known_pos_first_stably_after_fallback() {
        let mut defs = parse(
            r#"{"list":[
            {"definition":"Unknown first"},
            {"definition":"To leave quickly."},
            {"definition":"Unknown second"},
            {"definition":"Noun: a thing."},
            {"definition":"Adjective: good."}
        ]}"#,
        )
        .unwrap();

        known_pos_first(&mut defs);

        assert_eq!(
            defs.iter()
                .map(|def| (def.text.as_str(), def.pos))
                .collect::<Vec<_>>(),
            [
                ("To leave quickly.", Some("verb")),
                ("Noun: a thing.", Some("noun")),
                ("Adjective: good.", Some("adjective")),
                ("Unknown first", None),
                ("Unknown second", None),
            ]
        );

        defs[4].pos = Some("adjective");
        known_pos_first(&mut defs);
        assert_eq!(
            defs.iter().map(|def| def.text.as_str()).collect::<Vec<_>>(),
            [
                "To leave quickly.",
                "Noun: a thing.",
                "Adjective: good.",
                "Unknown second",
                "Unknown first"
            ]
        );
    }

    #[test]
    fn short_date_is_empty() {
        let defs =
            parse(r#"{"list":[{"definition":"x","written_on":"2020"}]}"#)
                .unwrap();
        assert_eq!(defs[0].date, "");
    }

    #[test]
    fn infers_only_from_clear_definition_openers() {
        assert_eq!(
            infer_part_of_speech("A slang term for someone stylish."),
            Some("noun")
        );
        assert_eq!(infer_part_of_speech("To leave quickly."), Some("verb"));
        assert_eq!(
            infer_part_of_speech("An adjective meaning exceptionally good."),
            Some("adjective")
        );
        assert_eq!(
            infer_part_of_speech("An adverb meaning very loudly."),
            Some("adverb")
        );
        assert_eq!(
            infer_part_of_speech("An interjection meaning surprise."),
            Some("interjection")
        );
    }

    #[test]
    fn relative_clause_with_known_noun_head_is_a_noun() {
        for text in [
            "People who think they know everything.",
            "People who are always late.",
            "Young people who are always late.",
            "Students who think they are right.",
            "Devices that work as intended.",
            "Blorbo who thinks they are special.",
            "A person that does whatever they want.",
        ] {
            assert_eq!(infer_part_of_speech(text), Some("noun"), "{text}");
        }
        assert_eq!(
            infer_part_of_speech("To think that people are wrong."),
            Some("verb")
        );
        assert_eq!(
            infer_part_of_speech("I think that people are wrong."),
            None
        );
        assert_eq!(infer_part_of_speech("People that beautiful."), None);
        assert_eq!(
            infer_part_of_speech("To make devices that work."),
            Some("verb")
        );
        assert_eq!(
            infer_part_of_speech(
                "I think that people who are wrong are annoying."
            ),
            None
        );
        assert_eq!(
            infer_part_of_speech("Some definition that the person mentioned."),
            None
        );
        assert_eq!(infer_part_of_speech("I know who thinks that."), None);
        assert_eq!(infer_part_of_speech("People. Who are they?"), None);
    }

    #[test]
    fn declines_ambiguous_or_unmarked_definitions() {
        assert_eq!(
            infer_part_of_speech("Cool, awesome, or really good."),
            None
        );
        assert_eq!(infer_part_of_speech("To be honest, I disagree."), None);
        assert_eq!(infer_part_of_speech("noun/verb: something"), None);
        assert_eq!(infer_part_of_speech("noun-like expression"), None);
        assert_eq!(infer_part_of_speech("Noun: a thing"), Some("noun"));
        assert_eq!(infer_part_of_speech("In a quick manner."), Some("adverb"));
        assert_eq!(
            infer_part_of_speech("Characterized by heat."),
            Some("adjective")
        );
        assert_eq!(infer_part_of_speech("A large mammal."), Some("noun"));
        assert_eq!(
            infer_part_of_speech("Used to describe a person who runs."),
            None
        );
        assert_eq!(
            infer_part_of_speech("A weird word, maybe a verb or a noun."),
            None
        );
        assert_eq!(
            infer_part_of_speech("  \"The act of making a scene.\""),
            Some("noun")
        );
        assert_eq!(
            infer_part_of_speech(
                "Fatherless being who thinks they get whatever they want."
            ),
            Some("noun")
        );
        assert_eq!(
            infer_part_of_speech(
                "Truckers’ lingo; long strip of tire tread left behind after a blowout, usually a recapped tire."
            ),
            Some("noun")
        );
        assert_eq!(
            infer_part_of_speech(
                "Another word for spitting game/how good you are with pulling."
            ),
            Some("noun")
        );
        assert_eq!(
            infer_part_of_speech("Another word for to spit game."),
            Some("verb")
        );
        assert_eq!(infer_part_of_speech("Rich Jizz"), None);
        assert_eq!(infer_part_of_speech("NYC Slang created by someone."), None);
        assert_eq!(infer_part_of_speech("Another word for something."), None);
    }

    #[test]
    fn register_and_synonym_cues_do_not_override_ambiguous_prose() {
        for text in [
            "Truckers’ lingo; long strip of tire tread.",
            "A synonym for the act of leaving.",
            "Young person who drives.",
        ] {
            assert_eq!(infer_part_of_speech(text), Some("noun"), "{text}");
        }
        assert_eq!(
            infer_part_of_speech("Truckers' lingo; to drive without stopping."),
            Some("verb")
        );
        for text in [
            "Being rude to someone.",
            "Long enough to reach.",
            "Some background; a person who runs.",
            "Another word for beautiful.",
            "Another word for another word for to go.",
        ] {
            assert_eq!(infer_part_of_speech(text), None, "{text}");
        }
        let defs = parse(r#"{"list":[{"definition":"[Another word] for spitting game/how good you are with flirting.\r\n"}]}"#).unwrap();
        assert_eq!(infer_part_of_speech(&defs[0].text), Some("noun"));
    }

    #[test]
    fn fallback_requires_unresolved_unambiguous_definitions() {
        assert_eq!(infer_part_of_speech("A person who runs."), Some("noun"));
        assert!(!fallback_is_allowed("noun/verb: a word used in many ways"));
        assert!(!fallback_is_allowed("Could be a noun or a verb."));
        assert!(!fallback_is_allowed("Maybe adjective and verb."));
        assert!(!fallback_is_allowed("Used as a noun & verb."));
        assert!(fallback_is_allowed("Someone who is quick."));
    }

    #[test]
    #[ignore]
    fn network_lookup_returns_results() {
        let defs = urban("rizz").unwrap();
        assert!(!defs.is_empty());
    }
}
