//! Urban Dictionary lookups: fetch the public define API and reduce each entry
//! to the fields the dictionary UI shows.

use crate::model::{curl, encode};
use serde::Deserialize;

const API: &str = "https://api.urbandictionary.com/v0/define?term=";

/// At most this many definitions are kept.
pub const MAX_DEFS: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrbanDef {
    pub word: String,
    pub text: String,
    pub example: Option<String>,
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
    parse(&body)
}

pub fn parse(body: &str) -> Result<Vec<UrbanDef>, String> {
    let resp: Response = serde_json::from_str(body).map_err(|e| e.to_string())?;
    Ok(resp
        .list
        .into_iter()
        .filter_map(|e| {
            let text = clean(&e.definition.unwrap_or_default());
            if text.is_empty() {
                return None;
            }
            let example = clean(&e.example.unwrap_or_default());
            Some(UrbanDef {
                word: e.word.unwrap_or_default(),
                text,
                example: (!example.is_empty()).then_some(example),
                author: e.author.unwrap_or_default(),
                date: e.written_on.unwrap_or_default().get(..10).unwrap_or("").to_string(),
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
    fn short_date_is_empty() {
        let defs = parse(r#"{"list":[{"definition":"x","written_on":"2020"}]}"#).unwrap();
        assert_eq!(defs[0].date, "");
    }

    #[test]
    #[ignore]
    fn network_lookup_returns_results() {
        let defs = urban("rizz").unwrap();
        assert!(!defs.is_empty());
    }
}
