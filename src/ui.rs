//! The window: header, search, results, and in-window modals.
//!
//! Every colour comes from the saved palette. Icons are the monochrome
//! Lucide SVGs that gpui-kit ships, painted in the text colour, so they take
//! the theme like everything else.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::base::{SelectableText, TextSelectionLayer, ThemeAppearance};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, ClickEvent,
    Context, Div, ElementId, Entity, FocusHandle, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement, Render, RenderOnce,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled, Window,
    WindowBackgroundAppearance, deferred, div, px,
};
use gpui_omarchy::{IconName, Theme as OmarchyTheme, icon, input};
use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::color::Rgba;
use crate::daily::{self, DailyWord};
use crate::daily_cache::{self, Source};
use crate::hover_lift::HoverLift;
use crate::model::{self, Entry, LANGS, Outcome};
use crate::similar::{self, Profile};
use crate::theme::{Palette, Store, Theme};
use crate::urban::{self, UrbanDef};

/// Panels fade and scale from 98% over 120 ms with an ease-out-cubic curve,
/// and closing runs the same motion backwards.
const MOTION: Duration = Duration::from_millis(120);
const SEARCH_PULSE: Duration = Duration::from_millis(900);
const LOADING_SPIN: Duration = Duration::from_millis(1000);
/// The blur samples the page on a grid of this many pixels, inside a disk of this radius.
const BLUR_STEP: f32 = 2.2;
const BLUR_RADIUS: f32 = 7.0;
const SCRIM_ALPHA: f32 = 0.60;

fn reduced_motion() -> bool {
    std::env::var("DICTIONARY_REDUCED_MOTION").is_ok_and(|v| v == "1")
}

fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

pub fn hsla(color: Rgba) -> Hsla {
    gpui_kit::Rgba {
        r: f32::from(color.r) / 255.0,
        g: f32::from(color.g) / 255.0,
        b: f32::from(color.b) / 255.0,
        a: f32::from(color.a) / 255.0,
    }
    .into()
}

/// The toolkit theme for a palette, so the text field and every default
/// widget colour follow it too.
fn toolkit_theme(p: &Palette) -> OmarchyTheme {
    OmarchyTheme {
        name: "Dictionary".into(),
        appearance: if p.is_light() {
            ThemeAppearance::Light
        } else {
            ThemeAppearance::Dark
        },
        background: hsla(p.background.opaque()),
        surface: hsla(p.surface),
        inset: hsla(p.surface_alt),
        foreground: hsla(p.text),
        secondary: hsla(p.muted),
        bright: hsla(p.text),
        accent: hsla(p.accent),
        on_accent: hsla(p.accent_text),
        selection: hsla(p.selection),
        border: hsla(p.border),
        danger: hsla(p.danger),
        warning: hsla(p.warning),
        success: hsla(p.success),
        font: ".SystemUIFont".into(),
        mono_font: "monospace".into(),
    }
}

fn background_appearance(p: &Palette) -> WindowBackgroundAppearance {
    // Opacity is honoured when the compositor supports it; the RGB channels
    // are the same either way.
    if p.background.a < 255 {
        WindowBackgroundAppearance::Blurred
    } else {
        WindowBackgroundAppearance::Opaque
    }
}

fn daily_needs_refresh(loaded: bool, fetched_at: Option<Instant>) -> bool {
    !loaded || fetched_at.is_none_or(|at| at.elapsed() >= Duration::from_secs(60 * 60))
}

fn update_daily_result(
    current: &mut Option<Result<Vec<DailyWord>, String>>,
    result: Result<Vec<DailyWord>, String>,
) {
    if result.is_ok() || !current.as_ref().is_some_and(Result::is_ok) {
        *current = Some(result);
    }
}

enum View {
    Idle,
    Loading(String),
    Done(String, Outcome),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Language,
    Help,
}

struct Modal {
    kind: Kind,
    closing: bool,
    /// Distinguishes one open or close from the next so each animates once.
    id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SourceVisibility {
    dictionary_loading: bool,
    dictionary_results: bool,
    urban_loading: bool,
    urban_results: bool,
    no_results: bool,
}

fn source_visibility(
    dictionary_pending: bool,
    dictionary_found: bool,
    dictionary_failed: bool,
    urban_enabled: bool,
    urban_pending: bool,
    urban_found: bool,
) -> SourceVisibility {
    let urban_results = urban_enabled && urban_found;
    let urban_loading =
        urban_enabled && urban_pending && !urban_results && !dictionary_pending;
    SourceVisibility {
        dictionary_loading: dictionary_pending,
        dictionary_results: dictionary_found,
        urban_loading,
        urban_results,
        no_results: !dictionary_pending
            && dictionary_failed
            && !dictionary_found
            && !urban_loading
            && !urban_results,
    }
}

pub struct Dictionary {
    theme: Theme,
    input: Entity<InputState>,
    lang: &'static str,
    view: View,
    seq: u64,
    modal: Option<Modal>,
    modal_row: usize,
    /// Urban Dictionary results for the current word; `None` until asked.
    urban: Option<Result<Vec<UrbanDef>, String>>,
    urban_pending: bool,
    similar_words: Vec<String>,
    similar_cancel: Option<Arc<AtomicBool>>,
    /// Whether the Urban Dictionary section is shown (Ctrl U).
    urban_on: bool,
    daily_wiktionary: Option<Result<Vec<DailyWord>, String>>,
    daily_urban: Option<Result<Vec<DailyWord>, String>>,
    daily_wiktionary_pending: bool,
    daily_urban_pending: bool,
    daily_wiktionary_fetched: Option<Instant>,
    daily_urban_fetched: Option<Instant>,
    focus: FocusHandle,
    _watcher: Option<RecommendedWatcher>,
}

impl Dictionary {
    pub fn new(
        store: Store,
        word: Option<String>,
        lang: Option<String>,
        panel: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut theme = store.load().unwrap_or_default();
        if store.refresh(&mut theme).unwrap_or(false) {
            let _ = store.save(&theme);
        }
        apply_theme(&theme.palette, window, cx);

        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search for a word")
        });
        cx.subscribe_in(&input, window, |this, _, event, _window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit(cx);
            }
        })
        .detach();

        let dirty = Arc::new(AtomicBool::new(false));
        let watcher = watch(&store, &dirty);
        let watched = store.clone();
        cx.spawn_in(window, async move |this, cx| {
            let mut tick = 0_u32;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                if this.update_in(cx, |_, _, _| ()).is_err() {
                    break;
                }
                tick = tick.wrapping_add(1);
                // The file watcher flags edits at once; the poll is the
                // safety net, and the only way to notice a followed
                // palette that changes while the settings file does not.
                if !dirty.swap(false, Ordering::SeqCst) && tick % 4 != 0 {
                    continue;
                }
                let store = watched.clone();
                let next = cx
                    .background_executor()
                    .spawn(async move { sync(&store) })
                    .await;
                let Some(next) = next else {
                    if this.update_in(cx, |_, _, _| ()).is_err() {
                        break;
                    }
                    continue;
                };
                let alive = this
                    .update_in(cx, |this, window, cx| {
                        if this.theme != next {
                            apply_theme(&next.palette, window, cx);
                            this.theme = next;
                            cx.notify();
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();

        let lang = lang
            .as_deref()
            .and_then(|code| LANGS.iter().find(|l| l.0 == code))
            .map_or("en", |l| l.0);
        let cached_wiktionary = (!cfg!(test)).then(|| daily_cache::load(Source::Wiktionary)).flatten();
        let cached_urban = (!cfg!(test)).then(|| daily_cache::load(Source::Urban)).flatten();
        let mut this = Self {
            theme,
            input,
            lang,
            view: View::Idle,
            seq: 0,
            modal: None,
            modal_row: 0,
            urban: None,
            urban_pending: false,
            urban_on: true,
            similar_words: Vec::new(),
            similar_cancel: None,
            daily_wiktionary: cached_wiktionary.as_ref().map(|cache| Ok(cache.words.clone())),
            daily_urban: cached_urban.as_ref().map(|cache| Ok(cache.words.clone())),
            daily_wiktionary_pending: false,
            daily_urban_pending: false,
            daily_wiktionary_fetched: cached_wiktionary.map(|cache| cache.fetched_at),
            daily_urban_fetched: cached_urban.map(|cache| cache.fetched_at),
            focus: cx.focus_handle(),
            _watcher: watcher,
        };
        if let Some(word) = word {
            this.input.update(cx, |state, cx| {
                state.set_value(word.clone(), window, cx)
            });
            this.search(word, cx);
        } else if !cfg!(test) {
            this.ensure_daily(cx);
        }
        match panel.as_deref() {
            Some("help") => this.open(Kind::Help, window, cx),
            Some("language") => this.open(Kind::Language, window, cx),
            _ => {}
        }
        this
    }

    pub fn focus_input(&self, window: &mut Window, cx: &mut App) {
        let input = self.input.clone();
        input.update(cx, |state, cx| state.focus(window, cx));
    }

    fn palette(&self) -> Palette {
        self.theme.palette.shown()
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let word = self.input.read(cx).value().trim().to_owned();
        if word.is_empty() {
            self.seq += 1;
            self.reset_similar();
            self.view = View::Idle;
            self.urban = None;
            self.urban_pending = false;
            self.ensure_daily(cx);
            cx.notify();
        } else {
            self.search(word, cx);
        }
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |state, cx| state.set_value("", window, cx));
        self.submit(cx);
        self.focus_input(window, cx);
    }

    fn ensure_daily(&mut self, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        if daily_needs_refresh(self.daily_wiktionary.is_some(), self.daily_wiktionary_fetched) && !self.daily_wiktionary_pending {
            self.request_daily_wiktionary(cx);
        }
        if self.urban_on
            && daily_needs_refresh(self.daily_urban.is_some(), self.daily_urban_fetched)
            && !self.daily_urban_pending
        {
            self.request_daily_urban(cx);
        }
    }

    fn request_daily_wiktionary(&mut self, cx: &mut Context<Self>) {
        self.daily_wiktionary_pending = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async {
                    let result = daily::wiktionary();
                    if let Ok(words) = &result {
                        daily_cache::save(Source::Wiktionary, words);
                    }
                    result
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                update_daily_result(&mut this.daily_wiktionary, result);
                this.daily_wiktionary_fetched = Some(Instant::now());
                this.daily_wiktionary_pending = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn request_daily_urban(&mut self, cx: &mut Context<Self>) {
        if !self.urban_on {
            return;
        }
        self.daily_urban_pending = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async {
                    let result = daily::urban();
                    if let Ok(words) = &result {
                        daily_cache::save(Source::Urban, words);
                    }
                    result
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                update_daily_result(&mut this.daily_urban, result);
                this.daily_urban_fetched = Some(Instant::now());
                this.daily_urban_pending = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn retry_daily(&mut self, urban: bool, cx: &mut Context<Self>) {
        if (urban && self.daily_urban_pending) || (!urban && self.daily_wiktionary_pending) {
            return;
        }
        if urban {
            self.daily_urban = None;
            self.request_daily_urban(cx);
        } else {
            self.daily_wiktionary = None;
            self.request_daily_wiktionary(cx);
        }
    }

    fn search(&mut self, word: String, cx: &mut Context<Self>) {
        self.seq += 1;
        self.reset_similar();
        let seq = self.seq;
        let lang = self.lang;
        self.view = View::Loading(word.clone());
        self.urban = None;
        self.urban_pending = false;
        cx.notify();
        if self.urban_on && lang == "en" {
            self.request_urban(word.clone(), seq, cx);
        }
        cx.spawn(async move |this, cx| {
            let query = word.clone();
            let outcome = cx
                .background_executor()
                .spawn(async move { model::lookup(&query, lang) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.seq == seq {
                    this.view = View::Done(word, outcome);
                    this.request_similar(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn request_urban(
        &mut self,
        word: String,
        seq: u64,
        cx: &mut Context<Self>,
    ) {
        self.urban_pending = true;
        let query = word;
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async move { urban::urban(&query) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.seq == seq {
                    this.urban = Some(found);
                    this.urban_pending = false;
                    this.request_similar(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn reset_similar(&mut self) {
        if let Some(cancelled) = self.similar_cancel.take() {
            cancelled.store(true, Ordering::Relaxed);
        }
        self.similar_words.clear();
    }

    fn request_similar(&mut self, cx: &mut Context<Self>) {
        self.reset_similar();
        let View::Done(word, outcome) = &self.view else {
            return;
        };
        if self.lang != "en" || (self.urban_on && self.urban_pending) {
            return;
        }
        let limit = similar::limit();
        if limit == 0 {
            return;
        }
        let entry = match outcome {
            Outcome::Found(entry) => Some(entry),
            _ => None,
        };
        let urban = if self.urban_on {
            self.urban
                .as_ref()
                .and_then(|result| result.as_ref().ok())
                .map(Vec::as_slice)
                .unwrap_or_default()
        } else {
            &[]
        };
        let profile = Profile::new(word, entry, urban);
        let seq = self.seq;
        let cancelled = Arc::new(AtomicBool::new(false));
        self.similar_cancel = Some(cancelled.clone());
        cx.spawn(async move |this, cx| {
            let worker_cancel = cancelled.clone();
            let words =
                cx.background_executor()
                    .spawn(async move {
                        similar::find(&profile, limit, &worker_cancel)
                    })
                    .await;
            let _ = this.update(cx, |this, cx| {
                if this.seq == seq && !cancelled.load(Ordering::Relaxed) {
                    this.similar_words = words;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn pick(
        &mut self,
        word: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.input.update(cx, |state, cx| {
            state.set_value(word.to_owned(), window, cx)
        });
        self.search(word.to_owned(), cx);
    }

    fn pick_daily(
        &mut self,
        word: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.lang = "en";
        self.pick(word, window, cx);
    }

    fn open(
        &mut self,
        kind: Kind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.modal.as_ref().map_or(0, |m| m.id) + 1;
        self.modal_row =
            LANGS.iter().position(|l| l.0 == self.lang).unwrap_or(0);
        self.modal = Some(Modal {
            kind,
            closing: false,
            id,
        });
        // Keys go to the window while a panel is up, not the text field.
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.modal.as_mut() else {
            return;
        };
        if modal.closing {
            return;
        }
        modal.closing = true;
        modal.id += 1;
        let id = modal.id;
        self.focus_input(window, cx);
        cx.notify();
        let wait = if reduced_motion() {
            Duration::ZERO
        } else {
            MOTION
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |this, cx| {
                if this.modal.as_ref().is_some_and(|m| m.id == id && m.closing)
                {
                    this.modal = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn choose_language(
        &mut self,
        row: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.lang = LANGS[row].0;
        self.close(window, cx);
        let word = self.input.read(cx).value().trim().to_owned();
        if !word.is_empty() {
            self.search(word, cx);
        }
    }

    fn key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let ctrl = event.keystroke.modifiers.control;
        if self.modal.as_ref().is_some_and(|m| !m.closing) {
            let kind = self.modal.as_ref().map(|m| m.kind);
            match key {
                "escape" => self.close(window, cx),
                "f1" => self.close(window, cx),
                "down" if kind == Some(Kind::Language) => {
                    self.modal_row = (self.modal_row + 1) % LANGS.len();
                    cx.notify();
                }
                "up" if kind == Some(Kind::Language) => {
                    self.modal_row =
                        (self.modal_row + LANGS.len() - 1) % LANGS.len();
                    cx.notify();
                }
                "enter" if kind == Some(Kind::Language) => {
                    self.choose_language(self.modal_row, window, cx)
                }
                _ => {}
            }
            return;
        }
        match key {
            "1" if ctrl && matches!(self.view, View::Idle) => {
                if let Some(daily) = self.daily_wiktionary.as_ref().and_then(|r| r.as_ref().ok()).and_then(|words| words.first()) {
                    let word = daily.word.clone();
                    self.pick_daily(&word, window, cx);
                }
            }
            "2" if ctrl && matches!(self.view, View::Idle) && self.urban_on => {
                if let Some(daily) = self.daily_urban.as_ref().and_then(|r| r.as_ref().ok()).and_then(|words| words.first()) {
                    let word = daily.word.clone();
                    self.pick_daily(&word, window, cx);
                }
            }
            "escape" => {
                if self.input.read(cx).value().is_empty() {
                    self.seq += 1;
                    self.reset_similar();
                    self.view = View::Idle;
                    self.urban = None;
                    self.urban_pending = false;
                    self.ensure_daily(cx);
                } else {
                    self.input.update(cx, |state, cx| {
                        state.set_value("", window, cx)
                    });
                }
                cx.notify();
            }
            "l" if ctrl => self.open(Kind::Language, window, cx),
            "u" if ctrl => {
                self.urban_on = !self.urban_on;
                let pending = match &self.view {
                    View::Loading(word) => Some(word.clone()),
                    View::Done(word, _) => Some(word.clone()),
                    _ => None,
                };
                if let Some(word) = pending.filter(|_| {
                    self.urban_on
                        && self.lang == "en"
                        && self.urban.is_none()
                        && !self.urban_pending
                }) {
                    self.request_urban(word, self.seq, cx);
                }
                if self.urban_on && matches!(self.view, View::Idle) {
                    self.ensure_daily(cx);
                }
                self.request_similar(cx);
                cx.notify();
            }
            "f1" => self.open(Kind::Help, window, cx),
            _ => {}
        }
    }
}

/// Load the saved theme, follow its source, and persist what changed.
fn sync(store: &Store) -> Option<Theme> {
    let mut theme = store.load().ok()?;
    if store.refresh(&mut theme).unwrap_or(false) {
        let _ = store.save(&theme);
    }
    Some(theme)
}

/// Watch the settings *directory*: an atomic rename replaces the file's
/// inode, which a watch on the file itself would lose.
fn watch(store: &Store, dirty: &Arc<AtomicBool>) -> Option<RecommendedWatcher> {
    std::fs::create_dir_all(store.dir()).ok()?;
    let flag = dirty.clone();
    let mut watcher = notify::recommended_watcher(move |_| {
        flag.store(true, Ordering::SeqCst)
    })
    .ok()?;
    watcher
        .watch(store.dir(), RecursiveMode::NonRecursive)
        .ok()?;
    Some(watcher)
}

fn apply_theme(palette: &Palette, window: &mut Window, cx: &mut App) {
    toolkit_theme(&palette.shown()).apply(cx);
    window.set_background_appearance(background_appearance(palette));
}

// ---------------------------------------------------------------- drawing

fn pill(id: impl Into<ElementId>, p: &Palette, accent: bool) -> Stateful<Div> {
    let (bg, fg, hover) = if accent {
        (
            hsla(p.accent),
            hsla(p.accent_text),
            hsla(p.accent).opacity(0.85),
        )
    } else {
        (hsla(p.surface), hsla(p.text), hsla(p.surface_alt))
    };
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(40.))
        .px(px(14.))
        .rounded(px(12.))
        .bg(bg)
        .text_color(fg)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
}

fn ic(name: IconName, size: f32) -> impl IntoElement {
    icon(name).size(px(size))
}

impl Dictionary {
    fn header(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let lang = model::lang_name(self.lang);
        let caption = self.caption();
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .size(px(40.))
                    .rounded(px(12.))
                    .bg(hsla(p.accent))
                    .text_color(hsla(p.accent_text))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(ic(IconName::BookOpen, 22.)),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Dictionary"),
                    )
                    .when(!caption.is_empty(), |d| {
                        d.child(
                            div()
                                .text_size(px(12.))
                                .text_color(hsla(p.muted))
                                .child(caption),
                        )
                    }),
            )
            .child(
                pill("lang", &p, false)
                    .child(ic(IconName::Globe, 16.))
                    .child(div().text_size(px(13.)).child(lang))
                    .child(ic(IconName::ChevronDown, 14.))
                    .on_click(cx.listener(
                        |this, _: &ClickEvent, window, cx| {
                            this.open(Kind::Language, window, cx)
                        },
                    )),
            )
            .child(
                pill("help", &p, false)
                    .px(px(11.))
                    .child(ic(IconName::Info, 16.))
                    .on_click(cx.listener(
                        |this, _: &ClickEvent, window, cx| {
                            this.open(Kind::Help, window, cx)
                        },
                    )),
            )
    }

    fn caption(&self) -> String {
        match &self.view {
            View::Idle => String::new(),
            View::Loading(_) => "Looking up…".into(),
            View::Done(_, Outcome::Found(_)) => String::new(),
            View::Done(
                _,
                Outcome::Missing { .. } | Outcome::Offline { .. },
            ) if self.urban_on && self.lang == "en" => {
                match self.urban.as_ref() {
                    None => "Looking up…".into(),
                    Some(Ok(defs)) if !defs.is_empty() => String::new(),
                    _ if matches!(
                        self.view,
                        View::Done(_, Outcome::Offline { .. })
                    ) =>
                    {
                        "Could not reach Wiktionary".into()
                    }
                    _ => "No definition".into(),
                }
            }
            View::Done(_, Outcome::Missing { .. }) => "No definition".into(),
            View::Done(_, Outcome::Offline { .. }) => {
                "Could not reach Wiktionary".into()
            }
        }
    }

    /// The live text field and search button.
    fn search_row(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let search_icon = div()
            .size(px(16.))
            .flex()
            .items_center()
            .justify_center()
            .child(ic(IconName::Search, 16.));
        let search_icon = if self.is_search_pending() && !reduced_motion() {
            search_icon
                .with_animation(
                    ElementId::from("search-pending"),
                    Animation::new(SEARCH_PULSE).repeat(),
                    |element, t| {
                        element.opacity(
                            1.0 - 0.22 * (std::f32::consts::PI * t).sin(),
                        )
                    },
                )
                .into_any_element()
        } else {
            search_icon.into_any_element()
        };
        let field = input("query", &self.input, window, cx)
            .rounded(px(12.))
            .h(px(44.))
            .text_size(px(15.))
            .prefix(search_icon)
            .into_any_element();
        div()
            .flex()
            .gap(px(10.))
            .child(div().flex_1().min_w_0().child(field))
            .child(
                pill("go", &p, true)
                    .h(px(44.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Search"),
                    )
                    .child(ic(IconName::ArrowRight, 16.))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.submit(cx)
                    })),
            )
            .child(
                pill("clear", &p, false)
                    .h(px(44.))
                    .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Clear"))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.clear_search(window, cx)
                    })),
            )
    }

    fn is_search_pending(&self) -> bool {
        matches!(self.view, View::Loading(_))
            || (self.urban_on && self.lang == "en" && self.urban_pending)
    }

    /// The same row drawn without behaviour, for the blurred copies.
    fn search_row_still(&self, cx: &App) -> Div {
        let p = self.palette();
        let text = self.input.read(cx).value();
        let shown = if text.is_empty() {
            "Search for a word".into()
        } else {
            text
        };
        let muted = hsla(p.muted);
        div()
            .flex()
            .gap(px(10.))
            .child(
                div()
                    .flex_1()
                    .h(px(44.))
                    .px(px(12.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(hsla(p.border))
                    .bg(hsla(p.surface))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(15.))
                    .child(
                        div()
                            .text_color(muted)
                            .child(ic(IconName::Search, 16.)),
                    )
                    .child(div().child(shown)),
            )
            .child(
                pill("go-still", &p, true)
                    .h(px(44.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Search"),
                    )
                    .child(ic(IconName::ArrowRight, 16.)),
            )
            .child(
                pill("clear-still", &p, false)
                    .h(px(44.))
                    .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Clear")),
            )
    }

    fn body(&self, scroll: bool, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();
        let content = match &self.view {
            View::Idle => self.daily_home(&p, scroll, cx),
            View::Loading(word) => self.loading_results(&p, word, scroll),
            View::Done(word, outcome) => {
                self.done_results(&p, word, outcome, scroll, cx)
            }
        };
        let card = div().flex_1().min_h_0().child(if scroll {
            div()
                .id("results")
                .size_full()
                .overflow_y_scroll()
                .child(content)
                .into_any_element()
        } else {
            div().size_full().child(content).into_any_element()
        });
        card.into_any_element()
    }

    fn daily_home(
        &self,
        p: &Palette,
        live: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut column = div()
            .flex()
            .flex_col()
            .gap(px(22.))
            .child(
                div()
                    .text_size(px(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Words of the day"),
            )
            .child(self.daily_panel(
                p,
                "Wiktionary",
                self.daily_wiktionary.as_ref(),
                self.daily_wiktionary_pending,
                false,
                live,
                cx,
            ));
        if self.urban_on {
            column = column.child(self.daily_panel(
                p,
                "Urban Dictionary",
                self.daily_urban.as_ref(),
                self.daily_urban_pending,
                true,
                live,
                cx,
            ));
        }
        column.into_any_element()
    }

    fn daily_panel(
        &self,
        p: &Palette,
        source: &'static str,
        result: Option<&Result<Vec<DailyWord>, String>>,
        pending: bool,
        urban_source: bool,
        live: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let content = match result {
            Some(Ok(words)) => {
                let mut body = div().flex().flex_col().gap(px(28.));
                for (index, daily) in words.iter().take(2).enumerate() {
                    let word = daily.word.clone();
                    let title = if live {
                        div()
                            .id(format!("daily-{source}-word-{index}"))
                            .cursor_pointer()
                            .hover(|style| style.underline())
                            .child(daily.word.clone())
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.pick_daily(&word, window, cx)
                            }))
                            .into_any_element()
                    } else {
                        div().child(daily.word.clone()).into_any_element()
                    };
                    let credits = daily_credits(daily, urban_source);
                    body = body.child(daily_entry_content(p, daily, source, live, &credits, Some(title)));
                }
                body.into_any_element()
            }
            Some(Err(_)) => {
                let mut row = div().flex().items_center().gap(px(12.)).child(
                    div()
                        .flex_1()
                        .text_size(px(14.))
                        .text_color(hsla(p.muted))
                        .child("Could not load today's words."),
                );
                if live {
                    let id = if urban_source {
                        "daily-urban-retry"
                    } else {
                        "daily-wiktionary-retry"
                    };
                    row = row.child(
                        pill(id, p, false)
                            .h(px(34.))
                            .child(div().text_size(px(12.)).child("Try again"))
                            .on_click(cx.listener(
                                move |this, _: &ClickEvent, _, cx| {
                                    this.retry_daily(urban_source, cx)
                                },
                            )),
                    );
                }
                row.into_any_element()
            }
            _ => centered(
                p,
                IconName::Loader,
                &if pending {
                    format!("Loading {source} words…")
                } else {
                    format!("{source} words are not loaded.")
                },
                "",
                live && pending,
            ),
        };
        result_panel(p).child(content)
    }

    fn loading_results(
        &self,
        p: &Palette,
        word: &str,
        scroll: bool,
    ) -> AnyElement {
        let urban_enabled = self.urban_on && self.lang == "en";
        let urban_found = urban_enabled
            && self.urban.as_ref().is_some_and(|result| {
                result.as_ref().is_ok_and(|defs| !defs.is_empty())
            });
        let visibility = source_visibility(
            true,
            false,
            false,
            urban_enabled,
            self.urban_pending,
            urban_found,
        );
        let loading = if urban_enabled && self.urban_pending {
            centered(p, IconName::Loader, &format!("Looking up “{word}”…"), "", scroll)
        } else {
            source_loading(p, "Dictionary", word, scroll)
        };
        let mut column = div()
            .flex()
            .flex_col()
            .gap(px(22.))
            .child(result_panel(p).child(loading));
        if visibility.urban_results {
            column = column.child(result_panel(p).child(urban_view(
                p,
                word,
                self.urban.as_ref(),
                scroll,
            )));
        }
        column.into_any_element()
    }

    fn done_results(
        &self,
        p: &Palette,
        word: &str,
        outcome: &Outcome,
        scroll: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let urban_enabled = self.urban_on && self.lang == "en";
        let primary_found = matches!(outcome, Outcome::Found(_));
        let urban_found = urban_enabled
            && self.urban.as_ref().is_some_and(|result| {
                result.as_ref().is_ok_and(|defs| !defs.is_empty())
            });
        let visibility = source_visibility(
            false,
            primary_found,
            !primary_found,
            urban_enabled,
            self.urban_pending,
            urban_found,
        );
        let primary = match outcome {
            Outcome::Found(entry) => {
                Some(result_panel(p).child(entry_view(p, entry, scroll)))
            }
            Outcome::Missing { .. } | Outcome::Offline { .. } => None,
        };
        let urban = urban_found.then(|| {
            result_panel(p).child(urban_view(
                p,
                word,
                self.urban.as_ref(),
                scroll,
            ))
        });

        if visibility.dictionary_results || visibility.urban_results {
            let mut column = div().flex().flex_col().gap(px(22.));
            if let Some(primary) = primary {
                column = column.child(primary);
            }
            if let Some(urban) = urban {
                column = column.child(urban);
            } else if visibility.urban_loading {
                column = column.child(result_panel(p).child(source_loading(
                    p,
                    "Urban Dictionary",
                    word,
                    scroll,
                )));
            }
            if !self.similar_words.is_empty() {
                column = column.child(self.similar_section(p, scroll, cx));
            }
            return column.into_any_element();
        }

        if visibility.urban_loading {
            return result_panel(p)
                .child(source_loading(p, "Urban Dictionary", word, scroll))
                .into_any_element();
        }

        let fallback = match outcome {
            Outcome::Found(_) => centered(p, IconName::Search, "No results", "", false),
            Outcome::Missing { suggestions } => self.miss(p, word, suggestions, "No results", cx),
            Outcome::Offline { suggestions } => self.miss(
                p, word, suggestions, "No results. Could not reach Wiktionary. Check your connection.", cx,
            ),
        };
        debug_assert!(visibility.no_results);
        result_panel(p).child(fallback).into_any_element()
    }

    fn similar_section(
        &self,
        p: &Palette,
        live: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut badges = div().flex().flex_wrap().gap(px(8.));
        for (index, word) in self.similar_words.iter().enumerate() {
            if live {
                let word = word.clone();
                badges = badges.child(
                    similar_word_badge(p, &word, true)
                        .id(("similar-word", index))
                        .cursor_pointer()
                        .debug_selector(move || format!("similar-word-{index}"))
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, window, cx| {
                                this.pick(&word, window, cx)
                            },
                        )),
                );
            } else {
                badges = badges.child(similar_word_badge(p, word, false));
            }
        }
        result_panel(p).child(
            div()
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(
                    div()
                        .text_size(px(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Explore similar words"),
                )
                .child(badges),
        )
    }

    fn miss(
        &self,
        p: &Palette,
        word: &str,
        suggestions: &[String],
        message: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut column = div().flex().flex_col().gap(px(14.)).child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .text_color(hsla(p.warning))
                .child(ic(IconName::Info, 18.))
                .child(
                    div()
                        .text_size(px(15.))
                        .child(format!("“{word}”: {message}")),
                ),
        );
        if !suggestions.is_empty() {
            let mut chips =
                div().flex().flex_wrap().items_center().gap(px(8.)).child(
                    div()
                        .text_size(px(13.))
                        .text_color(hsla(p.muted))
                        .child("Did you mean"),
                );
            for (index, suggestion) in suggestions.iter().enumerate() {
                let value = suggestion.clone();
                chips = chips.child(
                    pill(("chip", index), p, false)
                        .h(px(32.))
                        .px(px(12.))
                        .child(
                            div().text_size(px(13.)).child(suggestion.clone()),
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, window, cx| {
                                this.pick(&value, window, cx)
                            },
                        )),
                );
            }
            column = column.child(chips);
        }
        column.into_any_element()
    }

    /// The whole page. `live` pages carry the real text field and scroll.
    fn page(&self, live: bool, window: &Window, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(16.))
            .p(px(22.))
            .text_color(hsla(p.text))
            .child(self.header(cx))
            .child(if live {
                self.search_row(window, cx)
            } else {
                self.search_row_still(cx)
            })
            .child(self.body(live, cx))
    }

    fn modal_layer(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let modal = self.modal.as_ref()?;
        let p = self.palette();
        let (kind, closing, id) = (modal.kind, modal.closing, modal.id);
        let duration = if reduced_motion() {
            Duration::from_millis(1)
        } else {
            MOTION
        };

        // A frozen, blurred copy of the page. Each copy is opaque and layer k
        // has weight 1/(k+1), so the stack is a running mean of the samples
        // (a disk of offsets on a grid about a stroke wide). The first copy
        // is unshifted and covers the live page.
        let bg = hsla(p.background.opaque());
        let mut still = |dx: f32, dy: f32, weight: f32, n: usize| {
            div()
                .id(("still", n))
                .absolute()
                .top(px(dy))
                .left(px(dx))
                .size_full()
                .bg(bg)
                .opacity(weight)
                .child(self.page(false, window, cx))
        };
        let mut offsets: Vec<(f32, f32)> = Vec::new();
        for j in -3_i32..=3 {
            for i in -3_i32..=3 {
                let (x, y) = (i as f32 * BLUR_STEP, j as f32 * BLUR_STEP);
                if (x, y) != (0.0, 0.0) && x.hypot(y) <= BLUR_RADIUS {
                    offsets.push((x, y));
                }
            }
        }
        let mut blur = div()
            .absolute()
            .inset_0()
            .overflow_hidden()
            .child(still(0.0, 0.0, 1.0, 0));
        for (k, (x, y)) in offsets.into_iter().enumerate() {
            blur = blur.child(still(x, y, 1.0 / (k as f32 + 2.0), k + 1));
        }
        let scrim = hsla(p.background.opaque()).opacity(SCRIM_ALPHA);
        let card = self.card(kind, &p, cx);

        let progress = move |t: f32| {
            let eased = ease_out_cubic(t);
            if closing { 1.0 - eased } else { eased }
        };
        let layer = div()
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|this, _, window, cx| this.close(window, cx)),
            )
            .child(blur.with_animation(
                ElementId::from(("blur", id)),
                Animation::new(duration),
                move |el, t| el.opacity(progress(t)),
            ))
            .child(div().absolute().inset_0().bg(scrim).with_animation(
                ElementId::from(("scrim", id)),
                Animation::new(duration),
                move |el, t| el.opacity(progress(t)),
            ))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(card.with_animation(
                        ElementId::from(("card", id)),
                        Animation::new(duration),
                        move |el, t| {
                            let v = progress(t);
                            el.opacity(v)
                                .w(px(480.0 * (0.98 + 0.02 * v)))
                                .mt(px((1.0 - v) * 6.0))
                        },
                    )),
            );
        Some(deferred(layer).with_priority(10).into_any_element())
    }

    fn card(
        &self,
        kind: Kind,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let (title, subtitle) = match kind {
            Kind::Language => (
                "Language",
                "Choose a Wiktionary edition. English also searches Webster's 1913 offline.",
            ),
            Kind::Help => {
                ("Dictionary", "")
            }
        };
        let mut card = div()
            .id("modal-card")
            .occlude()
            .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                cx.stop_propagation()
            })
            .max_h(px(560.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(24.))
            .rounded(px(18.))
            .bg(hsla(p.surface))
            .border_1()
            .border_color(hsla(p.border))
            .text_color(hsla(p.text))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .when(!subtitle.is_empty(), |d| {
                                d.child(
                                    div()
                                        .text_size(px(13.))
                                        .text_color(hsla(p.muted))
                                        .child(subtitle),
                                )
                            }),
                    )
                    .child(
                        pill("modal-close", p, false)
                            .size(px(34.))
                            .px(px(0.))
                            .justify_center()
                            .child(ic(IconName::Close, 16.))
                            .on_click(cx.listener(
                                |this, _: &ClickEvent, window, cx| {
                                    this.close(window, cx)
                                },
                            )),
                    ),
            );
        match kind {
            Kind::Language => {
                let mut list = div()
                    .id("lang-list")
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .overflow_y_scroll()
                    .max_h(px(380.));
                for (row, (code, name, _)) in LANGS.iter().enumerate() {
                    let current = *code == self.lang;
                    let highlighted = row == self.modal_row;
                    list = list.child(
                        div()
                            .id(("lang-row", row))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .h(px(38.))
                            .px(px(12.))
                            .rounded(px(10.))
                            .cursor_pointer()
                            .when(highlighted, |d| d.bg(hsla(p.selection)))
                            .hover({
                                let hover = hsla(p.surface_alt);
                                move |s| s.bg(hover)
                            })
                            .child(
                                div().flex_1().text_size(px(14.)).child(*name),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(hsla(p.muted))
                                    .child(code.to_uppercase()),
                            )
                            .child(
                                div()
                                    .w(px(16.))
                                    .text_color(hsla(p.accent))
                                    .when(current, |d| {
                                        d.child(ic(IconName::Check, 16.))
                                    }),
                            )
                            .on_click(cx.listener(
                                move |this, _: &ClickEvent, window, cx| {
                                    this.choose_language(row, window, cx)
                                },
                            )),
                    );
                }
                card = card.child(list);
            }
            Kind::Help => {
                let rows = [
                    ("Enter", "Look up the word"),
                    ("Esc", "Clear the field, or close a panel"),
                    ("Ctrl L", "Choose a language"),
                    ("Ctrl U", "Show or hide Urban Dictionary"),
                    ("F1", "Show this panel"),
                ];
                let mut list = div().flex().flex_col().gap(px(8.));
                for (keys, what) in rows {
                    list = list.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(
                                div()
                                    .w(px(72.))
                                    .px(px(8.))
                                    .py(px(3.))
                                    .rounded(px(8.))
                                    .bg(hsla(p.surface_alt))
                                    .border_1()
                                    .border_color(hsla(p.border))
                                    .text_size(px(12.))
                                    .flex()
                                    .justify_center()
                                    .child(keys),
                            )
                            .child(div().text_size(px(14.)).child(what)),
                    );
                }
                card = card.child(list).child(
                    div()
                        .text_size(px(12.))
                        .text_color(hsla(p.muted))
                        .child("English definitions come from Webster's 1913 (public domain text, GCIDE data under the GPL). Other editions use Wiktionary. English lookups also show Urban Dictionary, whose entries can be explicit."),
                );
            }
        }
        card
    }
}

fn status_icon(p: &Palette, name: IconName, id: &str, live: bool) -> AnyElement {
    if !live || name != IconName::Loader || reduced_motion() {
        return ic(name, 40.).into_any_element();
    }
    gpui_kit::svg()
        .path(name.path())
        .size(px(40.))
        .text_color(hsla(p.muted))
        .with_animation(
            ElementId::from(format!("loading-spinner-{id}")),
            Animation::new(LOADING_SPIN).repeat(),
            |icon, t| icon.with_transformation(gpui_kit::Transformation::rotate(
                gpui_kit::Radians(std::f32::consts::TAU * t),
            )),
        )
        .into_any_element()
}

fn centered(
    p: &Palette,
    name: IconName,
    title: &str,
    hint: &str,
    live: bool,
) -> AnyElement {
    div()
        .size_full()
        .min_h(px(180.))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .text_color(hsla(p.muted))
        .child(status_icon(p, name, title, live))
        .child(
            div()
                .text_size(px(16.))
                .text_color(hsla(p.text))
                .child(title.to_owned()),
        )
        .when(!hint.is_empty(), |d| {
            d.child(div().text_size(px(13.)).child(hint.to_owned()))
        })
        .into_any_element()
}

fn result_panel(p: &Palette) -> Div {
    div()
        .w_full()
        .p(px(22.))
        .rounded(px(16.))
        .bg(hsla(p.surface))
        .border_1()
        .border_color(hsla(p.border))
        .overflow_hidden()
}

fn selectable(id: String, text: String, live: bool) -> AnyElement {
    if live {
        div()
            .cursor_text()
            .child(SelectableText::new(id, text))
            .into_any_element()
    } else {
        div().child(text).into_any_element()
    }
}

fn valid_pronunciation(text: &str) -> bool {
    !text.trim().is_empty()
        && text.chars().any(char::is_alphabetic)
        && !text.chars().any(|ch| {
            ch.is_control()
                || matches!(ch, '\u{fffd}' | '\u{fffc}' | '\u{25a1}' | '\u{25af}')
        })
}

#[derive(IntoElement)]
struct PronunciationText {
    id: String,
    text: String,
    live: bool,
}

impl RenderOnce for PronunciationText {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let visible = valid_pronunciation(&self.text) && {
            let style = window.text_style();
            let line = window.text_system().shape_line(
                self.text.clone().into(),
                px(15.),
                &[style.to_run(self.text.len())],
                None,
            );
            // Shaping includes font fallback; glyph zero still means a missing glyph.
            let mut glyphs = line.runs.iter().flat_map(|run| &run.glyphs).peekable();
            glyphs.peek().is_some() && glyphs.all(|glyph| glyph.id.0 != 0)
        };
        div().when(visible, |d| d.child(selectable(self.id, self.text, self.live)))
    }
}

/// The Urban Dictionary section under the entry. Still copies (`full` false)
/// draw a single definition, since the blur paints the page dozens of times.
fn urban_view(
    p: &Palette,
    word: &str,
    found: Option<&Result<Vec<UrbanDef>, String>>,
    full: bool,
) -> AnyElement {
    let title = match found {
        Some(Ok(defs)) if !defs.is_empty() => defs[0].word.to_lowercase(),
        _ => word.to_lowercase(),
    };
    let mut entry = Entry {
        word: title,
        phonetic: String::new(),
        source: "Urban Dictionary",
        groups: vec![],
    };
    let mut credits = Vec::new();
    if let Some(Ok(defs)) = found {
        for def in defs.iter().take(if full { defs.len() } else { 1 }) {
            let pos = def
                .pos
                .map_or_else(|| "N/A".to_owned(), |pos| pos.to_owned());
            if entry.groups.last().is_none_or(|group| group.pos != pos) {
                entry.groups.push(model::Group { pos, defs: vec![] });
            }
            entry.groups.last_mut().unwrap().defs.push(model::Def {
                text: def.text.clone(),
                example: def.example.clone(),
            });
            credits.push(format!("by: {} · {}", def.author, def.date));
        }
    }
    let note = match found {
        None => Some("Looking up…"),
        Some(Err(_)) => Some("Could not reach Urban Dictionary."),
        Some(Ok(defs)) if defs.is_empty() => {
            Some("No entry on Urban Dictionary.")
        }
        _ => None,
    };
    div()
        .child(entry_content(p, &entry, full, &credits))
        .when_some(note, |d, note| {
            d.child(
                div()
                    .text_size(px(13.))
                    .text_color(hsla(p.muted))
                    .child(note),
            )
        })
        .into_any_element()
}

fn source_loading(p: &Palette, source: &str, word: &str, live: bool) -> AnyElement {
    centered(
        p,
        IconName::Loader,
        &format!("Looking up “{word}” on {source}…"),
        "",
        live,
    )
}

fn badge(p: &Palette, text: &str) -> Div {
    div()
        .relative()
        .child(
            badge_face(gpui_kit::hsla(0., 0., 0., 0.35), text)
                .absolute()
                .top(px(3.))
                .left(px(0.)),
        )
        .child(badge_face(hsla(p.accent), text).relative())
}

fn similar_word_badge(p: &Palette, word: &str, live: bool) -> Div {
    let shadow = gpui_kit::hsla(0., 0., 0., 0.35);
    let background = p.surface.mix(Rgba::rgb(255, 255, 255), 0.06);
    let hover = background.mix(Rgba::rgb(0, 0, 0), 0.12);
    let face = similar_word_face(hsla(background), hsla(p.text), word).relative();
    let face = if live {
        face.id("similar-word-face")
            .hover(move |style| style.bg(hsla(hover)))
            .into_any_element()
    } else {
        face.into_any_element()
    };
    let badge = div()
        .relative()
        .max_w_full()
        .rounded(px(12.))
        .child(
            similar_word_face(shadow, shadow, word)
                .absolute()
                .top(px(3.))
                .left(px(0.)),
        )
        .child(face);
    div().max_w_full().child(if live {
        HoverLift::new(
            badge,
            px(3.),
            gpui_kit::base::motion::Transition::new(MOTION).ease(ease_out_cubic),
            reduced_motion(),
        )
        .into_any_element()
    } else {
        badge.into_any_element()
    })
}

fn similar_word_face(background: Hsla, text: Hsla, word: &str) -> Div {
    div()
        .flex()
        .items_center()
        .h(px(32.))
        .px(px(12.))
        .max_w_full()
        .rounded(px(12.))
        .bg(background)
        .text_size(px(13.))
        .text_color(text)
        .child(
            div()
                .overflow_hidden()
                .text_ellipsis()
                .child(word.to_owned()),
        )
}

fn badge_face(color: Hsla, text: &str) -> Div {
    div()
        .px(px(10.))
        .py(px(2.))
        .rounded(px(999.))
        .border_1()
        .border_color(color)
        .text_size(px(11.))
        .text_color(color)
        .child(SharedString::from(text.to_owned()))
}

fn daily_credits(daily: &DailyWord, urban: bool) -> Vec<String> {
    let date = daily.date.as_deref().unwrap_or("");
    let credit = if urban {
        match daily.author.as_deref() {
            Some(author) if !date.is_empty() => format!("by: {author} · {date}"),
            Some(author) => format!("by: {author}"),
            None => date.to_owned(),
        }
    } else {
        date.to_owned()
    };
    if credit.is_empty() { vec![] } else { vec![credit] }
}

fn entry_view(p: &Palette, entry: &Entry, live: bool) -> AnyElement {
    entry_content(p, entry, live, &[])
}

fn entry_content(
    p: &Palette,
    entry: &Entry,
    live: bool,
    credits: &[String],
) -> AnyElement {
    entry_content_with_title(p, entry, live, credits, None)
}

fn entry_content_with_title(
    p: &Palette,
    entry: &Entry,
    live: bool,
    credits: &[String],
    title: Option<AnyElement>,
) -> AnyElement {
    entry_content_from(p, entry, live, credits, title)
}

struct DailyEntry<'a> {
    daily: &'a DailyWord,
    source: &'static str,
}

trait EntryContentSource {
    fn word(&self) -> &str;
    fn phonetic(&self) -> &str;
    fn source(&self) -> &'static str;
    fn group_count(&self) -> usize;
    fn group_pos(&self, group: usize) -> &str;
    fn def_count(&self, group: usize) -> usize;
    fn def_text(&self, group: usize, def: usize) -> &str;
    fn def_example(&self, group: usize, def: usize) -> Option<&str>;
}

impl EntryContentSource for Entry {
    fn word(&self) -> &str {
        &self.word
    }

    fn phonetic(&self) -> &str {
        &self.phonetic
    }

    fn source(&self) -> &'static str {
        self.source
    }

    fn group_count(&self) -> usize {
        self.groups.len()
    }

    fn group_pos(&self, group: usize) -> &str {
        &self.groups[group].pos
    }

    fn def_count(&self, group: usize) -> usize {
        self.groups[group].defs.len()
    }

    fn def_text(&self, group: usize, def: usize) -> &str {
        &self.groups[group].defs[def].text
    }

    fn def_example(&self, group: usize, def: usize) -> Option<&str> {
        self.groups[group].defs[def].example.as_deref()
    }
}

impl EntryContentSource for DailyEntry<'_> {
    fn word(&self) -> &str {
        &self.daily.word
    }

    fn phonetic(&self) -> &str {
        &self.daily.phonetic
    }

    fn source(&self) -> &'static str {
        self.source
    }

    fn group_count(&self) -> usize {
        1
    }

    fn group_pos(&self, _: usize) -> &str {
        self.daily.pos.as_deref().unwrap_or("N/A")
    }

    fn def_count(&self, _: usize) -> usize {
        1
    }

    fn def_text(&self, _: usize, _: usize) -> &str {
        &self.daily.text
    }

    fn def_example(&self, _: usize, _: usize) -> Option<&str> {
        self.daily.example.as_deref()
    }
}

fn daily_entry_content(
    p: &Palette,
    daily: &DailyWord,
    source: &'static str,
    live: bool,
    credits: &[String],
    title: Option<AnyElement>,
) -> AnyElement {
    let data = DailyEntry { daily, source };
    entry_content_from(p, &data, live, credits, title)
}

fn entry_content_from<T: EntryContentSource>(
    p: &Palette,
    entry: &T,
    live: bool,
    credits: &[String],
    title: Option<AnyElement>,
) -> AnyElement {
    let mut credit_index = 0;
    let word = entry.word();
    let source = entry.source();
    let heading = div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .text_size(px(30.))
                        .line_height(px(32.))
                        .font_weight(FontWeight::BOLD)
                        .child(title.unwrap_or_else(|| selectable(
                            format!("{source}-entry-title-{word}"),
                            word.to_owned(),
                            live,
                        ))),
                )
                .child(badge(p, source)),
        )
        .when(valid_pronunciation(entry.phonetic()), |d| {
            d.child(
                div()
                    .text_size(px(15.))
                    .line_height(px(18.))
                    .italic()
                    .text_color(hsla(p.muted))
                    .child(PronunciationText {
                        id: format!("{source}-{word}-entry-phonetic"),
                        text: entry.phonetic().to_owned(),
                        live,
                    }),
            )
        });
    let mut column = div().flex().flex_col().gap(px(18.)).child(heading);
    for group_index in 0..entry.group_count() {
        let pos = entry.group_pos(group_index);
        let mut section = div().flex().flex_col().gap(px(10.));
        if !pos.is_empty() {
            section = section.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(hsla(p.accent))
                            .child(selectable(
                                format!(
                                    "{}-{}-entry-pos-{group_index}-{}",
                                    source, word, pos
                                ),
                                pos.to_uppercase(),
                                live,
                            )),
                    )
                    .child(div().flex_1().h(px(1.)).bg(hsla(p.border))),
            );
        }
        for index in 0..entry.def_count(group_index) {
            let def = entry.def_text(group_index, index);
            section = section.child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(div().w(px(22.)).flex_shrink_0().text_size(px(13.)).text_color(hsla(p.muted)).child(format!("{}.", index + 1)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(div().text_size(px(15.)).line_height(px(22.)).child(selectable(format!("{source}-{word}-entry-def-{group_index}-{index}-{def}"), def.to_owned(), live)))
                            .when_some(entry.def_example(group_index, index), |d, example| {
                                d.child(
                                    div()
                                        .pl(px(10.))
                                        .border_l_2()
                                        .border_color(hsla(p.accent))
                                        .text_size(px(13.))
                                        .italic()
                                        .text_color(hsla(p.muted))
                                        .child(selectable(format!("{source}-{word}-entry-example-{group_index}-{index}-{example}"), example.to_owned(), live)),
                                )
                            })
                            .when_some(credits.get(credit_index), |d, credit| {
                                let mut color = hsla(p.muted);
                                color.l = (color.l - 0.05).max(0.);
                                d.child(div().text_size(px(11.)).text_color(color)
                                    .child(selectable(format!("{source}-{word}-urban-credit-{credit_index}-{credit}"), credit.clone(), live)))
                            }),
                    ),
            );
            credit_index += 1;
        }
        column = column.child(section);
    }
    column.into_any_element()
}

impl Render for Dictionary {
    fn render(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = self.palette();
        let modal = self.modal_layer(window, cx);
        div()
            .id("root")
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(hsla(p.background))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(
                |this, event: &KeyDownEvent, window, cx| {
                    this.key(event, window, cx)
                },
            ))
            .on_mouse_move(|event, window, _| {
                if event.pressed_button == Some(gpui_kit::MouseButton::Left) {
                    window.refresh();
                }
            })
            .child(TextSelectionLayer)
            .child(self.page(true, window, cx))
            .children(modal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{Point, TestAppContext, VisualTestContext};

    #[test]
    fn pronunciation_hides_corrupt_text_and_placeholder_symbols() {
        for text in ["", "  ", "�", "/ə�/", "□", "a\u{fffc}", "a\0", "???"] {
            assert!(!valid_pronunciation(text), "{text:?}");
        }
        for text in ["/əˈdʌltəri/", "ă-dŭl′tĕr-ē", "/fʁɑ̃s/", "ə"] {
            assert!(valid_pronunciation(text), "{text:?}");
        }
    }

    #[test]
    fn alpha_survives_the_conversion() {
        let c = hsla(Rgba {
            r: 0x0e,
            g: 0x1e,
            b: 0x36,
            a: 0xe0,
        });
        assert!((c.a - 224.0 / 255.0).abs() < 0.01);
    }

    #[test]
    fn easing_runs_from_zero_to_one() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert!(ease_out_cubic(0.5) > 0.5);
    }

    #[test]
    fn failed_daily_refresh_keeps_cached_words() {
        let words = vec![DailyWord {
            word: "cached".into(), text: "Saved definition".into(),
            pos: None, phonetic: String::new(), example: None, author: None, date: None,
        }];
        let mut current = Some(Ok(words.clone()));
        update_daily_result(&mut current, Err("offline".into()));
        assert_eq!(current, Some(Ok(words)));
        update_daily_result(&mut current, Ok(Vec::new()));
        assert_eq!(current, Some(Ok(Vec::new())));
        let mut missing = None;
        update_daily_result(&mut missing, Err("offline".into()));
        assert_eq!(missing, Some(Err("offline".into())));
    }

    #[test]
    fn daily_cache_refreshes_after_one_hour() {
        assert!(daily_needs_refresh(false, None));
        assert!(!daily_needs_refresh(true, Some(Instant::now())));
        assert!(daily_needs_refresh(true, Some(Instant::now() - Duration::from_secs(3601))));
    }

    #[test]
    fn daily_entry_render_source_borrows_long_definition_fields() {
        let daily = DailyWord {
            word: "fixture".into(),
            phonetic: "/fɪkstʃɚ/".into(),
            text: "A long featured definition".repeat(20),
            pos: Some("noun".into()),
            example: Some("A featured example".into()),
            author: None,
            date: None,
        };
        let source = DailyEntry {
            daily: &daily,
            source: "Wiktionary",
        };
        assert_eq!(source.def_text(0, 0), daily.text);
        assert_eq!(source.def_text(0, 0).as_ptr(), daily.text.as_ptr());
        assert_eq!(source.def_example(0, 0), daily.example.as_deref());
        assert_eq!(source.group_pos(0), "noun");
    }

    #[test]
    fn source_visibility_handles_independent_completion_and_empty_sources() {
        let both_pending =
            source_visibility(true, false, false, true, true, false);
        assert!(both_pending.dictionary_loading);
        assert!(!both_pending.urban_loading);
        assert!(!both_pending.no_results);

        let urban_first =
            source_visibility(true, false, false, true, false, true);
        assert!(urban_first.dictionary_loading);
        assert!(urban_first.urban_results);
        assert!(!urban_first.no_results);

        let dictionary_first =
            source_visibility(false, true, false, true, true, false);
        assert!(dictionary_first.dictionary_results);
        assert!(dictionary_first.urban_loading);
        assert!(!dictionary_first.no_results);

        let urban_empty_while_dictionary_pending =
            source_visibility(true, false, false, true, false, false);
        assert!(urban_empty_while_dictionary_pending.dictionary_loading);
        assert!(!urban_empty_while_dictionary_pending.urban_loading);
        assert!(!urban_empty_while_dictionary_pending.urban_results);
        assert!(!urban_empty_while_dictionary_pending.no_results);

        let both_empty =
            source_visibility(false, false, true, true, false, false);
        assert!(both_empty.no_results);
        assert!(!both_empty.dictionary_results);
        assert!(!both_empty.urban_results);
    }

    fn open(
        cx: &mut TestAppContext,
    ) -> (
        Entity<Dictionary>,
        &mut VisualTestContext,
        tempfile::TempDir,
    ) {
        cx.update(gpui_omarchy::init);
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("cfg"));
        let (view, cx) = cx.add_window_view(move |window, cx| {
            Dictionary::new(store, None, None, None, window, cx)
        });
        cx.update(|window, cx| window.focus(&view.read(cx).focus.clone(), cx));
        draw(cx);
        (view, cx, dir)
    }

    fn draw(cx: &mut VisualTestContext) {
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
    }

    fn press(cx: &mut VisualTestContext, keys: &str) {
        cx.simulate_keystrokes(keys);
        cx.run_until_parked();
        draw(cx);
    }

    fn modal_kind(
        view: &Entity<Dictionary>,
        cx: &mut VisualTestContext,
    ) -> Option<(bool, bool)> {
        view.read_with(cx, |d, _| {
            d.modal
                .as_ref()
                .map(|m| (m.kind == Kind::Language, m.closing))
        })
    }

    #[gpui_kit::test]
    fn enter_looks_a_word_up_and_escape_clears_it(cx: &mut TestAppContext) {
        let (view, cx, _dir) = open(cx);
        cx.update(|window, cx| {
            let input = view.read(cx).input.clone();
            input.update(cx, |state, cx| {
                state.set_value("xanthic", window, cx);
                state.focus(window, cx);
            });
        });
        press(cx, "enter");
        view.read_with(cx, |d, _| match &d.view {
            View::Done(word, Outcome::Found(entry)) => {
                assert_eq!(word, "xanthic");
                assert_eq!(entry.source, "Webster's 1913");
            }
            _ => panic!("expected a definition"),
        });
        press(cx, "escape");
        assert!(
            view.read_with(cx, |d, cx| d.input.read(cx).value().is_empty())
        );
        press(cx, "escape");
        assert!(view.read_with(cx, |d, _| matches!(d.view, View::Idle)));
    }

    #[gpui_kit::test]
    fn similar_badge_repaints_on_hover_entry_and_exit(cx: &mut TestAppContext) {
        struct BadgeFixture(std::rc::Rc<std::cell::Cell<usize>>);

        impl Render for BadgeFixture {
            fn render(
                &mut self,
                _: &mut Window,
                _: &mut Context<Self>,
            ) -> impl IntoElement {
                self.0.set(self.0.get() + 1);
                div().size_full().child(
                    similar_word_badge(&Palette::default(), "related", true)
                        .id("badge")
                        .debug_selector(|| "hover-badge".into()),
                )
            }
        }

        let renders = std::rc::Rc::new(std::cell::Cell::new(0));
        let (_, cx) = cx.add_window_view({
            let renders = renders.clone();
            move |_, _| BadgeFixture(renders)
        });
        draw(cx);
        let bounds = cx.debug_bounds("hover-badge").unwrap();
        let outside = Point {
            x: bounds.right() + px(40.),
            y: bounds.bottom() + px(40.),
        };
        cx.simulate_mouse_move(outside, None, gpui_kit::Modifiers::none());
        let before = renders.get();
        cx.simulate_mouse_move(bounds.center(), None, gpui_kit::Modifiers::none());
        assert!(
            renders.get() > before,
            "hover entry must repaint immediately"
        );
        let entered = renders.get();
        cx.simulate_mouse_move(outside, None, gpui_kit::Modifiers::none());
        assert!(
            renders.get() > entered,
            "hover exit must repaint immediately"
        );
    }

    #[gpui_kit::test]
    fn similar_badge_replaces_query_and_searches(cx: &mut TestAppContext) {
        let (view, cx, _dir) = open(cx);
        view.update(cx, |d, cx| {
            d.urban_on = false;
            d.view = View::Done(
                "old".into(),
                Outcome::Found(Entry {
                    word: "old".into(),
                    phonetic: String::new(),
                    source: "fixture",
                    groups: vec![model::Group {
                        pos: "noun".into(),
                        defs: vec![model::Def {
                            text: "An old word.".into(),
                            example: None,
                        }],
                    }],
                }),
            );
            d.similar_words = vec!["xanthic".into()];
            cx.notify();
        });
        draw(cx);
        let bounds = cx
            .debug_bounds("similar-word-0")
            .expect("visible word badge");
        cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
        view.read_with(cx, |d, cx| {
            assert_eq!(d.input.read(cx).value().as_str(), "xanthic");
            assert!(matches!(&d.view, View::Loading(word) | View::Done(word, _) if word == "xanthic"));
            assert!(!d.similar_words.iter().any(|word| word == "xanthic"));
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |d, _| matches!(&d.view, View::Done(word, Outcome::Found(_)) if word == "xanthic")));
    }

    #[gpui_kit::test]
    fn clearing_search_cancels_similar_words(cx: &mut TestAppContext) {
        let (view, cx, _dir) = open(cx);
        cx.update(|window, cx| {
            view.update(cx, |d, cx| {
                d.urban_on = false;
                d.view = View::Done(
                    "happy".into(),
                    Outcome::Found(model::webster("happy").unwrap()),
                );
                d.request_similar(cx);
                let cancelled =
                    d.similar_cancel.clone().expect("pending recommendations");
                d.clear_search(window, cx);
                assert!(cancelled.load(Ordering::Relaxed));
                assert!(d.similar_words.is_empty());
                assert!(d.similar_cancel.is_none());
            })
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |d, _| d.similar_words.is_empty()
            && matches!(d.view, View::Idle)));
    }

    #[gpui_kit::test]
    fn clear_search_returns_home_and_invalidates_pending_results(cx: &mut TestAppContext) {
        let (view, cx, _dir) = open(cx);
        cx.update(|window, cx| {
            view.update(cx, |d, cx| {
                d.input.update(cx, |state, cx| state.set_value("pending", window, cx));
                d.view = View::Loading("pending".into());
                d.urban_pending = true;
                let previous_seq = d.seq;
                d.clear_search(window, cx);
                assert_eq!(d.seq, previous_seq + 1);
                assert!(matches!(d.view, View::Idle));
                assert!(!d.urban_pending);
                assert!(d.urban.is_none());
                assert!(d.input.read(cx).value().is_empty());
            });
        });
        draw(cx);
    }

    #[gpui_kit::test]
    fn the_language_panel_opens_chooses_with_the_keyboard_and_closes(
        cx: &mut TestAppContext,
    ) {
        let (view, cx, _dir) = open(cx);
        press(cx, "ctrl-l");
        assert_eq!(modal_kind(&view, cx), Some((true, false)));
        // English is highlighted; one down is French.
        press(cx, "down");
        press(cx, "enter");
        assert_eq!(
            modal_kind(&view, cx),
            Some((true, true)),
            "closing animates first"
        );
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
        assert_eq!(modal_kind(&view, cx), None);
        assert_eq!(view.read_with(cx, |d, _| d.lang), "fr");
    }

    #[gpui_kit::test]
    fn help_opens_with_f1_and_escape_closes_it(cx: &mut TestAppContext) {
        let (view, cx, _dir) = open(cx);
        press(cx, "f1");
        assert_eq!(modal_kind(&view, cx), Some((false, false)));
        press(cx, "escape");
        assert_eq!(modal_kind(&view, cx), Some((false, true)));
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
        assert_eq!(modal_kind(&view, cx), None);
    }

    #[gpui_kit::test]
    fn a_click_outside_the_card_dismisses_the_panel(cx: &mut TestAppContext) {
        let (view, cx, _dir) = open(cx);
        press(cx, "f1");
        cx.simulate_click(
            Point::new(px(4.), px(4.)),
            gpui_kit::Modifiers::none(),
        );
        draw(cx);
        assert_eq!(modal_kind(&view, cx), Some((false, true)));
    }

    struct ResultSelectionTest;

    impl Render for ResultSelectionTest {
        fn render(
            &mut self,
            _: &mut Window,
            _: &mut Context<Self>,
        ) -> impl IntoElement {
            let p = Theme::default().palette.shown();
            let entry = Entry {
                word: "primary".into(),
                phonetic: String::new(),
                source: "Wiktionary",
                groups: vec![model::Group {
                    pos: String::new(),
                    defs: vec![model::Def {
                        text: "primary definition".into(),
                        example: None,
                    }],
                }],
            };
            let urban = Some(Ok(vec![UrbanDef {
                word: "urban".into(),
                text: "urban definition".into(),
                example: Some("urban example".into()),
                pos: None,
                author: "author".into(),
                date: "2022-03-31".into(),
            }]));
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(TextSelectionLayer)
                .child(entry_view(&p, &entry, true))
                .child(urban_view(&p, "urban", urban.as_ref(), true))
        }
    }

    #[gpui_kit::test]
    fn result_text_participates_in_pointer_selection(cx: &mut TestAppContext) {
        cx.update(gpui_omarchy::init);
        let (_, cx) = cx.add_window_view(|_, _| ResultSelectionTest);
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.simulate_mouse_down(
            Point::new(px(1.), px(12.)),
            gpui_kit::MouseButton::Left,
            gpui_kit::Modifiers::none(),
        );
        cx.simulate_mouse_move(
            Point::new(px(400.), px(250.)),
            Some(gpui_kit::MouseButton::Left),
            gpui_kit::Modifiers::none(),
        );
        cx.update(|window, cx| {
            let _ = window.draw(cx);
            let text = gpui_kit::base::TextSelection::selected_text(window, cx);
            assert!(
                text.contains("urban definition"),
                "selection must update during drag: {text:?}"
            );
        });
        cx.simulate_mouse_up(
            Point::new(px(400.), px(250.)),
            gpui_kit::MouseButton::Left,
            gpui_kit::Modifiers::none(),
        );
        cx.update(|window, cx| {
            let _ = window.draw(cx);
            let text = gpui_kit::base::TextSelection::selected_text(window, cx);
            assert!(
                text.contains("primary definition"),
                "primary definition missing: {text:?}"
            );
            assert!(
                text.contains("urban definition"),
                "urban definition missing: {text:?}"
            );
            assert!(
                text.contains("urban example"),
                "urban example missing: {text:?}"
            );
        });
    }

    #[gpui_kit::test]
    fn caption_omits_parts_of_speech_and_keeps_lookup_status(
        cx: &mut TestAppContext,
    ) {
        let (view, cx, _dir) = open(cx);
        view.update(cx, |d, _| {
            d.view = View::Done(
                "test".into(),
                Outcome::Missing {
                    suggestions: vec![],
                },
            );
            assert_eq!(d.caption(), "Looking up…");
            d.urban = Some(Ok(vec![UrbanDef {
                word: "test".into(),
                text: "A person who tests.".into(),
                example: None,
                pos: Some("noun"),
                author: String::new(),
                date: String::new(),
            }]));
            assert!(d.caption().is_empty());
            d.urban = Some(Ok(vec![UrbanDef {
                word: "test".into(),
                text: "Unmarked slang description.".into(),
                example: None,
                pos: Some("adjective"),
                author: String::new(),
                date: String::new(),
            }]));
            assert!(d.caption().is_empty());
            d.urban_on = false;
            assert_eq!(d.caption(), "No definition");
            d.urban_on = true;
            d.urban = Some(Ok(vec![]));
            assert_eq!(d.caption(), "No definition");
            d.view = View::Done(
                "happy".into(),
                Outcome::Found(model::webster("happy").unwrap()),
            );
            assert!(d.caption().is_empty());
        });
    }

    #[gpui_kit::test]
    fn search_icon_pending_state_includes_enabled_urban_after_primary_finishes(
        cx: &mut TestAppContext,
    ) {
        let (view, cx, _dir) = open(cx);
        view.update(cx, |d, _| {
            d.view = View::Done(
                "word".into(),
                Outcome::Missing {
                    suggestions: vec![],
                },
            );
            d.urban_pending = true;
            assert!(d.is_search_pending());
            d.urban_on = false;
            assert!(!d.is_search_pending());
            d.urban_on = true;
            d.urban_pending = false;
            assert!(!d.is_search_pending());
        });
    }

    #[gpui_kit::test]
    fn every_screen_paints(cx: &mut TestAppContext) {
        let (view, cx, _dir) = open(cx);
        let entry = model::webster("xanthic").unwrap();
        let screens = [
            View::Idle,
            View::Loading("word".into()),
            View::Done("xanthic".into(), Outcome::Found(entry)),
            View::Done(
                "recieve".into(),
                Outcome::Missing {
                    suggestions: vec!["receive".into()],
                },
            ),
            View::Done(
                "zzz".into(),
                Outcome::Offline {
                    suggestions: vec![],
                },
            ),
        ];
        let def = UrbanDef {
            word: "x".into(),
            text: "t".into(),
            example: Some("e".into()),
            pos: None,
            author: "a".into(),
            date: "2022-03-31".into(),
        };
        let urbans = [
            None,
            Some(Ok(vec![def.clone(), def])),
            Some(Ok(vec![])),
            Some(Err("down".to_owned())),
        ];
        for screen in screens {
            view.update(cx, |d, cx| {
                d.view = screen;
                cx.notify();
            });
            for urban in &urbans {
                view.update(cx, |d, cx| {
                    d.urban = urban.clone();
                    d.urban_pending = urban.is_none()
                        && !matches!(d.view, View::Idle)
                        && d.urban_on;
                    cx.notify();
                });
                draw(cx);
                press(cx, "ctrl-l");
                press(cx, "escape");
                cx.executor().advance_clock(Duration::from_millis(200));
                cx.run_until_parked();
            }
        }
    }

    #[gpui_kit::test]
    fn daily_home_paints_fixture_loading_loaded_and_error_states_without_network(
        cx: &mut TestAppContext,
    ) {
        let (view, cx, _dir) = open(cx);
        assert!(!view.read_with(cx, |d, _| d.daily_wiktionary_pending
            || d.daily_urban_pending));
        draw(cx);
        view.update(cx, |d, cx| {
            d.daily_wiktionary_pending = true;
            d.daily_urban_pending = true;
            cx.notify();
        });
        draw(cx);
        view.update(cx, |d, cx| {
            let word = DailyWord {
                word: "luminous".into(),
                phonetic: "/ˈluːmɪnəs/".into(),
                text: "Giving off light.".into(),
                pos: Some("adjective".into()),
                example: Some("A luminous star.".into()),
                author: None,
                date: Some("2026-11-12".into()),
            };
            d.daily_wiktionary = Some(Ok(vec![word.clone(), DailyWord { word: "radiant".into(), ..word.clone() }]));
            d.daily_wiktionary_pending = false;
            d.daily_urban = Some(Ok(vec![DailyWord {
                author: Some("example author".into()),
                ..word.clone()
            }, DailyWord { word: "glowing".into(), ..word }]));
            cx.notify();
        });
        draw(cx);
        view.update(cx, |d, cx| {
            d.daily_urban_pending = false;
            d.daily_wiktionary = Some(Err("fixture failure".into()));
            d.daily_urban = Some(Err("fixture failure".into()));
            cx.notify();
        });
        draw(cx);
        view.update(cx, |d, cx| {
            d.urban_on = false;
            cx.notify();
        });
        draw(cx);
    }

    #[gpui_kit::test]
    fn daily_selection_uses_english_and_survives_returning_home(cx: &mut TestAppContext) {
        let (view, cx, _dir) = open(cx);
        view.update(cx, |d, cx| {
            d.lang = "fr";
            d.urban_on = false;
            d.daily_wiktionary = Some(Ok(vec![DailyWord {
                word: "xanthic".into(), phonetic: String::new(), text: "Yellow.".into(), pos: None,
                example: None, author: None, date: None,
            }]));
            cx.notify();
        });
        press(cx, "ctrl-1");
        assert!(view.read_with(cx, |d, _| d.lang == "en"
            && matches!(d.view, View::Done(_, Outcome::Found(_)))));
        press(cx, "escape");
        press(cx, "escape");
        assert!(view.read_with(cx, |d, _| matches!(d.view, View::Idle)
            && d.daily_wiktionary.as_ref().is_some_and(|result|
                result.as_ref().is_ok_and(|words| words[0].word == "xanthic"))));
    }

    #[gpui_kit::test]
    fn hidden_daily_urban_does_not_start_a_request_in_tests(
        cx: &mut TestAppContext,
    ) {
        let (view, cx, _dir) = open(cx);
        press(cx, "ctrl-u");
        assert!(
            !view.read_with(cx, |d, _| d.urban_on || d.daily_urban_pending)
        );
        press(cx, "ctrl-u");
        assert!(view.read_with(cx, |d, _| d.urban_on));
        assert!(!view.read_with(cx, |d, _| d.daily_urban_pending));
    }

    #[gpui_kit::test]
    fn ctrl_u_toggles_urban_even_with_the_field_focused(
        cx: &mut TestAppContext,
    ) {
        let (view, cx, _dir) = open(cx);
        cx.update(|window, cx| {
            let input = view.read(cx).input.clone();
            input.update(cx, |state, cx| {
                state.set_value("rizz", window, cx);
                state.focus(window, cx);
            });
        });
        press(cx, "ctrl-u");
        assert!(!view.read_with(cx, |d, _| d.urban_on));
        assert_eq!(
            view.read_with(cx, |d, cx| d.input.read(cx).value().to_string()),
            "rizz"
        );
        press(cx, "ctrl-u");
        assert!(view.read_with(cx, |d, _| d.urban_on));
    }

    #[gpui_kit::test]
    fn a_saved_theme_change_is_applied_live(cx: &mut TestAppContext) {
        let (view, cx, dir) = open(cx);
        let store = Store::new(dir.path().join("cfg"));
        let mut theme = Theme::default();
        theme.palette.background = Rgba::parse("#e00e1e36").unwrap();
        store.save(&theme).unwrap();
        for _ in 0..8 {
            cx.executor().advance_clock(Duration::from_millis(300));
            cx.run_until_parked();
        }
        assert_eq!(
            view.read_with(cx, |d, _| d.theme.palette.background.to_saved()),
            "#e00e1e36"
        );
    }
}
