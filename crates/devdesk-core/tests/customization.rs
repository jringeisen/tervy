use devdesk_core::{
    customization::Draft,
    supervisor::{Command, CoreEvent, Supervisor},
};
use std::time::{Duration, Instant};

fn next(runtime: &Supervisor, deadline: Instant) -> CoreEvent {
    loop {
        if let Ok(event) = runtime.events.try_recv() {
            return event;
        }
        assert!(
            Instant::now() < deadline,
            "customization supervisor timed out"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn reviewed_proposal_updates_views_model_and_opens_a_preset_then_undoes() {
    let directory = std::env::temp_dir().join(format!(
        "devdesk-customization-integration-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(directory.join("nested")).unwrap();
    let path = directory.join("config.toml");
    let original = format!(
        "# preserve me\n[[projects]]\nid='test'\nname='Test'\npath='{}'\n",
        directory.display()
    );
    std::fs::write(&path, &original).unwrap();
    let runtime = Supervisor::start(path.clone(), None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !matches!(next(&runtime, deadline), CoreEvent::Config(_)) {}
    let (cwd, _) = Draft::prepare(&path, "Warm theme and nested terminal").unwrap();
    let proposal = format!(
        "[settings]\ntheme='warm-dark'\nfont_size=18\nsidebar_width=240\n{original}\n[[projects.terminals]]\nname='Nested'\ncwd='nested'\n"
    );
    std::fs::write(cwd.join("proposal.toml"), &proposal).unwrap();
    runtime.handle.send(Command::ReviewCustomization);
    loop {
        match next(&runtime, deadline) {
            CoreEvent::CustomizationPreview(preview) => {
                assert_eq!(preview.proposal, proposal);
                assert!(preview.changes.contains("+ theme='warm-dark'"));
                break;
            }
            CoreEvent::ConfigError(error) => panic!("{error}"),
            _ => {}
        }
    }
    runtime.handle.send(Command::ApplyCustomization(proposal));
    let mut configured = false;
    loop {
        match next(&runtime, deadline) {
            CoreEvent::Config(config) => {
                assert_eq!(config.settings.theme, "warm-dark");
                assert_eq!(config.settings.font_size, 18.);
                assert_eq!(
                    config.projects[0].terminals[0].cwd.as_ref().unwrap(),
                    &directory.join("nested")
                );
                configured = true;
            }
            CoreEvent::CustomizationApplied { undone: false } => {
                assert!(configured);
                break;
            }
            CoreEvent::ConfigError(error) => panic!("{error}"),
            _ => {}
        }
    }
    runtime.handle.send(Command::NewTerminalPreset {
        project: "test".into(),
        name: "Nested".into(),
    });
    loop {
        match next(&runtime, deadline) {
            CoreEvent::SessionStarted { info, .. } => {
                assert_eq!(info.name, "Nested");
                break;
            }
            CoreEvent::ConfigError(error) => panic!("{error}"),
            _ => {}
        }
    }
    runtime.handle.send(Command::UndoCustomization);
    loop {
        match next(&runtime, deadline) {
            CoreEvent::CustomizationApplied { undone: true } => break,
            CoreEvent::ConfigError(error) => panic!("{error}"),
            _ => {}
        }
    }
    assert_eq!(std::fs::read_to_string(path).unwrap(), original);
    runtime.handle.send(Command::Shutdown);
    loop {
        if let CoreEvent::ShutdownComplete { clean } = next(&runtime, deadline) {
            assert!(clean);
            break;
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}
