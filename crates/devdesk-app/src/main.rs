//! DevDesk application binary: the GPUI window and views.
//!
//! Lifecycle logic never lives here (architecture.md §2): views send commands
//! to `devdesk-core` and render the state it reports back.

mod logging;
mod perf;
mod workspace;

use gpui_kit::component::Theme;
use gpui_kit::*;

use crate::workspace::Workspace;

gpui_kit::actions!(devdesk, [Quit]);

fn main() {
    let perf = perf::StartupTimer::from_env();
    logging::init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "DevDesk starting");

    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);

    // Closing the window keeps DevDesk running (ADR-018); clicking the Dock
    // icon brings a window back.
    app.on_reopen(|cx| {
        if cx.windows().is_empty() {
            open_main_window(None, cx);
        }
    });

    app.run(move |cx| {
        gpui_kit::init(cx);
        install_menus_and_keys(cx);
        open_main_window(Some(perf), cx);
        cx.activate(true);
    });
}

fn install_menus_and_keys(cx: &mut App) {
    // Cmd-Q quits immediately for now. M2 replaces this with the
    // "N sessions running — stop all and quit?" confirmation (ADR-018).
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
    cx.set_menus([Menu::new("DevDesk").items([MenuItem::action("Quit DevDesk", Quit)])]);
}

fn open_main_window(perf: Option<perf::StartupTimer>, cx: &mut App) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(1200.), px(780.)), cx)),
        ..Default::default()
    };

    let result = gpui_kit::open_window(options, cx, |window, cx| {
        // Follow the macOS light/dark setting, now and when it changes.
        Theme::sync_system_appearance(Some(window), cx);
        window
            .observe_window_appearance(|window, cx| {
                Theme::sync_system_appearance(Some(window), cx);
            })
            .detach();

        if let Some(perf) = perf {
            // Runs right after the first frame has been rendered.
            window.on_next_frame(move |_, cx| perf.first_frame_rendered(cx));
        }

        cx.new(|_| Workspace::new())
    });

    if let Err(error) = result {
        tracing::error!(%error, "failed to open the main window");
    }
}
