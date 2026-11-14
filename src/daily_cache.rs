use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::daily::DailyWord;

const VERSION: u32 = 1;
const MAX_CACHE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Wiktionary,
    Urban,
}

impl Source {
    fn filename(self) -> &'static str {
        match self {
            Self::Wiktionary => "wiktionary.json",
            Self::Urban => "urban.json",
        }
    }
}

#[derive(Debug)]
pub struct CachedWords {
    pub words: Vec<DailyWord>,
    pub fetched_at: Instant,
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    fetched_at_unix: u64,
    words: Vec<DailyWord>,
}

fn cache_dir() -> Option<PathBuf> {
    let xdg = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).filter(|path| path.is_absolute());
    let base = xdg.or_else(|| {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .map(|home| home.join(".cache"))
    })?;
    Some(base.join("dictionary").join("daily"))
}

fn file_path(dir: &Path, source: Source) -> PathBuf {
    dir.join(source.filename())
}

fn valid_words(words: &[DailyWord]) -> bool {
    words.len() == 2
        && words.iter().all(|word| !word.word.trim().is_empty() && !word.text.trim().is_empty())
        && !words[0].word.eq_ignore_ascii_case(&words[1].word)
}

fn instant_from_unix(timestamp: u64) -> Option<Instant> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    let age = elapsed.checked_sub(timestamp)?;
    Instant::now().checked_sub(Duration::from_secs(age))
}

fn load_from(dir: &Path, source: Source) -> Option<CachedWords> {
    let path = file_path(dir, source);
    if fs::metadata(&path).ok()?.len() > MAX_CACHE_BYTES {
        return None;
    }
    let file = fs::File::open(path).ok()?;
    let mut contents = Vec::new();
    file.take(MAX_CACHE_BYTES + 1).read_to_end(&mut contents).ok()?;
    if contents.len() as u64 > MAX_CACHE_BYTES {
        return None;
    }
    let cache: CacheFile = serde_json::from_slice(&contents).ok()?;
    if cache.version != VERSION || !valid_words(&cache.words) {
        return None;
    }
    Some(CachedWords {
        words: cache.words,
        fetched_at: instant_from_unix(cache.fetched_at_unix)?,
    })
}

pub fn load(source: Source) -> Option<CachedWords> {
    load_from(&cache_dir()?, source)
}

pub fn save(source: Source, words: &[DailyWord]) {
    let Some(dir) = cache_dir() else { return };
    save_source_to(&dir, source, words);
}

fn save_source_to(dir: &Path, source: Source, words: &[DailyWord]) {
    if !valid_words(words) {
        return;
    }
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    let Ok(fetched_at_unix) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return;
    };
    let cache = CacheFile {
        version: VERSION,
        fetched_at_unix: fetched_at_unix.as_secs(),
        words: words.to_vec(),
    };
    let Ok(contents) = serde_json::to_vec(&cache) else { return };
    if contents.len() as u64 > MAX_CACHE_BYTES {
        return;
    }
    let Ok(mut file) = tempfile::NamedTempFile::new_in(dir) else { return };
    if file.write_all(&contents).is_err() || file.flush().is_err() {
        return;
    }
    let _ = file.persist(file_path(dir, source));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words() -> Vec<DailyWord> {
        ["first", "second"]
            .into_iter()
            .map(|word| DailyWord {
                word: word.into(),
                text: format!("{word} definition"),
                pos: None,
                phonetic: String::new(),
                example: None,
                author: None,
                date: None,
            })
            .collect()
    }

    #[test]
    fn round_trips_separate_sources() {
        let dir = tempfile::tempdir().unwrap();
        let entries = words();
        save_source_to(dir.path(), Source::Wiktionary, &entries);
        let mut urban = entries.clone();
        urban[0].word = "urban".into();
        save_source_to(dir.path(), Source::Urban, &urban);

        assert_eq!(load_from(dir.path(), Source::Wiktionary).unwrap().words, entries);
        assert_eq!(load_from(dir.path(), Source::Urban).unwrap().words, urban);
    }

    #[test]
    fn rejects_invalid_shape_and_future_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let invalid = CacheFile {
            version: VERSION,
            fetched_at_unix: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
            words: vec![words().remove(0)],
        };
        fs::write(file_path(dir.path(), Source::Wiktionary), serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(load_from(dir.path(), Source::Wiktionary).is_none());

        let future = CacheFile { words: words(), fetched_at_unix: u64::MAX, version: VERSION };
        fs::write(file_path(dir.path(), Source::Urban), serde_json::to_vec(&future).unwrap()).unwrap();
        assert!(load_from(dir.path(), Source::Urban).is_none());
    }

    #[test]
    fn preserves_stale_age_and_rejects_missing_malformed_or_unknown_versions() {
        let dir = tempfile::tempdir().unwrap();
        let stale_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_sub(7200);
        let stale = CacheFile { version: VERSION, fetched_at_unix: stale_timestamp, words: words() };
        fs::write(file_path(dir.path(), Source::Wiktionary), serde_json::to_vec(&stale).unwrap()).unwrap();
        assert!(load_from(dir.path(), Source::Wiktionary).unwrap().fetched_at.elapsed() >= Duration::from_secs(7199));

        assert!(load_from(dir.path(), Source::Urban).is_none());
        fs::write(file_path(dir.path(), Source::Urban), b"not json").unwrap();
        assert!(load_from(dir.path(), Source::Urban).is_none());

        let unknown = CacheFile { version: VERSION + 1, fetched_at_unix: stale_timestamp, words: words() };
        fs::write(file_path(dir.path(), Source::Urban), serde_json::to_vec(&unknown).unwrap()).unwrap();
        assert!(load_from(dir.path(), Source::Urban).is_none());
    }

    #[test]
    fn refuses_oversized_cache() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(file_path(dir.path(), Source::Urban), vec![b'x'; MAX_CACHE_BYTES as usize + 1]).unwrap();
        assert!(load_from(dir.path(), Source::Urban).is_none());
    }
}
