//! Native views and direct input; all session lifecycle belongs to the core.
mod composition;
mod logging;
mod m2_perf;
mod perf;
mod terminal_element;
mod terminal_perf;
mod terminal_view;
mod workspace;
use crate::workspace::*;
use devdesk_core::{config, instance::InstanceGuard, supervisor::Supervisor};
use gpui_kit::component::Theme;
use gpui_kit::*;
gpui_kit::actions!(devdesk, [Quit]);
struct Desktop {
    workspace: Entity<Workspace>,
    window: AnyWindowHandle,
}
impl Global for Desktop {}

fn main() {
    let perf = perf::StartupTimer::from_env();
    logging::init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "DevDesk starting");
    let path = config::support_directory();
    // Benchmark windows use an in-memory project and do not contend with the
    // owner's instance or modify their configuration.
    let benchmark = std::env::var_os("DEVDESK_PERF_LAYOUT").is_some();
    let instance = if benchmark {
        None
    } else {
        match InstanceGuard::acquire(&path) {
            Ok(Some(instance)) => Some(instance),
            Ok(None) => return,
            Err(error) => {
                eprintln!("Could not start DevDesk: {error}");
                return;
            }
        }
    };
    let scenario = benchmark
        .then(|| std::env::var("DEVDESK_PERF_SCENARIO").unwrap_or_else(|_| "shell".into()));
    let runtime = match Supervisor::start(path.join("config.toml"), scenario) {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Could not start supervisor: {error}");
            return;
        }
    };
    let core = runtime.handle.clone();
    let events = runtime.events.clone();
    let activation = instance.as_ref().map(|(_, receiver)| receiver.clone());
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);
    app.on_reopen(|cx| {
        if let Some(desktop) = cx.try_global::<Desktop>() {
            let window = desktop.window;
            let _ = window.update(cx, |_, window, cx| {
                window.activate_window();
                cx.activate(true);
            });
        }
    });
    app.run(move |cx| {
        gpui_kit::init(cx);
        install_menus_and_keys(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1200.), px(780.)), cx)),
            ..Default::default()
        };
        let result = gpui_kit::open_window(options, cx, |window, cx| {
            Theme::sync_system_appearance(Some(window), cx);
            window
                .observe_window_appearance(|window, cx| {
                    Theme::sync_system_appearance(Some(window), cx)
                })
                .detach();
            window.on_window_should_close(cx, |_, cx| {
                cx.hide();
                false
            });
            window.on_next_frame(move |_, cx| perf.first_frame_rendered(cx));
            cx.new(|cx| Workspace::new(window, cx, core, events))
        });
        match result {
            Ok((window, workspace)) => {
                cx.set_global(Desktop { workspace, window });
                if let Some(activation) = activation {
                    cx.spawn(async move |cx| {
                        while activation.recv().await.is_ok() {
                            let _ = window.update(cx, |_, window, cx| {
                                window.activate_window();
                                cx.activate(true);
                            });
                        }
                    })
                    .detach();
                }
            }
            Err(error) => {
                tracing::error!(%error, "failed to open the main window");
                cx.quit();
            }
        }
        cx.activate(true);
    });
    drop(runtime);
    drop(instance);
}
fn install_menus_and_keys(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| {
        if let Some(desktop) = cx.try_global::<Desktop>() {
            let view = desktop.workspace.clone();
            let window = desktop.window;
            let _ = window.update(cx, |_, window, cx| {
                view.update(cx, |view, cx| view.request_quit(window, cx))
            });
        }
    });
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-p", Palette, Some("Workspace")),
        KeyBinding::new("cmd-t", NewTerminal, Some("Workspace")),
        KeyBinding::new("cmd-w", CloseTerminal, Some("Workspace")),
        KeyBinding::new("cmd-shift-r", RenameTerminal, Some("Workspace")),
        KeyBinding::new("cmd-o", AddProject, Some("Workspace")),
        KeyBinding::new("cmd-shift-]", NextTab, Some("Workspace")),
        KeyBinding::new("cmd-shift-[", PreviousTab, Some("Workspace")),
        KeyBinding::new("cmd-1", Tab1, Some("Workspace")),
        KeyBinding::new("cmd-2", Tab2, Some("Workspace")),
        KeyBinding::new("cmd-3", Tab3, Some("Workspace")),
        KeyBinding::new("cmd-4", Tab4, Some("Workspace")),
        KeyBinding::new("cmd-5", Tab5, Some("Workspace")),
        KeyBinding::new("cmd-6", Tab6, Some("Workspace")),
        KeyBinding::new("cmd-7", Tab7, Some("Workspace")),
        KeyBinding::new("cmd-8", Tab8, Some("Workspace")),
        KeyBinding::new("cmd-9", Tab9, Some("Workspace")),
    ]);
    cx.set_menus([
        Menu::new("DevDesk").items([MenuItem::action("Quit DevDesk", Quit)]),
        Menu::new("File").items([
            MenuItem::action("Add Project…", AddProject),
            MenuItem::action("New Terminal", NewTerminal),
            MenuItem::action("Close Terminal", CloseTerminal),
            MenuItem::action("Rename Terminal…", RenameTerminal),
        ]),
        Menu::new("View").items([MenuItem::action("Command Palette", Palette)]),
    ]);
}
