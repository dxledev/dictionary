use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const HELPER: &str = include_str!("../scripts/urban_pos.py");
const MAX_ROWS: usize = 5;
const MAX_WORD_CHARS: usize = 100;
const MAX_EXAMPLE_CHARS: usize = 4000;
const MAX_OUTPUT_BYTES: usize = 2048;
const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Serialize)]
pub struct Request {
    pub word: String,
    pub example: String,
}

#[derive(Deserialize)]
struct Response(Vec<Option<String>>);

pub fn classify(requests: &[Request]) -> Vec<Option<&'static str>> {
    if std::env::var("DICTIONARY_NLTK").as_deref() == Ok("0")
        || requests.is_empty()
    {
        return vec![None; requests.len()];
    }
    let Some(rows) = bounded_requests(requests) else {
        return vec![None; requests.len()];
    };
    let (interpreter, managed) = interpreter();
    let mut command = Command::new(interpreter);
    command
        .arg("-c")
        .arg(HELPER)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if managed && std::env::var_os("NLTK_DATA").is_none() {
        if let Some(path) = managed_nltk_data() {
            command.env("NLTK_DATA", path);
        }
    }
    if std::env::var_os("DICTIONARY_NLTK_MIN_CONFIDENCE").is_none() {
        command.env("DICTIONARY_NLTK_MIN_CONFIDENCE", "0.9");
    }
    classify_with(&mut command, &rows)
}

fn bounded_requests(requests: &[Request]) -> Option<Vec<Request>> {
    if requests.len() > MAX_ROWS
        || requests.iter().any(|row| {
            row.word.chars().count() > MAX_WORD_CHARS
                || row.example.chars().count() > MAX_EXAMPLE_CHARS
        })
    {
        return None;
    }
    Some(
        requests
            .iter()
            .map(|row| Request {
                word: row.word.clone(),
                example: row.example.clone(),
            })
            .collect(),
    )
}

fn interpreter() -> (String, bool) {
    if let Some(path) = std::env::var_os("DICTIONARY_NLTK_PYTHON") {
        return (path.to_string_lossy().into_owned(), false);
    }
    if let Some(path) = managed_python() {
        return (path.to_string_lossy().into_owned(), true);
    }
    ("python3".into(), false)
}

fn managed_python() -> Option<std::path::PathBuf> {
    let mut path = if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
        std::path::PathBuf::from(data)
    } else {
        std::path::PathBuf::from(std::env::var_os("HOME")?).join(".local/share")
    };
    path.push("dictionary/nltk/bin/python");
    path.is_file().then_some(path)
}

fn managed_nltk_data() -> Option<std::path::PathBuf> {
    managed_python()?
        .parent()?
        .parent()
        .map(|path| path.join("nltk_data"))
}

fn classify_with(
    command: &mut Command,
    requests: &[Request],
) -> Vec<Option<&'static str>> {
    classify_with_timeout(command, requests, TIMEOUT)
}

fn classify_with_timeout(
    command: &mut Command,
    requests: &[Request],
    timeout: Duration,
) -> Vec<Option<&'static str>> {
    let failed = || vec![None; requests.len()];
    let Some(rows) = bounded_requests(requests) else {
        return failed();
    };
    let Ok(input) = serde_json::to_vec(&rows) else {
        return failed();
    };
    let Ok(mut input_file) = tempfile::tempfile() else {
        return failed();
    };
    let Ok(mut output_file) = tempfile::tempfile() else {
        return failed();
    };
    if input_file.write_all(&input).is_err()
        || input_file.seek(SeekFrom::Start(0)).is_err()
    {
        return failed();
    }
    let Ok(child_input) = input_file.try_clone() else {
        return failed();
    };
    let Ok(child_output) = output_file.try_clone() else {
        return failed();
    };
    command
        .stdin(Stdio::from(child_input))
        .stdout(Stdio::from(child_output));
    let Ok(mut child) = command.spawn() else {
        return failed();
    };
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if start.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(20))
            }
            _ => break None,
        }
        if start.elapsed() >= timeout {
            break None;
        }
    };
    let Some(status) = status else {
        let _ = child.kill();
        let _ = child.wait();
        return failed();
    };
    if !status.success() {
        return failed();
    }
    if output_file.seek(SeekFrom::End(0)).is_err() {
        return failed();
    }
    let Ok(output_len) = output_file.stream_position() else {
        return failed();
    };
    if output_len > MAX_OUTPUT_BYTES as u64
        || output_file.seek(SeekFrom::Start(0)).is_err()
    {
        return failed();
    }
    let mut bytes = Vec::with_capacity(output_len as usize);
    if output_file
        .take(MAX_OUTPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return failed();
    }
    let Ok(Response(values)) = serde_json::from_slice::<Response>(&bytes)
    else {
        return failed();
    };
    if values.len() != requests.len() {
        return failed();
    }
    values
        .into_iter()
        .map(|value| value.as_deref().and_then(label))
        .collect()
}

fn label(value: &str) -> Option<&'static str> {
    match value {
        "noun" => Some("noun"),
        "verb" => Some("verb"),
        "adjective" => Some("adjective"),
        "adverb" => Some("adverb"),
        "interjection" => Some("interjection"),
        "pronoun" => Some("pronoun"),
        "preposition" => Some("preposition"),
        "conjunction" => Some("conjunction"),
        "determiner" => Some("determiner"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fake(script: &str) -> (tempfile::TempDir, Command) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fake-python");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .unwrap();
        (dir, Command::new(path))
    }

    fn rows() -> Vec<Request> {
        vec![Request {
            word: "slangword".into(),
            example: "That slangword works.".into(),
        }]
    }

    #[test]
    fn accepts_valid_category_and_rejects_unknown_or_wrong_length() {
        let (_dir, mut command) = fake("cat >/dev/null; printf '[\"noun\"]'");
        assert_eq!(classify_with(&mut command, &rows()), vec![Some("noun")]);
        let (_dir, mut command) = fake("cat >/dev/null; printf '[\"bogus\"]'");
        assert_eq!(classify_with(&mut command, &rows()), vec![None]);
        let (_dir, mut command) = fake("cat >/dev/null; printf '[]'");
        assert_eq!(classify_with(&mut command, &rows()), vec![None]);
    }

    #[test]
    fn fails_closed_for_malformed_output_nonzero_and_timeout() {
        let (_dir, mut command) = fake("cat >/dev/null; printf 'nope'");
        assert_eq!(classify_with(&mut command, &rows()), vec![None]);
        let (_dir, mut command) = fake("cat >/dev/null; exit 1");
        assert_eq!(classify_with(&mut command, &rows()), vec![None]);
        let (_dir, mut command) = fake("exec sleep 5");
        assert_eq!(
            classify_with_timeout(
                &mut command,
                &rows(),
                Duration::from_millis(100)
            ),
            vec![None]
        );
    }

    #[test]
    fn batch_keeps_each_result_in_request_order() {
        let requests = vec![
            Request {
                word: "first".into(),
                example: "The first works.".into(),
            },
            Request {
                word: "second".into(),
                example: "We second it.".into(),
            },
        ];
        let (_dir, mut command) =
            fake("cat >/dev/null; printf '[null,\"verb\"]'");
        assert_eq!(
            classify_with(&mut command, &requests),
            vec![None, Some("verb")]
        );
    }

    #[test]
    fn large_input_cannot_block_timeout_when_helper_does_not_read() {
        let requests: Vec<_> = (0..MAX_ROWS)
            .map(|_| Request {
                word: "rizz".into(),
                example: "x".repeat(MAX_EXAMPLE_CHARS),
            })
            .collect();
        let (_dir, mut command) = fake("exec sleep 5");
        let start = Instant::now();
        assert_eq!(
            classify_with_timeout(
                &mut command,
                &requests,
                Duration::from_millis(100)
            ),
            vec![None; MAX_ROWS]
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    #[ignore = "requires the optional installed NLTK environment and model"]
    fn installed_model_classifies_headword_in_context() {
        let requests = vec![Request {
            word: "rizz".into(),
            example: "He has incredible rizz.".into(),
        }];
        assert_eq!(classify(&requests), vec![Some("noun")]);
    }

    #[test]
    fn rejects_oversized_requests_before_spawning() {
        let mut request = rows();
        request[0].example = "x".repeat(MAX_EXAMPLE_CHARS + 1);
        let (_dir, mut command) = fake("exit 1");
        assert_eq!(classify_with(&mut command, &request), vec![None]);
        assert!(bounded_requests(&request).is_none());
    }
}
