//! Looking words up: an offline English edition (Webster's 1913, embedded and
//! decompressed one letter at a time) and Wiktionary editions over the
//! network, tried in order, with "did you mean" suggestions on a miss.

use std::fmt;
use std::io::Read as _;
use std::process::{Command, Stdio};

use flate2::read::GzDecoder;
use serde::Deserialize;
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, Visitor};

/// The Wiktionary editions offered, as (code, English name, section name).
/// Only the editions whose section titles are not English need a native
/// name to find the right section.
pub const LANGS: [(&str, &str, &str); 23] = [
    ("ar", "Arabic", "Arabic"),
    ("bn", "Bengali", "Bengali"),
    ("zh", "Chinese", "Chinese"),
    ("nl", "Dutch", "Dutch"),
    ("en", "English", "English"),
    ("fr", "French", "French"),
    ("de", "German", "German"),
    ("hi", "Hindi", "Hindi"),
    ("id", "Indonesian", "Indonesian"),
    ("it", "Italian", "Italian"),
    ("ja", "Japanese", "日本語"),
    ("ko", "Korean", "한국어"),
    ("ms", "Malay", "Malay"),
    ("fa", "Persian", "Persian"),
    ("pl", "Polish", "Polish"),
    ("pt", "Portuguese", "Portuguese"),
    ("ru", "Russian", "Russian"),
    ("es", "Spanish", "Spanish"),
    ("sw", "Swahili", "Swahili"),
    ("sv", "Swedish", "Swedish"),
    ("th", "Thai", "ภาษาไทย"),
    ("tr", "Turkish", "Turkish"),
    ("vi", "Vietnamese", "Vietnamese"),
];

/// Each edition's name for its own language, as its section headings spell it.
const AUTONYMS: [(&str, &str); 23] = [
    ("ar", "العربية"), ("bn", "বাংলা"), ("zh", "汉语"), ("nl", "Nederlands"), ("en", "English"),
    ("fr", "Français"), ("de", "Deutsch"), ("hi", "हिन्दी"), ("id", "Indonesia"), ("it", "Italiano"),
    ("ja", "日本語"), ("ko", "한국어"), ("ms", "Melayu"), ("fa", "فارسی"), ("pl", "polski"),
    ("pt", "Português"), ("ru", "Русский"), ("es", "Español"), ("sw", "Kiswahili"), ("sv", "Svenska"),
    ("th", "ภาษาไทย"), ("tr", "Türkçe"), ("vi", "Tiếng Việt"),
];

pub fn lang_name(code: &str) -> &'static str {
    LANGS.iter().find(|l| l.0 == code).map_or("English", |l| l.1)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub pos: String,
    pub defs: Vec<Def>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Def {
    pub text: String,
    pub example: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub word: String,
    pub phonetic: String,
    pub source: &'static str,
    pub groups: Vec<Group>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Found(Entry),
    Missing { suggestions: Vec<String> },
    Offline { suggestions: Vec<String> },
}

impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.word)
    }
}

/// Look `word` up in `lang`: Webster's 1913 first for English, then
/// Wiktionary; a miss everywhere yields suggestions.
pub fn lookup(word: &str, lang: &str) -> Outcome {
    let word = word.trim();
    if lang == "en" {
        if let Some(entry) = webster(word) {
            return Outcome::Found(entry);
        }
    }
    match wiktionary(word, lang) {
        Ok(Some(entry)) => Outcome::Found(entry),
        Ok(None) => Outcome::Missing { suggestions: suggest(word) },
        Err(_) => Outcome::Offline { suggestions: suggest(word) },
    }
}

// ---------------------------------------------------------------- Webster

macro_rules! buckets {
    ($($name:literal),*) => {
        fn bucket_data(name: &str) -> &'static [u8] {
            match name {
                $($name => include_bytes!(concat!("../data/webster/", $name, ".json.gz")),)*
                _ => include_bytes!("../data/webster/other.json.gz"),
            }
        }
    };
}
buckets!(
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o",
    "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z", "other"
);

#[derive(Deserialize)]
struct WebsterRecord {
    w: String,
    #[serde(default)]
    pr: String,
    #[serde(default)]
    pos: Vec<(String, Vec<String>)>,
}

/// Picks one record out of a letter's JSON object while skipping the rest
/// without building them, so a lookup never holds a whole letter in memory.
struct Pick<'a>(&'a str);

impl<'de> DeserializeSeed<'de> for Pick<'_> {
    type Value = Option<WebsterRecord>;
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for Pick<'_> {
    type Value = Option<WebsterRecord>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a map of headwords")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut found = None;
        while let Some(key) = map.next_key::<String>()? {
            if key == self.0 && found.is_none() {
                found = Some(map.next_value::<WebsterRecord>()?);
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(found)
    }
}

fn webster_key(word: &str) -> String {
    word.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn webster_pos(abbrev: &str) -> String {
    let a = abbrev.trim().trim_end_matches('.');
    match a {
        "n" | "prop. n" | "prop n" => "noun".into(),
        "a" => "adjective".into(),
        "adv" => "adverb".into(),
        "prep" => "preposition".into(),
        "pron" => "pronoun".into(),
        "conj" => "conjunction".into(),
        "interj" => "interjection".into(),
        "art" => "article".into(),
        _ if a == "v" || a.starts_with("v ") => "verb".into(),
        _ => a.to_owned(),
    }
}

pub fn webster(word: &str) -> Option<Entry> {
    let key = webster_key(word);
    let first = key.chars().next()?;
    let bucket = if first.is_ascii_lowercase() { first.to_string() } else { "other".into() };
    let reader = GzDecoder::new(bucket_data(&bucket));
    let mut de = serde_json::Deserializer::from_reader(reader);
    let record = Pick(&key).deserialize(&mut de).ok()??;
    record_entry(record)
}

fn record_entry(record: WebsterRecord) -> Option<Entry> {
    let groups: Vec<Group> = record
        .pos
        .into_iter()
        .filter_map(|(abbrev, defs)| {
            let defs: Vec<Def> = defs
                .into_iter()
                .map(|d| d.trim().to_owned())
                .filter(|d| !d.is_empty())
                .map(|text| Def { text, example: None })
                .collect();
            (!defs.is_empty()).then(|| Group { pos: webster_pos(&abbrev), defs })
        })
        .collect();
    (!groups.is_empty()).then(|| Entry {
        word: record.w,
        phonetic: record.pr.trim_matches(|c| c == '(' || c == ')').to_owned(),
        source: "Webster's 1913",
        groups,
    })
}

pub(crate) fn scan_webster(mut visit: impl FnMut(Entry) -> bool) {
    struct Scan<'a, F>(&'a mut F);
    impl<'de, F: FnMut(Entry) -> bool> Visitor<'de> for Scan<'_, F> {
        type Value = bool;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a map of headwords")
        }

        fn visit_map<A: MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<bool, A::Error> {
            while map.next_key::<IgnoredAny>()?.is_some() {
                if let Some(entry) =
                    record_entry(map.next_value::<WebsterRecord>()?)
                {
                    if !(self.0)(entry) {
                        return Ok(false);
                    }
                }
            }
            Ok(true)
        }
    }

    for bucket in ('a'..='z')
        .map(|letter| letter.to_string())
        .chain(["other".into()])
    {
        let mut decoder = serde_json::Deserializer::from_reader(
            GzDecoder::new(bucket_data(&bucket)),
        );
        if !matches!(
            serde::Deserializer::deserialize_map(
                &mut decoder,
                Scan(&mut visit)
            ),
            Ok(true)
        ) {
            break;
        }
    }
}

// -------------------------------------------------------------- Wiktionary

pub(crate) fn curl(url: &str) -> Result<String, String> {
    let mut child = Command::new("curl")
        .args(["-fsS", "--max-time", "6", "-A", concat!("dictionary/", env!("CARGO_PKG_VERSION")), url])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("curl is required for online editions: {e}"))?;
    let mut body = String::new();
    child.stdout.take().ok_or("no output")?.read_to_string(&mut body).map_err(|e| e.to_string())?;
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() { Ok(body) } else { Err(format!("request failed ({status})")) }
}

pub(crate) fn encode(word: &str) -> String {
    word.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn wiktionary(word: &str, lang: &str) -> Result<Option<Entry>, String> {
    if word.is_empty() {
        return Ok(None);
    }
    if lang == "en" {
        if let Some(entry) = wiktionary_rest(word)? {
            return Ok(Some(entry));
        }
    }
    let url = format!(
        "https://{lang}.wiktionary.org/w/api.php?action=query&prop=extracts&explaintext=1&redirects=1&format=json&titles={}",
        encode(word)
    );
    parse_extract(word, lang, &curl(&url)?)
}

fn wiktionary_rest(word: &str) -> Result<Option<Entry>, String> {
    let url = format!("https://en.wiktionary.org/api/rest_v1/page/definition/{}", encode(word));
    let body = match curl(&url) {
        Ok(body) => body,
        // The endpoint answers 404 for an unknown word; that is a miss, not
        // an outage, so let the extract endpoint decide.
        Err(_) => return Ok(None),
    };
    let mut entry = parse_rest(word, &body);
    if let Some(entry) = &mut entry {
        entry.phonetic = pronunciation(word, "en").ok().flatten().unwrap_or_default();
    }
    Ok(entry)
}

/// Fetch IPA from the requested language's pronunciation section. This is
/// separate from the definition endpoint because its response omits IPA.
pub fn pronunciation(word: &str, lang: &str) -> Result<Option<String>, String> {
    if word.trim().is_empty() {
        return Ok(None);
    }
    let url = format!(
        "https://{lang}.wiktionary.org/w/api.php?action=query&prop=extracts&explaintext=1&redirects=1&format=json&titles={}",
        encode(word)
    );
    let body = curl(&url)?;
    parse_pronunciation(&body, lang)
}

fn parse_pronunciation(body: &str, lang: &str) -> Result<Option<String>, String> {
    let value: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let pages = value.pointer("/query/pages").and_then(|p| p.as_object()).ok_or("unexpected response")?;
    let Some(page) = pages.values().next() else { return Ok(None) };
    let text = page.get("extract").and_then(|e| e.as_str()).unwrap_or_default();
    let sections = parse_sections(text);
    let language = language_section(&sections, lang);
    let Some(language) = language else { return Ok(None) };
    let mut phonetic = String::new();
    find_language_ipa(language, &mut phonetic);
    Ok((!phonetic.is_empty()).then_some(phonetic))
}

fn language_section<'a>(sections: &'a [Section], lang: &str) -> Option<&'a Section> {
    let native = LANGS.iter().find(|l| l.0 == lang);
    let autonym = AUTONYMS.iter().find(|a| a.0 == lang).map(|a| a.1.to_lowercase());
    let matches = |title: &str| {
        let title = title.to_lowercase();
        native.is_some_and(|n| title.contains(&n.1.to_lowercase()) || title.contains(&n.2.to_lowercase()))
            || autonym.as_ref().is_some_and(|a| title.contains(a.as_str()))
    };
    sections.iter().find(|s| s.level <= 2 && matches(&s.title))
}

fn find_language_ipa(section: &Section, phonetic: &mut String) {
    for child in &section.children {
        let title = child.title.to_lowercase();
        if title.starts_with("pronunciation") || title.starts_with("prononciation") || title.starts_with("aussprache") {
            if phonetic.is_empty() {
                *phonetic = find_ipa(&child.body);
            }
        }
        if phonetic.is_empty() {
            find_language_ipa(child, phonetic);
        }
    }
}

pub fn parse_rest(word: &str, body: &str) -> Option<Entry> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let object = value.as_object()?;
    let sections = object.get("en").or_else(|| object.values().next())?.as_array()?;
    let groups: Vec<Group> = sections
        .iter()
        .filter_map(|section| {
            let pos = section.get("partOfSpeech")?.as_str()?.to_lowercase();
            let defs: Vec<Def> = section
                .get("definitions")?
                .as_array()?
                .iter()
                .filter_map(|d| {
                    let text = strip_html(d.get("definition")?.as_str()?);
                    let example = d
                        .get("examples")
                        .and_then(|e| e.as_array())
                        .and_then(|e| e.first())
                        .and_then(|e| e.as_str())
                        .map(strip_html)
                        .filter(|e| !e.is_empty());
                    (!text.is_empty()).then_some(Def { text, example })
                })
                .collect();
            (!defs.is_empty()).then_some(Group { pos, defs })
        })
        .collect();
    (!groups.is_empty()).then(|| Entry {
        word: word.to_owned(),
        phonetic: String::new(),
        source: "Wiktionary",
        groups,
    })
}

pub fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct Section {
    level: usize,
    title: String,
    body: String,
    children: Vec<Section>,
}

/// Build the `==`/`===` heading tree of a plain-text extract.
fn parse_sections(text: &str) -> Vec<Section> {
    let mut stack: Vec<Section> = vec![Section { level: 1, title: String::new(), body: String::new(), children: Vec::new() }];
    for line in text.lines() {
        let trimmed = line.trim();
        let level = trimmed.bytes().take_while(|b| *b == b'=').count();
        if (1..=5).contains(&level) && trimmed.len() > level * 2 && trimmed.ends_with(&"=".repeat(level)) {
            let title = trimmed[level..trimmed.len() - level].trim().to_owned();
            while stack.len() > 1 && stack.last().is_some_and(|s| s.level >= level) {
                let done = stack.pop().expect("non-empty");
                stack.last_mut().expect("root").children.push(done);
            }
            stack.push(Section { level, title, body: String::new(), children: Vec::new() });
        } else if stack.len() > 1 {
            let top = stack.last_mut().expect("non-empty");
            top.body.push_str(line);
            top.body.push('\n');
        }
    }
    while stack.len() > 1 {
        let done = stack.pop().expect("non-empty");
        stack.last_mut().expect("root").children.push(done);
    }
    stack.pop().map(|r| r.children).unwrap_or_default()
}

/// Headings that never hold definitions, in the editions' own languages.
/// Matching is by prefix of the lowercased heading, digits stripped.
const SKIP_PREFIXES: [&str; 89] = [
    "translation", "derived", "related", "descendant", "reference", "further", "anagram",
    "conjugat", "declension", "inflection", "see also", "external", "quotation", "homophone",
    "hyponym", "hypernym", "synonym", "antonym", "usage", "alternative", "etymolog", "pronunciation",
    "trivia", "notes", "étymolog", "prononciation", "variantes", "hyperonym", "hyponyme", "dérivés",
    "voir aussi", "références", "apparentés", "vocabulaire", "traductions", "holonym", "méronym",
    "proverbes", "anagrammes", "herkunft", "aussprache", "übersetzung", "wortbildung", "redewendung",
    "beispiele", "gegenwörter", "oberbegriffe", "unterbegriffe", "referenzen", "charakteristische",
    "etimolog", "pronunciaci", "морфолог", "произношение", "семантическ", "этимолог", "синоним",
    "антоним", "родствен", "перевод", "字源", "熟語", "アクセント", "発音", "関連", "翻訳", "語源", "派生",
    "worttrennung", "hörbeispiele", "reime", "quellen", "véase", "deklinierte", "proverbi", "sinônimo", "sinonimo", "uitspraak", "afbreking", "synoniem", "etymologi", "przysłow", "verbetes", "na wikip", "gangbaar", "wymowa", "odmiana", "skład", "kolokacje",
];

fn is_citation(line: &str) -> bool {
    let l = line.trim();
    let year = l.chars().take(5).count() == 5 && l.chars().take(4).all(|c| c.is_ascii_digit());
    let label = l.split_once(':').is_some_and(|(head, _)| head.chars().count() <= 14 && !head.contains(' ') && !head.starts_with('['));
    year || label || l.contains(" — (") || l.starts_with("Synonym") || l.starts_with("Antonym") || l.starts_with("—")
}

/// Definitions out of one part-of-speech section: the first line is the
/// headword line, quotations and attributions are dropped.
fn extract_defs(body: &str) -> Vec<Def> {
    body.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .skip(1)
        .filter(|l| !is_citation(l) && !l.chars().all(|c| c.is_ascii_digit()) && l.chars().count() <= 260)
        .take(6)
        .map(|l| Def { text: l.to_owned(), example: None })
        .collect()
}

/// Senses from a section with no headword line: German lists them under
/// "Bedeutungen:", Russian puts examples after a diamond, and the rest are
/// plain lines.
fn extract_loose(body: &str) -> Vec<Def> {
    let lines: Vec<&str> = body.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let is_label = |l: &str| l.ends_with(':') && l.chars().count() < 40;
    let picked: Vec<&str> = match lines.iter().position(|l| l.starts_with("Bedeutungen:")) {
        Some(at) => lines[at + 1..].iter().copied().take_while(|l| !is_label(l)).collect(),
        None => lines.iter().copied().filter(|l| !is_label(l)).collect(),
    };
    picked
        .into_iter()
        .filter(|l| !is_citation(l) && !l.chars().all(|c| c.is_ascii_digit()))
        .map(|l| match l.split_once(" ◆ ") {
            Some((text, example)) => Def { text: text.to_owned(), example: Some(example.to_owned()) },
            None => Def { text: l.to_owned(), example: None },
        })
        .filter(|d| d.text.chars().count() <= 260)
        .take(6)
        .collect()
}

fn find_ipa(body: &str) -> String {
    for marker in ["IPA", "API", "\\"] {
        if let Some(at) = body.find(marker) {
            let rest = &body[at..];
            if let Some(start) = rest.find('/') {
                if let Some(len) = rest[start + 1..].find('/') {
                    return format!("/{}/", &rest[start + 1..start + 1 + len]);
                }
            }
        }
    }
    String::new()
}

pub fn parse_extract(word: &str, lang: &str, body: &str) -> Result<Option<Entry>, String> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let pages = value.pointer("/query/pages").and_then(|p| p.as_object()).ok_or("unexpected response")?;
    let Some(page) = pages.values().next() else { return Ok(None) };
    if page.get("missing").is_some() {
        return Ok(None);
    }
    let text = page.get("extract").and_then(|e| e.as_str()).unwrap_or_default();
    if text.trim().is_empty() {
        return Ok(None);
    }
    let sections = parse_sections(text);
    let native = LANGS.iter().find(|l| l.0 == lang);
    let autonym = AUTONYMS.iter().find(|a| a.0 == lang).map(|a| a.1.to_lowercase());
    let matches = |title: &str| {
        let title = title.to_lowercase();
        native.is_some_and(|n| title.contains(&n.1.to_lowercase()) || title.contains(&n.2.to_lowercase()))
            || autonym.as_ref().is_some_and(|a| title.contains(a.as_str()))
    };
    let language = sections
        .iter()
        .find(|s| s.level <= 2 && matches(&s.title))
        .or_else(|| sections.iter().find(|s| s.level <= 2));
    let Some(language) = language else { return Ok(None) };
    let mut groups = Vec::new();
    let mut phonetic = String::new();
    visit(language, word, false, &mut groups, &mut phonetic);
    if groups.is_empty() {
        // Editions that do not open a section with its headword.
        visit(language, word, true, &mut groups, &mut phonetic);
    }
    groups.truncate(8);
    Ok((!groups.is_empty()).then(|| Entry { word: word.to_owned(), phonetic, source: "Wiktionary", groups }))
}

fn visit(section: &Section, word: &str, loose: bool, groups: &mut Vec<Group>, phonetic: &mut String) {
    for child in &section.children {
        let title = child.title.to_lowercase();
        let base = title.trim_end_matches(|c: char| c.is_ascii_digit() || c == ' ').to_owned();
        let skipped = SKIP_PREFIXES.iter().any(|prefix| base.starts_with(prefix));
        if base.starts_with("pronunciation") || base.starts_with("prononciation") || base.starts_with("aussprache") {
            if phonetic.is_empty() {
                *phonetic = find_ipa(&child.body);
            }
        } else if !skipped && child.level >= 3 && (loose || is_headword_line(&child.body, word)) {
            let defs = if loose { extract_loose(&child.body) } else { extract_defs(&child.body) };
            if !defs.is_empty() && child.title.chars().count() < 30 {
                groups.push(Group { pos: base, defs });
            }
            // Deeper headings (synonyms, derived terms...) are not senses.
            continue;
        }
        // Etymology and pronunciation headings may wrap the real sections.
        visit(child, word, loose, groups, phonetic);
    }
}

/// A part-of-speech section opens with a line naming the word itself.
fn is_headword_line(body: &str, word: &str) -> bool {
    let Some(first) = body.lines().map(str::trim).find(|l| !l.is_empty()) else { return false };
    let head: String = first.chars().take(80).collect::<String>().to_lowercase();
    !head.starts_with('(') && head.contains(&word.to_lowercase())
}

// ------------------------------------------------------------ Suggestions

const WORDLIST: &str = include_str!("../data/wordlist.txt");

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let up = row[j + 1];
            row[j + 1] = (diagonal + usize::from(ca != cb)).min(up + 1).min(row[j] + 1);
            diagonal = up;
        }
    }
    row[b.len()]
}

/// Up to three close English words from the bundled common-word list.
pub fn suggest(word: &str) -> Vec<String> {
    let query: Vec<char> = word.to_lowercase().chars().filter(|c| c.is_ascii_lowercase()).collect();
    if query.len() < 2 {
        return Vec::new();
    }
    let mut scored: Vec<(f32, usize, usize, &str)> = WORDLIST
        .lines()
        .filter_map(|candidate| {
            let chars: Vec<char> = candidate.chars().collect();
            if chars[0] != query[0] || chars.len().abs_diff(query.len()) > 3 || candidate == word {
                return None;
            }
            let distance = levenshtein(&query, &chars);
            let score = distance as f32 / query.len().max(chars.len()) as f32;
            (distance <= 3 && score <= 0.40).then_some((score, distance, chars.len().abs_diff(query.len()), candidate))
        })
        .collect();
    scored.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)).then(a.3.cmp(b.3)));
    scored.into_iter().take(3).map(|s| s.3.to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webster_finds_words_offline() {
        let entry = webster("Serendipity").or_else(|| webster("dictionary")).expect("a common word");
        assert!(!entry.groups.is_empty());
        let x = webster("xanthic").unwrap();
        assert_eq!(x.groups[0].pos, "adjective");
        assert!(webster("zzzzqqqq").is_none());
        assert!(webster("  XANTHATE ").is_some());
    }

    #[test]
    fn suggestions_for_a_typo() {
        assert!(suggest("recieve").contains(&"receive".to_owned()));
        assert!(suggest("q").is_empty());
    }

    #[test]
    fn rest_response_is_parsed_and_stripped() {
        let body = r#"{"en":[{"partOfSpeech":"Noun","language":"English","definitions":[{"definition":"A <a href=\"/x\">fine</a> &amp; rare thing.","examples":["<i>An example.</i>"]}]}]}"#;
        let entry = parse_rest("thing", body).unwrap();
        assert_eq!(entry.groups[0].pos, "noun");
        assert_eq!(entry.groups[0].defs[0].text, "A fine & rare thing.");
        assert_eq!(entry.groups[0].defs[0].example.as_deref(), Some("An example."));
    }

    #[test]
    fn pronunciation_uses_only_the_requested_language_section() {
        let body = r#"{"query":{"pages":{"1":{"extract":"==French==\n===Pronunciation===\nIPA: /fʁɑ̃s/\n==English==\n===Pronunciation===\nIPA: /ɪŋɡlɪʃ/"}}}}"#;
        assert_eq!(parse_pronunciation(body, "en").unwrap().as_deref(), Some("/ɪŋɡlɪʃ/"));
        assert_eq!(parse_pronunciation(body, "fr").unwrap().as_deref(), Some("/fʁɑ̃s/"));
        let no_english = r#"{"query":{"pages":{"1":{"extract":"==French==\n===Pronunciation===\nIPA: /fʁɑ̃s/"}}}}"#;
        assert_eq!(parse_pronunciation(no_english, "en").unwrap(), None);
    }

    #[test]
    #[ignore = "needs the network"]
    fn live_editions() {
        for (lang, word) in [("ru", "дом"), ("it", "casa"), ("pt", "gato"), ("nl", "huis"), ("pl", "dom")] {
            std::thread::sleep(std::time::Duration::from_secs(3));
            let r = wiktionary(word, lang);
            println!("== {lang} {word}: {r:?}");
        }
    }

    #[test]
    fn extract_response_is_sectioned() {
        let text = "== French ==\n\n=== Étymologie ===\nDu latin.\n\n=== Nom commun 1 ===\n\nchat \\ʃa\\ masculin\n\nMammifère carnivore félin.\nCe chat dort. — (Auteur, 1859)\n";
        let body = serde_json::json!({"query": {"pages": {"1": {"extract": text}}}}).to_string();
        let entry = parse_extract("chat", "fr", &body).unwrap().unwrap();
        assert_eq!(entry.groups[0].defs.len(), 1);
        assert!(entry.groups[0].defs[0].text.starts_with("Mammifère"));
        let missing = r#"{"query":{"pages":{"-1":{"missing":""}}}}"#;
        assert_eq!(parse_extract("x", "fr", missing).unwrap(), None);
    }
}
