//! Native views and direct input; all session lifecycle belongs to the core.
mod badge;
mod composition;
mod logging;
mod m2_perf;
mod microphone;
mod panic_reporter;
mod perf;
mod setup;
mod terminal_element;
mod terminal_perf;
mod terminal_view;
mod theme;
mod updater;
mod workspace;
use crate::workspace::*;
use gpui_kit::*;
use std::borrow::Cow;
use tervy_core::{config, instance::InstanceGuard, supervisor::Supervisor};
gpui_kit::actions!(tervy, [Quit]);
struct Desktop {
    workspace: Entity<Workspace>,
    window: AnyWindowHandle,
}
impl Global for Desktop {}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.get(1).map(String::as_str) == Some("hook") {
        tervy_core::hook::run(
            arguments.get(2).map(String::as_str),
            arguments.last().map(String::as_str),
        );
        return;
    }
    if arguments.get(1).map(String::as_str) == Some("mcp") {
        tervy_core::mcp::run_stdio();
        return;
    }
    let perf = perf::StartupTimer::from_env();
    if std::env::var_os("TERVY_PERF_LAYOUT").is_none()
        && let Err(error) = config::migrate_legacy_state()
    {
        eprintln!("Could not start Tervy: {error}");
        return;
    }
    logging::init();
    let panics = panic_reporter::PanicReporter::install();
    if arguments.get(1).map(String::as_str) == Some("--panic-probe") {
        // Explicit diagnostic command: isolated configuration, no native window.
        // Never selected through normal app startup or hook invocations.
        let path = std::path::PathBuf::from(arguments.get(2).expect("probe config path"));
        let runtime = Supervisor::start(path.clone(), None).expect("probe supervisor");
        panics.connect(runtime.handle.clone());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if let Ok(event) = runtime.events.try_recv() {
                match event {
                    tervy_core::supervisor::CoreEvent::SessionChanged(info)
                        if info.process.is_some()
                            && info.status == tervy_core::supervisor::SessionStatus::Stopped =>
                    {
                        runtime
                            .handle
                            .send(tervy_core::supervisor::Command::Start(info.id))
                    }
                    tervy_core::supervisor::CoreEvent::SessionStarted { info, .. } => {
                        let owned =
                            tervy_term::kill::OwnedSession::capture(info.pid.expect("probe pid"))
                                .expect("probe identity");
                        eprintln!(
                            "panic-probe sid={} started_us={}",
                            owned.leader.pid, owned.leader.started_us
                        );
                        while !path.parent().unwrap().join("ready").exists() {
                            assert!(
                                std::time::Instant::now() < deadline,
                                "probe child ready timeout"
                            );
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                        panic!("PRIVATE_PROBE_PAYLOAD");
                    }
                    _ => {}
                }
            }
            assert!(std::time::Instant::now() < deadline, "probe timed out");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Tervy starting");
    let path = config::support_directory();
    // Benchmark windows use an in-memory project and do not contend with the
    // owner's instance or modify their configuration.
    let benchmark = std::env::var_os("TERVY_PERF_LAYOUT").is_some();
    let instance = if benchmark {
        None
    } else {
        match InstanceGuard::acquire(&path) {
            Ok(Some(instance)) => Some(instance),
            Ok(None) => return,
            Err(error) => {
                eprintln!("Could not start Tervy: {error}");
                return;
            }
        }
    };
    let scenario =
        benchmark.then(|| std::env::var("TERVY_PERF_SCENARIO").unwrap_or_else(|_| "shell".into()));
    let runtime = match Supervisor::start(path.join("config.toml"), scenario) {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Could not start supervisor: {error}");
            return;
        }
    };
    panics.connect(runtime.handle.clone());
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
        if let Err(error) = cx.text_system().add_fonts(vec![
            Cow::Borrowed(
                include_bytes!("../../../assets/fonts/IBMPlexSans-Regular.ttf").as_slice(),
            ),
            Cow::Borrowed(
                include_bytes!("../../../assets/fonts/IBMPlexSans-SemiBold.ttf").as_slice(),
            ),
            Cow::Borrowed(
                include_bytes!("../../../assets/fonts/IBMPlexMono-Regular.ttf").as_slice(),
            ),
        ]) {
            tracing::warn!(%error, "Could not load IBM Plex fonts");
        }
        install_menus_and_keys(cx);
        if std::env::var_os("TERVY_TRACE_TAB").is_some() {
            cx.observe_keystrokes(|event, _, _| {
                if event.keystroke.key == "tab" {
                    eprintln!(
                        "tervy-tab resolved action={:?} contexts={:?}",
                        event.action.as_ref().map(|a| a.name()),
                        event.context_stack
                    );
                }
            })
            .detach();
        }
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1200.), px(780.)), cx)),
            window_min_size: Some(size(px(560.), px(480.))),
            ..Default::default()
        };
        let result = gpui_kit::open_window(options, cx, |window, cx| {
            theme::sync_appearance(window, cx);
            window
                .observe_window_appearance(theme::sync_appearance)
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
                cx.set_global(Desktop {
                    workspace: workspace.clone(),
                    window,
                });
                let (update_sender, update_events) = async_channel::unbounded();
                if !benchmark && updater::start(update_sender) {
                    cx.spawn(async move |cx| {
                        while let Ok(event) = update_events.recv().await {
                            let _ = window.update(cx, |_, window, cx| {
                                workspace.update(cx, |workspace, cx| {
                                    workspace.update_event(event, window, cx)
                                })
                            });
                        }
                    })
                    .detach();
                }
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
            // Keyboard/menu dispatch already holds the window lease. Wait
            // until it is released before entering the confirmation flow.
            cx.defer(move |cx| {
                if let Err(error) = window.update(cx, |_, window, cx| {
                    view.update(cx, |view, cx| view.request_quit(window, cx))
                }) {
                    tracing::error!(%error, "failed to request quit");
                }
            });
        }
    });
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-p", Palette, Some("Workspace")),
        KeyBinding::new("cmd-k", Palette, Some("Workspace")),
        KeyBinding::new("cmd-b", ToggleSidebar, Some("Workspace")),
        KeyBinding::new("cmd-t", NewTerminal, Some("Workspace")),
        KeyBinding::new("cmd-n", NewNote, Some("Workspace")),
        KeyBinding::new("cmd-w", CloseTerminal, Some("Workspace")),
        KeyBinding::new("cmd-shift-r", RenameTerminal, Some("Workspace")),
        KeyBinding::new("cmd-shift-a", AskAgentQuestion, Some("Workspace")),
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
    cx.bind_keys(terminal_view::key_bindings());
    cx.bind_keys(sidebar_key_bindings());
    cx.set_menus([
        Menu::new("Tervy").items([MenuItem::action("Quit Tervy", Quit)]),
        Menu::new("File").items([
            MenuItem::action("Add Project…", AddProject),
            MenuItem::action("New Terminal", NewTerminal),
            MenuItem::action("New Note", NewNote),
            MenuItem::action("Close Terminal", CloseTerminal),
            MenuItem::action("Rename Terminal…", RenameTerminal),
        ]),
        Menu::new("View").items([MenuItem::action("Command Palette", Palette)]),
    ]);
}
