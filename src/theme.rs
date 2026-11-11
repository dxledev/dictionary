//! The saved palette, its on-disk form, and the sources it can follow.
//!
//! `theme.json` is the single source of truth: every CLI command edits it
//! atomically and the window reloads from it. Following a source (a palette
//! directory, a symlink to one, or a file) is nothing more than re-resolving
//! that source and writing the result back, so the file always holds the last
//! colour state, even when no window is open.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::color::Rgba;

/// Every colour the interface reads, in display order.
pub const TOKENS: [&str; 12] = [
    "background",
    "surface",
    "surface_alt",
    "text",
    "muted",
    "accent",
    "accent_text",
    "border",
    "danger",
    "warning",
    "success",
    "selection",
];

/// The palette: one colour per token. Only `background` is meant to carry
/// alpha; the rest are painted opaque.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub background: Rgba,
    pub surface: Rgba,
    pub surface_alt: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub accent: Rgba,
    pub accent_text: Rgba,
    pub border: Rgba,
    pub danger: Rgba,
    pub warning: Rgba,
    pub success: Rgba,
    pub selection: Rgba,
}

impl Default for Palette {
    fn default() -> Self {
        Self::derive(&Base {
            background: Rgba::rgb(0x14, 0x17, 0x1f),
            text: Rgba::rgb(0xe6, 0xe9, 0xf0),
            accent: Rgba::rgb(0x7a, 0xa2, 0xf7),
            surface: None,
            muted: None,
            danger: None,
            warning: None,
            success: None,
        })
    }
}

/// The few colours a source must supply; the rest are derived from them.
pub struct Base {
    pub background: Rgba,
    pub text: Rgba,
    pub accent: Rgba,
    pub surface: Option<Rgba>,
    pub muted: Option<Rgba>,
    pub danger: Option<Rgba>,
    pub warning: Option<Rgba>,
    pub success: Option<Rgba>,
}

impl Palette {
    /// The palette as drawn: muted text is lifted half a shade (about 6% of
    /// the way to white) for every theme. The saved palette is untouched.
    pub fn shown(mut self) -> Self {
        let white = Rgba { r: 255, g: 255, b: 255, a: self.muted.a };
        self.muted = self.muted.mix(white, 0.06);
        self
    }

    /// Fill in every token from a base set, so any source with a background,
    /// a text colour and an accent produces a complete, readable palette.
    pub fn derive(base: &Base) -> Self {
        let bg = base.background.opaque();
        let surface = base.surface.map_or_else(|| bg.mix(base.text, 0.07), Rgba::opaque);
        Self {
            background: base.background,
            surface,
            surface_alt: surface.mix(base.text, 0.07),
            text: base.text.opaque(),
            muted: base.muted.map_or_else(|| bg.mix(base.text, 0.60), Rgba::opaque),
            accent: base.accent.opaque(),
            accent_text: base.accent.readable_on(),
            border: surface.mix(base.text, 0.16),
            danger: base.danger.map_or(Rgba::rgb(0xe0, 0x6c, 0x75), Rgba::opaque),
            warning: base.warning.map_or(Rgba::rgb(0xe5, 0xc0, 0x7b), Rgba::opaque),
            success: base.success.map_or(Rgba::rgb(0x98, 0xc3, 0x79), Rgba::opaque),
            selection: bg.mix(base.accent, 0.28),
        }
    }

    pub fn get(&self, token: &str) -> Option<Rgba> {
        Some(match token {
            "background" => self.background,
            "surface" => self.surface,
            "surface_alt" => self.surface_alt,
            "text" => self.text,
            "muted" => self.muted,
            "accent" => self.accent,
            "accent_text" => self.accent_text,
            "border" => self.border,
            "danger" => self.danger,
            "warning" => self.warning,
            "success" => self.success,
            "selection" => self.selection,
            _ => return None,
        })
    }

    pub fn set(&mut self, token: &str, color: Rgba) -> bool {
        let slot = match token {
            "background" => &mut self.background,
            "surface" => &mut self.surface,
            "surface_alt" => &mut self.surface_alt,
            "text" => &mut self.text,
            "muted" => &mut self.muted,
            "accent" => &mut self.accent,
            "accent_text" => &mut self.accent_text,
            "border" => &mut self.border,
            "danger" => &mut self.danger,
            "warning" => &mut self.warning,
            "success" => &mut self.success,
            "selection" => &mut self.selection,
            _ => return false,
        };
        *slot = color;
        true
    }

    /// Whether the background is light, for choosing the icon contrast.
    pub fn is_light(&self) -> bool {
        self.background.opaque().is_light()
    }

    fn to_map(self) -> BTreeMap<String, String> {
        TOKENS
            .iter()
            .filter_map(|token| Some(((*token).to_owned(), self.get(token)?.to_saved())))
            .collect()
    }

    /// Overlay a saved colour map onto `self`; unknown or unparsable entries
    /// are ignored so one bad value never discards the rest.
    fn overlay(mut self, map: &BTreeMap<String, String>) -> Self {
        for (token, value) in map {
            if let Some(color) = Rgba::parse(value) {
                self.set(token, color);
            }
        }
        self
    }
}

/// What the palette follows, if anything.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Follow {
    /// `desktop` (a palette directory or the symlink to one) or `file`.
    pub mode: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Saved {
    version: u32,
    colors: BTreeMap<String, String>,
    #[serde(default)]
    follow: Option<Follow>,
    /// Name of the source palette last applied, for display only.
    #[serde(default)]
    source_name: Option<String>,
}

/// The palette plus what it follows, as stored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Theme {
    pub palette: Palette,
    pub follow: Option<Follow>,
    pub source_name: Option<String>,
}

/// Where configuration lives.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// `$XDG_CONFIG_HOME/dictionary` or `~/.config/dictionary`.
    pub fn default_dir() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))?;
        Some(base.join("dictionary"))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn theme_path(&self) -> PathBuf {
        self.dir.join("theme.json")
    }

    /// Load the saved theme. A missing file is the default; a damaged one is
    /// an error so the caller can keep what it has on screen.
    pub fn load(&self) -> Result<Theme, String> {
        let text = match fs::read_to_string(self.theme_path()) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Theme::default());
            }
            Err(error) => return Err(format!("cannot read theme.json: {error}")),
        };
        let saved: Saved =
            serde_json::from_str(&text).map_err(|error| format!("invalid theme.json: {error}"))?;
        Ok(Theme {
            palette: Palette::default().overlay(&saved.colors),
            follow: saved.follow,
            source_name: saved.source_name,
        })
    }

    /// Write `theme` atomically: a temporary file in the same directory, then
    /// a rename, so a watcher never sees half a file.
    pub fn save(&self, theme: &Theme) -> Result<(), String> {
        let saved = Saved {
            version: 1,
            colors: theme.palette.to_map(),
            follow: theme.follow.clone(),
            source_name: theme.source_name.clone(),
        };
        let mut text = serde_json::to_string_pretty(&saved).map_err(|e| e.to_string())?;
        text.push('\n');
        fs::create_dir_all(&self.dir).map_err(|e| format!("cannot create {}: {e}", self.dir.display()))?;
        let temporary = self.dir.join(format!(".theme.json.{}.tmp", std::process::id()));
        let result = (|| {
            let mut file = fs::File::create(&temporary)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            fs::rename(&temporary, self.theme_path())
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary);
            return Err(format!("cannot write theme.json: {error}"));
        }
        Ok(())
    }

    /// Re-resolve the followed source into `theme`. Returns whether the
    /// theme changed. A source that cannot be read right now (a symlink
    /// mid-switch) is an `Err` so the caller keeps the current colours.
    pub fn refresh(&self, theme: &mut Theme) -> Result<bool, String> {
        let Some(follow) = theme.follow.clone() else {
            return Ok(false);
        };
        let (palette, name) = read_source(&resolve_selector(&follow))?;
        let next = Theme {
            palette,
            follow: Some(follow),
            source_name: Some(name),
        };
        let changed = next != *theme;
        *theme = next;
        Ok(changed)
    }
}

/// The path to read for a followed source. A `desktop` selector that does not
/// exist at all (as opposed to one that dangles mid-switch, which is left to
/// fail so the colors stay put) falls back to the `current` palette beside it.
fn resolve_selector(follow: &Follow) -> PathBuf {
    if follow.mode == "desktop" && fs::symlink_metadata(&follow.path).is_err() {
        if let Some(current) = follow.path.parent().map(|dir| dir.join("current")) {
            if fs::symlink_metadata(&current).is_ok() {
                return current;
            }
        }
    }
    follow.path.clone()
}

/// Resolve a palette source: a directory (or symlink to one) holding
/// `Colors.qml` and/or `palette.json`, or a single `.qml`/`.json` file.
pub fn read_source(path: &Path) -> Result<(Palette, String), String> {
    let mut real = fs::canonicalize(path)
        .map_err(|error| format!("cannot resolve {}: {error}", path.display()))?;
    if real.is_dir() {
        let colors = real.join("Colors.qml");
        let json = real.join("palette.json");
        let qml = fs::read_to_string(&colors).ok().map(|t| qml_colors(&t));
        let pal = fs::read_to_string(&json).ok().map(|t| json_colors(&t));
        let name = describe(&real);
        let palette = from_sources(qml.as_ref(), pal.as_ref())
            .ok_or_else(|| format!("{} has no readable palette", real.display()))?;
        return Ok((palette, name));
    }
    let text = fs::read_to_string(&real)
        .map_err(|error| format!("cannot read {}: {error}", real.display()))?;
    let name = real
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    real.set_extension("");
    let ext_json = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        || text.trim_start().starts_with('{');
    let palette = if ext_json {
        parse_json_palette(&text)
    } else {
        from_sources(Some(&qml_colors(&text)), None)
    };
    palette.map(|p| (p, name)).ok_or_else(|| format!("{} has no readable palette", path.display()))
}

/// "akane", or "dynamic <name>" for a generated palette.
fn describe(dir: &Path) -> String {
    let name = dir.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let dynamic = dir
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|parent| parent == ".dynamic");
    if dynamic { format!("dynamic {name}") } else { name }
}

/// The colour properties of a QML palette file. `readonly property color
/// name: "#hex"` and `Qt.rgba(...)` values are both understood.
pub fn qml_colors(contents: &str) -> BTreeMap<String, Rgba> {
    contents
        .lines()
        .filter_map(|line| {
            let (head, value) = line.split_once(':')?;
            if !head.contains("property color") {
                return None;
            }
            let name = head.split_whitespace().last()?;
            Some((name.to_owned(), Rgba::parse(value.split("//").next()?)?))
        })
        .collect()
}

/// The flat string entries of a `palette.json`-style object.
fn json_colors(contents: &str) -> BTreeMap<String, Rgba> {
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str(contents) else {
        return BTreeMap::new();
    };
    map.into_iter()
        .filter_map(|(key, value)| Some((key, Rgba::parse(value.as_str()?)?)))
        .collect()
}

/// A standalone JSON file: this program's own `theme.json`, a flat token
/// object, or a `palette.json` with `base`/`text`/`primary`.
fn parse_json_palette(contents: &str) -> Option<Palette> {
    let value: serde_json::Value = serde_json::from_str(contents).ok()?;
    if let Some(colors) = value.get("colors").and_then(|c| c.as_object()) {
        let map = colors
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
            .collect();
        return Some(Palette::default().overlay(&map));
    }
    let flat = json_colors(contents);
    if ["background", "accent", "surface"].iter().any(|t| flat.contains_key(*t)) {
        let map = flat.iter().map(|(k, v)| (k.clone(), v.to_saved())).collect();
        return Some(Palette::default().overlay(&map));
    }
    from_sources(None, Some(&flat))
}

/// Combine a `Colors.qml` and a `palette.json` the way the desktop's
/// palette directories use them. QML wins; JSON fills gaps.
fn from_sources(
    qml: Option<&BTreeMap<String, Rgba>>,
    json: Option<&BTreeMap<String, Rgba>>,
) -> Option<Palette> {
    let pick = |q: &[&str], j: &[&str]| -> Option<Rgba> {
        q.iter()
            .find_map(|k| qml.and_then(|m| m.get(*k)).copied())
            .or_else(|| j.iter().find_map(|k| json.and_then(|m| m.get(*k)).copied()))
    };
    // `background` keeps its alpha; its RGB is the window colour. Alpha-first
    // parsing already left the channels unshifted.
    let background = pick(&["background", "backgroundAlpha", "backgroundAlt"], &["base"])?;
    let text = pick(&["foreground"], &["text"])?;
    let accent = pick(&["primary"], &["primary"])?;
    Some(Palette::derive(&Base {
        background,
        text,
        accent,
        surface: pick(&["backgroundGray"], &["surface0"]),
        muted: pick(&["muted", "foregroundInactive"], &["subtext0"]),
        danger: pick(&["danger"], &["red"]),
        warning: pick(&["warning"], &["yellow"]),
        success: pick(&["success"], &["green"]),
    }))
}

/// Built-in presets for `theme preset NAME`.
pub fn preset(name: &str) -> Option<Palette> {
    let (bg, text, accent) = match name {
        "midnight" => ("#14171f", "#e6e9f0", "#7aa2f7"),
        "paper" => ("#f6f1e7", "#2b2a27", "#b4552d"),
        "forest" => ("#16201b", "#dfe8e0", "#8fbf8a"),
        "rose" => ("#201619", "#f0e2e6", "#e58fa8"),
        "mono" => ("#111111", "#eeeeee", "#bbbbbb"),
        _ => return None,
    };
    let c = |s: &str| Rgba::parse(s).expect("preset colours are valid");
    Some(Palette::derive(&Base {
        background: c(bg),
        text: c(text),
        accent: c(accent),
        surface: None,
        muted: None,
        danger: None,
        warning: None,
        success: None,
    }))
}

pub const PRESETS: [&str; 5] = ["midnight", "paper", "forest", "rose", "mono"];

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    const AKANE: &str = r##"pragma Singleton
Singleton {
    readonly property color background: "#e00e1e36"
    readonly property color backgroundAlt: "#0E1E36"
    readonly property color backgroundAlpha: Qt.rgba(26 / 255, 42 / 255, 70 / 255, 0.88)
    readonly property color backgroundGray: "#2a3a56"
    readonly property color foreground: "#f4b999"
    readonly property color primary: "#9279aa"
    readonly property color muted: "#574f72"
    readonly property color success: "#be6f76"
    readonly property color warning: "#f4b999"
    readonly property color danger: "#d2495b"
}
"##;

    #[test]
    fn akane_background_keeps_rgb_and_alpha() {
        let (palette, _) = {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join("Colors.qml"), AKANE).unwrap();
            read_source(dir.path()).unwrap()
        };
        assert_eq!(palette.background, Rgba { r: 0x0e, g: 0x1e, b: 0x36, a: 0xe0 });
        assert_eq!(palette.surface, Rgba::rgb(0x2a, 0x3a, 0x56));
        assert_eq!(palette.accent, Rgba::rgb(0x92, 0x79, 0xaa));
    }

    #[test]
    fn qt_rgba_background_is_understood() {
        let qml = "Singleton {\n readonly property color background: Qt.rgba(14 / 255, 30 / 255, 54 / 255, 0.88)\n readonly property color foreground: \"#ffffff\"\n readonly property color primary: \"#ff0000\"\n}";
        let map = qml_colors(qml);
        let p = from_sources(Some(&map), None).unwrap();
        assert_eq!((p.background.r, p.background.g, p.background.b), (14, 30, 54));
        assert_eq!(p.background.a, 224);
    }

    #[test]
    fn save_and_load_round_trip_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("cfg"));
        let mut theme = Theme::default();
        theme.palette.background = Rgba::parse("#e00e1e36").unwrap();
        store.save(&theme).unwrap();
        assert_eq!(store.load().unwrap(), theme);
        let text = fs::read_to_string(store.theme_path()).unwrap();
        assert!(text.contains("#e00e1e36"));
        assert_eq!(fs::read_dir(store.dir()).unwrap().count(), 1);
    }

    #[test]
    fn a_damaged_file_is_an_error_not_a_reset() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_owned());
        fs::write(store.theme_path(), "{ nope").unwrap();
        assert!(store.load().is_err());
    }

    #[test]
    fn following_a_selector_tracks_retarget_and_in_place_edits() {
        let root = tempfile::tempdir().unwrap();
        for (name, bg) in [("one", "#101010"), ("two", "#202020")] {
            let d = root.path().join(name);
            fs::create_dir(&d).unwrap();
            fs::write(d.join("Colors.qml"), AKANE.replace("#e00e1e36", bg)).unwrap();
        }
        symlink("one", root.path().join("sel")).unwrap();
        let store = Store::new(root.path().join("cfg"));
        let mut theme = Theme {
            follow: Some(Follow { mode: "desktop".into(), path: root.path().join("sel") }),
            ..Theme::default()
        };
        assert!(store.refresh(&mut theme).unwrap());
        assert_eq!(theme.palette.background.rgb_hex(), "#101010");
        assert!(!store.refresh(&mut theme).unwrap());
        fs::remove_file(root.path().join("sel")).unwrap();
        symlink("two", root.path().join("sel")).unwrap();
        assert!(store.refresh(&mut theme).unwrap());
        assert_eq!(theme.palette.background.rgb_hex(), "#202020");
        // In-place regeneration, same selector target.
        fs::write(root.path().join("two/Colors.qml"), AKANE.replace("#e00e1e36", "#303030")).unwrap();
        assert!(store.refresh(&mut theme).unwrap());
        assert_eq!(theme.palette.background.rgb_hex(), "#303030");
        // Dangling selector keeps the colours.
        fs::remove_file(root.path().join("sel")).unwrap();
        assert!(store.refresh(&mut theme).is_err());
        assert_eq!(theme.palette.background.rgb_hex(), "#303030");
    }

    #[test]
    fn a_missing_selector_falls_back_to_current_but_a_dangling_one_does_not() {
        let root = tempfile::tempdir().unwrap();
        let d = root.path().join("static");
        fs::create_dir(&d).unwrap();
        fs::write(d.join("Colors.qml"), AKANE.replace("#e00e1e36", "#222222")).unwrap();
        symlink("static", root.path().join("current")).unwrap();
        let store = Store::new(root.path().join("cfg"));
        let mut theme = Theme {
            follow: Some(Follow { mode: "desktop".into(), path: root.path().join("sel") }),
            ..Theme::default()
        };
        assert!(store.refresh(&mut theme).unwrap());
        assert_eq!(theme.palette.background.rgb_hex(), "#222222");
        symlink("gone", root.path().join("sel")).unwrap();
        assert!(store.refresh(&mut theme).is_err());
        assert_eq!(theme.palette.background.rgb_hex(), "#222222");
    }

    #[test]
    fn palette_json_fills_a_dynamic_source() {
        let json = r#"{"mode":"dark","base":"110d10","text":"f1e1eb","primary":"e5b9dd","surface0":"211c20","red":"ed98d8"}"#;
        let p = parse_json_palette(json).unwrap();
        assert_eq!(p.background.rgb_hex(), "#110d10");
        assert_eq!(p.danger.rgb_hex(), "#ed98d8");
    }
}

#[cfg(test)]
mod shown_tests {
    use super::*;

    #[test]
    fn muted_is_brighter_when_drawn_and_nothing_else_changes() {
        let p = Palette::default();
        let s = p.shown();
        assert!(s.muted.luminance() > p.muted.luminance());
        assert_eq!(s.text, p.text);
        assert_eq!(s.background, p.background);
    }
}
