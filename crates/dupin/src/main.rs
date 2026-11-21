//! Dupin: a resident launcher panel. ⌘Space (by default) shows it; type to find an app, or
//! press Space first to search files with Spotlight.

mod hotkey;
mod log;
mod settings;
mod ui;
mod view;

use rune::{App, AppSpec, WindowOptions, WindowSpec};

/// The panel's width; its height follows its content.
pub const PANEL_WIDTH: f32 = 680.0;
/// The panel with nothing below the field: the window's first size, before it fits its content.
pub const HEADER_HEIGHT: f32 = view::HEADER_H;

fn main() {
    rune::prewarm_gpu();
    let (config, config_problem) = settings::load();

    let spec = AppSpec {
        setup: Some(ui::Ui::setup),
        ..AppSpec::new(
            "Dupin",
            (PANEL_WIDTH, HEADER_HEIGHT),
            || unreachable!(),
            view::view,
        )
    };
    // Blend in sRGB as Core Animation does, so the translucent frost over the glass reads as a
    // native layer would.
    let app = App::new(spec)
        .windowless()
        .accessory()
        .fit_content_height()
        .srgb_blend(true);
    let handle = app.handle();
    let hotkey = config.hotkey.clone();
    let mut registered = false;
    let app = app
        .init({
            let handle = handle.clone();
            move || ui::Ui::new(handle, config, config_problem)
        })
        .on_ready(move |_| {
            // Once per process: a launcher window is made once and kept, but be explicit.
            if !std::mem::replace(&mut registered, true) {
                hotkey::register(&handle, &hotkey);
            }
        })
        .on_focus(|s, focused, tx| {
            if !focused {
                s.dismiss(tx);
            }
        });

    rune::run_windows(move |_| {
        // AppKit rounds the panel and its shadow; the view also clips, for headless renders.
        let options = WindowOptions::launcher_panel().macos(|native| native.corner_radius(view::RADIUS));
        let panel: rune::AnyWindowSpec = WindowSpec::from_app("panel", app)
            .configured_options(options)
            .into();
        [panel]
    });
}
