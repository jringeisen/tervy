//! Native views and direct input; all session lifecycle belongs to the core.
mod badge;
mod composition;
mod logging;
mod m2_perf;
mod panic_reporter;
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
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.get(1).map(String::as_str) == Some("hook") {
        devdesk_core::hook::run(
            arguments.get(2).map(String::as_str),
            arguments.last().map(String::as_str),
        );
        return;
    }
    let perf = perf::StartupTimer::from_env();
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
                    devdesk_core::supervisor::CoreEvent::SessionChanged(info)
                        if info.process.is_some()
                            && info.status == devdesk_core::supervisor::SessionStatus::Stopped =>
                    {
                        runtime
                            .handle
                            .send(devdesk_core::supervisor::Command::Start(info.id))
                    }
                    devdesk_core::supervisor::CoreEvent::SessionStarted { info, .. } => {
                        let owned =
                            devdesk_term::kill::OwnedSession::capture(info.pid.expect("probe pid"))
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
        install_menus_and_keys(cx);
        if std::env::var_os("DEVDESK_TRACE_TAB").is_some() {
            cx.observe_keystrokes(|event, _, _| {
                if event.keystroke.key == "tab" {
                    eprintln!(
                        "devdesk-tab resolved action={:?} contexts={:?}",
                        event.action.as_ref().map(|a| a.name()),
                        event.context_stack
                    );
                }
            })
            .detach();
        }
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
