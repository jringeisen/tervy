//! Native reminder regression using simulated activity, without recording audio.
use super::*;

impl Workspace {
    pub(super) fn smoke_teams(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.restoring = false;
        self.setup_open = false;
        self.readiness = Some(crate::setup::Readiness {
            shell: true,
            agents: vec![],
            meetings_supported: true,
            warning: None,
        });
        // An open dialog defers the reminder, rather than losing or covering it.
        self.note_modal_open = true;
        for _ in 0..3 {
            self.teams_activity(Some(true), window, cx);
        }
        assert!(!self.teams_prompt_open);
        self.note_modal_open = false;
        self.teams_activity(Some(true), window, cx);
        assert!(self.teams_prompt_open);
        let weak = cx.weak_entity();
        window.on_next_frame(move |window, _cx| {
            window.resize(size(px(560.), px(480.)));
            window.on_next_frame(move |window, cx| {
                window.dispatch_keystroke(Keystroke::parse("escape").unwrap(), cx);
                let _ = weak.update(cx, |this, cx| {
                    assert!(!this.teams_prompt_open);
                    for _ in 0..5 { this.teams_activity(Some(true), window, cx); }
                    assert!(!this.teams_prompt_open, "Escape suppresses the call");
                    for _ in 0..10 { this.teams_activity(Some(false), window, cx); }
                    // Existing manual recordings suppress reminders for this call.
                    let meeting = meetings::Meeting::new(std::env::temp_dir());
                    this.meetings.push(meeting);
                    for _ in 0..3 { this.teams_activity(Some(true), window, cx); }
                    assert!(!this.teams_prompt_open);
                    this.meetings.clear();
                    this.teams_activity(Some(true), window, cx);
                    assert!(!this.teams_prompt_open);
                    for _ in 0..10 { this.teams_activity(Some(false), window, cx); }
                    this.teams_reminders_disabled = true;
                    for _ in 0..3 { this.teams_activity(Some(true), window, cx); }
                    assert!(!this.teams_prompt_open);
                    this.teams_reminders_disabled = false;
                    for _ in 0..3 { this.teams_activity(Some(true), window, cx); }
                    assert!(this.teams_prompt_open);
                });
                window.on_next_frame(move |_, cx| {
                    eprintln!("tervy-teams-smoke PASS dialog_deferral=true escape_suppression=true existing_recording=true disable_enable=true narrow_window=true");
                    if std::env::var_os("TERVY_TEAMS_PREVIEW").is_none() { cx.quit(); }
                });
            });
        });
        cx.notify();
    }
}
