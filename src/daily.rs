use crate::model::{curl, strip_html};

const WIKTIONARY_API: &str = "https://en.wiktionary.org/w/api.php?action=parse&page=Wiktionary%3AWord_of_the_day&prop=text&format=json";
const URBAN_API: &str = "https://api.urbandictionary.com/v0/words_of_the_day";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DailyWord {
    pub word: String,
    pub text: String,
    pub pos: Option<String>,
    pub phonetic: String,
    pub example: Option<String>,
    pub author: Option<String>,
    pub date: Option<String>,
}

pub fn wiktionary() -> Result<Vec<DailyWord>, String> {
    let body = curl(WIKTIONARY_API)?;
    let html = response_html(&body)?;
    let mut current = parse_wiktionary_html(&html)?;
    enrich_pronunciation(&mut current);
    let previous_page = previous_archive_page(&archive_page(&html)?)?;
    let url = format!(
        "https://en.wiktionary.org/w/api.php?action=parse&page={}&prop=text&format=json",
        encode_component(&previous_page)
    );
    let previous_html = response_html(&curl(&url)?)?;
    let mut previous = parse_wiktionary_html(&previous_html)?;
    enrich_pronunciation(&mut previous);
    if current.word.eq_ignore_ascii_case(&previous.word) {
        return Err("Wiktionary current and previous featured words match".into());
    }
    Ok(vec![current, previous])
}

fn enrich_pronunciation(word: &mut DailyWord) {
    word.phonetic = crate::model::pronunciation(&word.word, "en")
        .ok().flatten().unwrap_or_default();
}

pub fn urban() -> Result<Vec<DailyWord>, String> {
    parse_urban(&curl(URBAN_API)?)
}

fn response_html(body: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    value.get("parse").and_then(|v| v.get("text")).and_then(|v| v.get("*"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "Wiktionary response has no expanded word".into())
}

fn parse_wiktionary_html(html: &str) -> Result<DailyWord, String> {
    let word = element(html, "WOTD-rss-title").ok_or("Wiktionary response has no word")?;
    let text = first_wiktionary_definition(html).ok_or("Wiktionary response has no definition")?;
    if word.is_empty() || text.is_empty() {
        return Err("Wiktionary response has an empty word or definition".into());
    }
    let date = element(html, "WOTD-rss-date").filter(|s| !s.is_empty());
    let pos = part_of_speech(html).map(|pos| normalize_part_of_speech(&pos));
    Ok(DailyWord { word, text, pos, phonetic: String::new(), example: None, author: None, date })
}

fn parse_urban(body: &str) -> Result<Vec<DailyWord>, String> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let entries = value.get("list").and_then(serde_json::Value::as_array)
        .ok_or("Urban Dictionary response has no entry list")?;
    let mut words = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for entry in entries {
        let field = |name: &str| entry.get(name).and_then(serde_json::Value::as_str).unwrap_or("").trim();
        let word = field("word");
        let text = first_urban_definition(&crate::urban::clean(field("definition")));
        if word.is_empty() || text.trim().is_empty() || !seen.insert(word.to_lowercase()) {
            continue;
        }
        let optional = |name: &str| {
            let value = field(name);
            (!value.is_empty()).then(|| value.to_owned())
        };
        words.push(DailyWord {
            word: word.to_owned(),
            pos: crate::urban::infer_part_of_speech(&text).map(str::to_owned),
            phonetic: String::new(),
            text,
            example: optional("example").map(|s| crate::urban::clean(&s)),
            author: optional("author"),
            date: optional("date"),
        });
        if words.len() == 2 { break; }
    }
    if words.len() < 2 {
        return Err("Urban Dictionary daily list has fewer than two distinct valid entries".into());
    }
    Ok(words)
}

fn normalize_part_of_speech(pos: &str) -> String {
    let normalized = pos.trim().trim_end_matches('.').to_ascii_lowercase();
    match normalized.as_str() {
        "adj" | "a" => "adjective".into(),
        "n" => "noun".into(),
        "v" | "vb" => "verb".into(),
        "adv" => "adverb".into(),
        "prep" => "preposition".into(),
        "pron" => "pronoun".into(),
        "conj" => "conjunction".into(),
        "interj" => "interjection".into(),
        "det" => "determiner".into(),
        "num" => "numeral".into(),
        _ => pos.trim().to_owned(),
    }
}

fn archive_page(html: &str) -> Result<String, String> {
    let marker = "Wiktionary:Word_of_the_day/";
    let start = html.find(marker).ok_or("Wiktionary response has no dated archive link")?;
    let end = html[start..].find(['&', '"', '<']).map(|i| start + i)
        .ok_or("Wiktionary archive link is incomplete")?;
    Ok(html[start..end].to_owned())
}

fn previous_archive_page(page: &str) -> Result<String, String> {
    let mut parts = page.rsplitn(3, '/');
    let month_day = parts.next().ok_or("invalid archive date")?;
    let year: i32 = parts.next().ok_or("invalid archive year")?.parse().map_err(|_| "invalid archive year")?;
    let prefix = parts.next().ok_or("invalid archive page")?;
    let months = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    let (month, day) = month_day.split_once('_').ok_or("invalid archive month and day")?;
    let day: u32 = day.parse().map_err(|_| "invalid archive day")?;
    let month_number = months.iter().position(|candidate| *candidate == month).ok_or("invalid archive month")?;
    Ok(if day > 1 {
        format!("{prefix}/{year}/{month}_{}", day - 1)
    } else if month_number > 0 {
        format!("{prefix}/{year}/{}_{}", months[month_number - 1], days_in_month(year, month_number - 1))
    } else {
        format!("{prefix}/{}/December_31", year - 1)
    })
}

fn days_in_month(year: i32, month: usize) -> u32 {
    match month {
        1 => if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) { 29 } else { 28 },
        3 | 5 | 8 | 10 => 30,
        _ => 31,
    }
}

fn encode_component(value: &str) -> String {
    value.bytes().map(|byte| match byte {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (byte as char).to_string(),
        _ => format!("%{byte:02X}"),
    }).collect()
}

fn part_of_speech(html: &str) -> Option<String> {
    let title = html.find("WOTD-rss-title")?;
    let after_title = &html[title..];
    let span_end = after_title.find("</span>")? + "</span>".len();
    let after_link = after_title[span_end..].find("</a>").map(|i| span_end + i + 4)?;
    let tail = &after_title[after_link..];
    let italic_start = tail.find("<i>")? + 3;
    let italic_end = tail[italic_start..].find("</i>")? + italic_start;
    let pos = strip_html(&tail[italic_start..italic_end]);
    (!pos.is_empty()).then_some(pos)
}

fn first_wiktionary_definition(html: &str) -> Option<String> {
    let marker = "id=\"WOTD-rss-description\"";
    let start = html.find(marker)?;
    let list_start = html[start..].find("<li")? + start;
    let content_start = html[list_start..].find('>')? + list_start + 1;
    let end = html[content_start..].find("</li>")? + content_start;
    Some(strip_html(&html[content_start..end]))
}

fn first_urban_definition(text: &str) -> String {
    let text = text.trim();
    let first = text.lines().next().unwrap_or("").trim();
    let Some((1, content)) = numbered_sense(first) else {
        return text.to_owned();
    };
    let mut definition = content.to_owned();
    for line in text.lines().skip(1) {
        if numbered_sense(line.trim()).is_some_and(|(number, _)| number > 1) {
            break;
        }
        definition.push('\n');
        definition.push_str(line);
    }
    definition.trim().to_owned()
}

fn numbered_sense(line: &str) -> Option<(usize, &str)> {
    let end = line.find(|c: char| !c.is_ascii_digit())?;
    let number = line[..end].parse().ok()?;
    let content = line[end..].strip_prefix(['.', ')'])?;
    if !content.starts_with(char::is_whitespace) {
        return None;
    }
    Some((number, content.trim_start()))
}

fn element(html: &str, id: &str) -> Option<String> {
    let marker = format!("id=\"{id}\"");
    let marker_start = html.find(&marker)?;
    let opening_start = html[..marker_start].rfind('<')?;
    let tag_end = html[opening_start..].find(char::is_whitespace)
        .or_else(|| html[opening_start..].find('>'))? + opening_start;
    let tag = html[opening_start + 1..tag_end].trim_start_matches('/');
    let content_start = html[marker_start..].find('>')? + marker_start + 1;
    let open = format!("<{tag}");
    let closing = format!("</{tag}>");
    let mut depth = 1usize;
    let mut cursor = content_start;
    let content_end = loop {
        let next_open = html[cursor..].find(&open).map(|i| cursor + i);
        let next_close = html[cursor..].find(&closing).map(|i| cursor + i);
        match (next_open, next_close) {
            (_, Some(close)) if next_open.is_none_or(|open| close < open) => {
                depth -= 1;
                if depth == 0 { break close; }
                cursor = close + closing.len();
            }
            (Some(opening), _) => {
                cursor = html[opening..].find('>')? + opening + 1;
                depth += 1;
            }
            _ => return None,
        }
    };
    Some(strip_html(&html[content_start..content_end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_current_word_and_archive_date() {
        let body = r#"{"parse":{"text":{"*":"<a href=\"/wiki/Wiktionary:Word_of_the_day/2026/November_13\">view</a><span id=\"WOTD-rss-title\">Carolean</span></a> <i>adj</i><div id=\"WOTD-rss-description\"><ol><li>A full <a href=\"/wiki/term\">definition</a>.</li></ol></div><span id=\"WOTD-rss-date\">November 13</span>"}}}"#;
        let html = response_html(body).unwrap();
        let word = parse_wiktionary_html(&html).unwrap();
        assert_eq!(word.word, "Carolean");
        assert_eq!(word.text, "A full definition.");
        assert_eq!(word.pos.as_deref(), Some("adjective"));
        assert!(word.phonetic.is_empty());
        assert_eq!(previous_archive_page(&archive_page(&html).unwrap()).unwrap(), "Wiktionary:Word_of_the_day/2026/November_12");
    }

    #[test]
    fn previous_archive_handles_month_and_year_boundaries() {
        assert_eq!(previous_archive_page("Wiktionary:Word_of_the_day/2024/March_1").unwrap(), "Wiktionary:Word_of_the_day/2024/February_29");
        assert_eq!(previous_archive_page("Wiktionary:Word_of_the_day/2026/January_1").unwrap(), "Wiktionary:Word_of_the_day/2025/December_31");
    }

    #[test]
    fn urban_uses_first_two_distinct_valid_entries_in_order_and_keeps_full_text() {
        let long = "x".repeat(700);
        let body = format!(r#"{{"list":[{{"word":"First","definition":"[{}]"}},{{"word":"FIRST","definition":"duplicate"}},{{"word":"","definition":"missing word"}},{{"word":"Second","definition":"[second]","example":"[Example]","author":"Writer","date":"November 11"}},{{"word":"Third","definition":"later"}}]}}"#, long);
        let words = parse_urban(&body).unwrap();
        assert_eq!(words.iter().map(|w| w.word.as_str()).collect::<Vec<_>>(), ["First", "Second"]);
        assert_eq!(words[0].text.len(), 700);
        assert_eq!(words[1].example.as_deref(), Some("Example"));
        assert_eq!(words[1].author.as_deref(), Some("Writer"));
        assert_eq!(words[1].date.as_deref(), Some("November 11"));
    }

    #[test]
    fn selects_only_the_first_featured_definition() {
        assert_eq!(first_urban_definition("1) First sense.\r\n  2) Second sense."), "First sense.");
        assert_eq!(first_urban_definition("A person.\nStill one definition."), "A person.\nStill one definition.");
        let html = "<div id=\"WOTD-rss-description\"><ol><li>First <b>meaning</b></li><li>Second meaning</li></ol></div>";
        assert_eq!(first_wiktionary_definition(html).as_deref(), Some("First meaning"));
        assert_eq!(first_urban_definition("1. first\n2. second"), "first");
    }

    #[test]
    fn expands_common_wiktionary_part_of_speech_abbreviations() {
        for (abbreviation, expanded) in [
            ("adj", "adjective"), ("n.", "noun"), ("v", "verb"),
            ("adv", "adverb"), ("prep", "preposition"),
        ] {
            assert_eq!(normalize_part_of_speech(abbreviation), expanded);
        }
        assert_eq!(normalize_part_of_speech("proper noun"), "proper noun");
    }

    #[test]
    fn rejects_invalid_or_insufficient_entries() {
        assert!(parse_urban("not json").is_err());
        assert!(parse_urban(r#"{"list":[]}"#).is_err());
        assert!(parse_urban(r#"{"list":[{"word":"x"}]}"#).is_err());
        assert!(response_html("not json").is_err());
    }

    #[test]
    #[ignore = "requires live Wiktionary and Urban Dictionary endpoints"]
    fn live_sources_return_two_daily_words() {
        assert_eq!(wiktionary().unwrap().len(), 2);
        assert_eq!(urban().unwrap().len(), 2);
    }
}
