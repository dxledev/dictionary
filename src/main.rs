//! Dictionary: a portable, themeable dictionary.

mod cli;
mod color;
mod daily;
mod daily_cache;
mod hover_lift;
mod model;
mod similar;
mod theme;
mod urban;
mod urban_pos;
mod ui;

use gpui_kit::{AppContext as _, WindowOptions, px, size};

/// The Wayland app id and the desktop entry's `StartupWMClass`.
pub const APP_ID: &str = "io.github.dictionary";

fn main() {
    let (store, word, lang, selection, panel) = match cli::parse(std::env::args().skip(1).collect()) {
        cli::Action::Done { code } => std::process::exit(code),
        cli::Action::Launch { store, word, lang, selection, panel } => (store, word, lang, selection, panel),
    };
    let word = if selection { selected_text().or(word) } else { word };
    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(move |cx| {
        gpui_omarchy::init(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let window = cx
            .open_window(
                WindowOptions {
                    titlebar: Some(gpui_kit::TitlebarOptions {
                        title: Some("Dictionary".into()),
                        ..Default::default()
                    }),
                    app_id: Some(APP_ID.to_owned()),
                    window_min_size: Some(size(px(520.), px(420.))),
                    window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds::new(
                        gpui_kit::point(px(120.), px(90.)),
                        size(px(760.), px(640.)),
                    ))),
                    window_background: gpui_kit::WindowBackgroundAppearance::Transparent,
                    ..Default::default()
                },
                move |window, cx| cx.new(|cx| ui::Dictionary::new(store, word, lang, panel, window, cx)),
            )
            .expect("open the Dictionary window");
        let _ = window.update(cx, |this, window, cx| this.focus_input(window, cx));
        cx.activate(true);
    });
}

/// The primary selection, for a launcher or key binding that looks up
/// whatever is highlighted.
fn selected_text() -> Option<String> {
    use std::process::Command;
    let attempts: [(&str, &[&str]); 3] = [
        ("wl-paste", &["--primary", "--no-newline"]),
        ("xclip", &["-o", "-selection", "primary"]),
        ("xsel", &["-o", "-p"]),
    ];
    attempts.iter().find_map(|(program, args)| {
        let output = Command::new(program).args(*args).output().ok().filter(|o| o.status.success())?;
        let text = String::from_utf8(output.stdout).ok()?;
        let text = text.trim();
        (!text.is_empty() && text.chars().count() < 80).then(|| text.to_owned())
    })
}
