//! Command line: `dictionary theme ...` edits `theme.json` and exits without
//! ever starting the window; anything else is a lookup or a launch.

use std::path::PathBuf;

use crate::color::Rgba;
use crate::theme::{self, Follow, PRESETS, Palette, Store, TOKENS, Theme};

pub const USAGE: &str = "\
Dictionary: a portable, themeable dictionary

usage: dictionary [--config-dir DIR] [OPTIONS] [WORD...]
       dictionary [--config-dir DIR] theme COMMAND

options:
  -l, --lang CODE     edition to open (en, fr, de, ...; default en)
      --selection     look up the primary selection (needs wl-paste or xclip)
      --panel NAME    open with the help or language panel showing
  -V, --version       print the version
  -h, --help          show this help

theme commands (they edit theme.json atomically; an open window reloads):
  theme show [--json]            print the saved palette
  theme set TOKEN=COLOR ...      change colors
  theme reset                    restore the default palette
  theme preset NAME              midnight, paper, forest, rose or mono
  theme import PATH              copy a palette once (directory or file)
  theme follow desktop DIR       keep following a palette directory or the
                                 symlink to one, and track it live
  theme follow file PATH         keep following a palette file
  theme follow off               stop following
  theme sync                     re-read the followed source now
  add --dry-run to any change to preview it without writing

tokens: background surface surface_alt text muted accent accent_text
        border danger warning success selection
colors: #rrggbb, #aarrggbb (Qt order: alpha first, so #e00e1e36 is #0e1e36
        at alpha 0xe0), or Qt.rgba(14 / 255, 30 / 255, 54 / 255, 0.88)
";

/// What the command line asks the program to do.
pub enum Action {
    /// Print this and exit with the status.
    Done { code: i32 },
    /// Open the window.
    Launch {
        store: Store,
        word: Option<String>,
        lang: Option<String>,
        selection: bool,
        panel: Option<String>,
    },
}

pub fn parse(args: Vec<String>) -> Action {
    let mut args = args.into_iter().peekable();
    let mut dir: Option<PathBuf> = None;
    let mut lang = None;
    let mut selection = false;
    let mut panel = None;
    let mut words = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Action::Done { code: 0 };
            }
            "-V" | "--version" => {
                println!("dictionary {}", env!("CARGO_PKG_VERSION"));
                return Action::Done { code: 0 };
            }
            "--config-dir" => match args.next() {
                Some(value) if value.starts_with('/') => dir = Some(value.into()),
                _ => return fail("--config-dir needs an absolute path"),
            },
            "-l" | "--lang" => match args.next() {
                Some(value) => lang = Some(value),
                None => return fail("--lang needs a language code"),
            },
            "--selection" => selection = true,
            "--panel" => match args.next().as_deref() {
                Some(v @ ("help" | "language")) => panel = Some(v.to_owned()),
                _ => return fail("--panel takes help or language"),
            },
            "theme" if words.is_empty() => {
                let Some(dir) = dir.or_else(Store::default_dir) else {
                    return fail("cannot find a configuration directory");
                };
                let rest: Vec<String> = args.collect();
                return match theme_command(&Store::new(dir), &rest) {
                    Ok(()) => Action::Done { code: 0 },
                    Err(message) => fail(&message),
                };
            }
            other if other.starts_with('-') && other.len() > 1 => {
                return fail(&format!("unknown option {other}"));
            }
            _ => words.push(arg),
        }
    }
    let Some(dir) = dir.or_else(Store::default_dir) else {
        return fail("cannot find a configuration directory");
    };
    Action::Launch {
        store: Store::new(dir),
        word: (!words.is_empty()).then(|| words.join(" ")),
        lang,
        selection,
        panel,
    }
}

fn fail(message: &str) -> Action {
    eprintln!("dictionary: {message}");
    Action::Done { code: 2 }
}

fn theme_command(store: &Store, args: &[String]) -> Result<(), String> {
    let dry = args.iter().any(|a| a == "--dry-run");
    let args: Vec<&str> = args.iter().map(String::as_str).filter(|a| *a != "--dry-run").collect();
    let mut theme = store.load()?;
    let commit = |theme: &Theme| -> Result<(), String> {
        if dry {
            print!("{}", render(theme));
            println!("(dry run: nothing written)");
            Ok(())
        } else {
            store.save(theme)
        }
    };
    match args.as_slice() {
        ["show"] => print!("{}", render(&theme)),
        ["show", "--json"] => {
            println!("{}", std::fs::read_to_string(store.theme_path()).unwrap_or_default().trim_end());
        }
        ["set", pairs @ ..] if !pairs.is_empty() => {
            let mut changes = Vec::new();
            for pair in pairs {
                let (token, value) = pair.split_once('=').ok_or_else(|| format!("expected TOKEN=COLOR, got {pair}"))?;
                let color = Rgba::parse(value).ok_or_else(|| format!("invalid color {value}"))?;
                if !TOKENS.contains(&token.trim()) {
                    return Err(format!("unknown token {token}; tokens: {}", TOKENS.join(" ")));
                }
                changes.push((token.trim(), color));
            }
            // A new background, text or accent re-derives the colors built
            // from them, so one `set` gives a coherent palette; anything
            // named in the same command then wins.
            let base_changed = changes.iter().any(|(t, _)| ["background", "text", "accent"].contains(t));
            if base_changed {
                let get = |t: &str| changes.iter().find(|c| c.0 == t).map_or_else(|| theme.palette.get(t), |c| Some(c.1));
                theme.palette = Palette::derive(&theme::Base {
                    background: get("background").ok_or("no background")?,
                    text: get("text").ok_or("no text")?,
                    accent: get("accent").ok_or("no accent")?,
                    surface: None,
                    muted: None,
                    danger: theme.palette.get("danger"),
                    warning: theme.palette.get("warning"),
                    success: theme.palette.get("success"),
                });
            }
            for (token, color) in changes {
                theme.palette.set(token, color);
            }
            theme.follow = None;
            theme.source_name = None;
            commit(&theme)?;
        }
        ["reset"] => commit(&Theme::default())?,
        ["preset", name] => {
            let palette = theme::preset(name)
                .ok_or_else(|| format!("unknown preset {name}; presets: {}", PRESETS.join(" ")))?;
            commit(&Theme { palette, follow: None, source_name: Some((*name).to_owned()) })?;
        }
        ["import", path] => {
            let (palette, name) = theme::read_source(&absolute(path)?)?;
            commit(&Theme { palette, follow: None, source_name: Some(name) })?;
        }
        ["follow", "off"] => {
            theme.follow = None;
            commit(&theme)?;
        }
        ["follow", mode @ ("desktop" | "file"), path] => {
            let path = absolute(path)?;
            let (palette, name) = theme::read_source(&path)?;
            commit(&Theme {
                palette,
                follow: Some(Follow { mode: (*mode).to_owned(), path }),
                source_name: Some(name),
            })?;
        }
        ["sync"] => {
            if theme.follow.is_none() {
                return Err("not following a source; use `theme follow`".into());
            }
            if store.refresh(&mut theme)? || dry {
                commit(&theme)?;
            }
        }
        _ => return Err("unknown theme command; see --help".to_owned()),
    }
    Ok(())
}

fn absolute(path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        Ok(path)
    } else {
        std::env::current_dir().map(|cwd| cwd.join(path)).map_err(|e| e.to_string())
    }
}

fn render(theme: &Theme) -> String {
    let mut out = String::new();
    for token in TOKENS {
        if let Some(color) = theme.palette.get(token) {
            out.push_str(&format!("{token:<12} {color}\n"));
        }
    }
    match &theme.follow {
        Some(f) => out.push_str(&format!("following    {} {}\n", f.mode, f.path.display())),
        None => out.push_str("following    nothing\n"),
    }
    out
}
