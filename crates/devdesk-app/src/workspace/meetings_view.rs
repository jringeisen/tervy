use super::*;
use gpui_kit::base::Disableable;
use gpui_kit::component::text::TextView;

impl Workspace {
    pub(super) fn open_meetings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.meetings_open = true;
        self.meetings_expanded = true;
        self.prompts_open = false;
        self.palette_open = false;
        self.core.send(CoreCommand::ListMeetings);
        if self.meeting_selected.is_none()
            && let Some(id) = self.meetings.first().map(|meeting| meeting.id.clone())
        {
            self.select_meeting(id, window, cx);
        }
        self.update_visibility(window, cx);
        cx.notify();
    }

    pub(super) fn select_meeting(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(meeting) = self
            .meetings
            .iter()
            .find(|meeting| meeting.id == id)
            .cloned()
        else {
            return;
        };
        self.meeting_selected = Some(id);
        self.meeting_notes_editing = false;
        self.meeting_title_input
            .update(cx, |input, cx| input.set_value(meeting.title, window, cx));
        self.meeting_notes_input
            .update(cx, |input, cx| input.set_value(meeting.notes, window, cx));
        self.meeting_error = None;
        cx.notify();
    }

    pub(super) fn active_recording(&self) -> Option<&meetings::Meeting> {
        self.meetings.iter().find(|meeting| {
            matches!(
                meeting.status,
                meetings::Status::Starting
                    | meetings::Status::Recording
                    | meetings::Status::Stopping
            )
        })
    }

    pub(super) fn start_meeting_timer(&mut self, cx: &mut Context<Self>) {
        if self.meeting_timer_running {
            return;
        }
        self.meeting_timer_running = true;
        self._tasks.push(cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                let active = view
                    .update(cx, |this, cx| {
                        let active = this.active_recording().is_some();
                        if !active {
                            this.meeting_timer_running = false;
                        }
                        cx.notify();
                        active
                    })
                    .unwrap_or(false);
                if !active {
                    break;
                }
            }
        }));
    }

    pub(super) fn record_meeting(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_recording().is_some() {
            return;
        }
        self.meeting_error = None;
        self.open_meetings(window, cx);
        self.core.send(CoreCommand::StartMeeting);
    }

    pub(super) fn stop_meeting(&mut self, cx: &mut Context<Self>) {
        self.core.send(CoreCommand::StopMeeting);
        cx.notify();
    }

    pub(super) fn save_meeting_edit(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.meeting_selected.clone() else {
            return;
        };
        let Some(meeting) = self.meetings.iter().find(|meeting| meeting.id == id) else {
            return;
        };
        let title = self.meeting_title_input.read(cx).value().to_string();
        let notes = self.meeting_notes_input.read(cx).value().to_string();
        if title.trim().is_empty() || title.len() > 160 {
            self.meeting_error = Some("Meeting title must be 1–160 bytes".into());
            cx.notify();
            return;
        }
        if notes.len() > 512 * 1024 {
            self.meeting_error = Some("Notes exceed 512 KB".into());
            cx.notify();
            return;
        }
        if title != meeting.title {
            self.core.send(CoreCommand::RenameMeeting {
                id: id.clone(),
                title,
            });
        }
        if notes != meeting.notes {
            self.core.send(CoreCommand::SaveMeetingNotes { id, notes });
        }
        self.meeting_notes_editing = false;
        self.meeting_error = None;
        cx.notify();
    }

    pub(super) fn delete_meeting_audio(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.meeting_selected.clone() else {
            return;
        };
        if self.prompt_pending {
            return;
        }
        self.prompt_pending = true;
        let answer = window.prompt(PromptLevel::Warning, "Delete this meeting's audio?", Some("The transcript and notes will stay. You won't be able to replay or retranscribe the recording."), &["Cancel", "Delete audio"], cx);
        self._tasks.push(cx.spawn(async move |view, cx| {
            let approved = answer.await == Ok(1);
            let _ = view.update(cx, |this, cx| {
                this.prompt_pending = false;
                if approved {
                    this.core.send(CoreCommand::DeleteMeetingAudio(id));
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn delete_meeting(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.meeting_selected.clone() else {
            return;
        };
        if self.prompt_pending {
            return;
        }
        self.prompt_pending = true;
        let answer = window.prompt(
            PromptLevel::Warning,
            "Delete this meeting?",
            Some("The recording, transcript, and notes will be removed."),
            &["Cancel", "Delete meeting"],
            cx,
        );
        self._tasks.push(cx.spawn(async move |view, cx| {
            let approved = answer.await == Ok(1);
            let _ = view.update(cx, |this, cx| {
                this.prompt_pending = false;
                if approved {
                    this.core.send(CoreCommand::DeleteMeeting(id));
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn meetings_content(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let selected = self
            .meeting_selected
            .as_ref()
            .and_then(|id| self.meetings.iter().find(|meeting| &meeting.id == id))
            .cloned();
        let mut panel = v_flex()
            .id("meetings-content-scroll")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .p_5()
            .gap_4()
            .child(div().text_lg().font_semibold().child("Meetings"))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Records microphone and Mac audio. Transcription runs on your Mac; your signed-in AI provider generates notes."),
            );
        if let Some(error) = &self.meeting_error {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(theme.danger)
                    .child(error.clone()),
            );
        }
        if self.meetings.is_empty() {
            panel = panel.child(
                div()
                    .text_sm()
                    .child("No meetings yet. Use Record in the toolbar to begin."),
            );
        }
        if let Some(meeting) = selected {
            let displayed_notes = self.meeting_notes_input.read(cx).value().to_string();
            let dirty = self.meeting_title_input.read(cx).value().as_ref() != meeting.title
                || displayed_notes != meeting.notes;
            let system_audio_present = meeting
                .audio_path
                .as_ref()
                .is_some_and(|path| path.is_file());
            let microphone_audio_present = meeting
                .microphone_path
                .as_ref()
                .is_some_and(|path| path.is_file());
            let has_audio = system_audio_present || microphone_audio_present;
            let finished = matches!(
                meeting.status,
                meetings::Status::Ready | meetings::Status::NeedsAttention
            );
            panel = panel.child(
                v_flex()
                    .gap_3()
                    .p_4()
                    .rounded_lg()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.muted)
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .child(Self::meeting_status_label(meeting.status)),
                    )
                    .when_some(meeting.error.clone(), |view, error| {
                        view.child(div().text_sm().text_color(theme.danger).child(error))
                    })
                    .child(div().text_xs().font_semibold().child("TITLE"))
                    .child(modal_input(&self.meeting_title_input, "Meeting title"))
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(div().text_xs().font_semibold().child("NOTES"))
                            .child(if self.meeting_notes_editing {
                                Button::new("cancel-meeting-notes")
                                    .small()
                                    .ghost()
                                    .label("Cancel editing")
                                    .on_click(cx.listener({
                                        let notes = meeting.notes.clone();
                                        move |this, _, window, cx| {
                                            this.meeting_notes_input.update(cx, |input, cx| {
                                                input.set_value(notes.clone(), window, cx)
                                            });
                                            this.meeting_notes_editing = false;
                                            cx.notify();
                                        }
                                    }))
                            } else {
                                Button::new("edit-meeting-notes")
                                    .small()
                                    .ghost()
                                    .label("Edit notes")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.meeting_notes_editing = true;
                                        this.meeting_notes_input.update(cx, |input, cx| {
                                            input.focus(window, cx)
                                        });
                                        cx.notify();
                                    }))
                            }),
                    )
                    .child(if self.meeting_notes_editing {
                        Textarea::new(&self.meeting_notes_input)
                            .aria_label("Meeting notes")
                            .h(px(260.))
                            .into_any_element()
                    } else {
                        div()
                            .min_w_0()
                            .p_3()
                            .rounded_md()
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.background)
                            .child(if displayed_notes.trim().is_empty() {
                                div()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .child("Notes will appear after processing. You can also add your own.")
                                    .into_any_element()
                            } else {
                                TextView::markdown("meeting-notes-preview", displayed_notes)
                                    .text_sm()
                                    .into_any_element()
                            })
                            .into_any_element()
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("save-meeting")
                                    .small()
                                    .primary()
                                    .disabled(!dirty)
                                    .label("Save changes")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.save_meeting_edit(cx)),
                                    ),
                            )
                            .child(
                                Button::new("retry-meeting")
                                    .small()
                                    .ghost()
                                    .disabled(
                                        !has_audio
                                            || matches!(
                                                meeting.status,
                                                meetings::Status::Starting
                                                    | meetings::Status::Recording
                                                    | meetings::Status::Stopping
                                                    | meetings::Status::Transcribing
                                                    | meetings::Status::GeneratingNotes
                                            ),
                                    )
                                    .label("Retry processing")
                                    .on_click(cx.listener({
                                        let id = meeting.id.clone();
                                        move |this, _, _, _| {
                                            this.core.send(CoreCommand::RetryMeeting(id.clone()))
                                        }
                                    })),
                            ),
                    )
                    .child(div().text_xs().font_semibold().child("TRANSCRIPT"))
                    .child(
                        div()
                            .id("meeting-transcript-scroll")
                            .h(px(240.))
                            .overflow_y_scroll()
                            .p_3()
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.background)
                            .child(if meeting.transcript.is_empty() {
                                "Transcript will appear after recording stops.".to_string()
                            } else {
                                meeting.transcript.clone()
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("play-meeting-audio")
                                    .small()
                                    .ghost()
                                    .disabled(!system_audio_present || !finished)
                                    .label("Play meeting audio")
                                    .on_click(cx.listener({
                                        let id = meeting.id.clone();
                                        move |this, _, _, _| {
                                            this.core.send(CoreCommand::OpenMeetingAudio {
                                                id: id.clone(),
                                                microphone: false,
                                            })
                                        }
                                    })),
                            )
                            .child(
                                Button::new("play-microphone-audio")
                                    .small()
                                    .ghost()
                                    .disabled(!microphone_audio_present || !finished)
                                    .label("Play microphone")
                                    .on_click(cx.listener({
                                        let id = meeting.id.clone();
                                        move |this, _, _, _| {
                                            this.core.send(CoreCommand::OpenMeetingAudio {
                                                id: id.clone(),
                                                microphone: true,
                                            })
                                        }
                                    })),
                            )
                            .child(
                                Button::new("delete-meeting-audio")
                                    .small()
                                    .ghost()
                                    .disabled(!has_audio || !finished)
                                    .label("Delete audio")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.delete_meeting_audio(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("delete-meeting")
                                    .small()
                                    .ghost()
                                    .disabled(!finished)
                                    .label("Delete meeting")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.delete_meeting(window, cx)
                                    })),
                            ),
                    ),
            );
        }
        panel.into_any_element()
    }

    fn meeting_status_label(status: meetings::Status) -> &'static str {
        match status {
            meetings::Status::Starting => "Starting recording…",
            meetings::Status::Recording => "Recording",
            meetings::Status::Stopping => "Saving recording…",
            meetings::Status::Transcribing => "Transcribing…",
            meetings::Status::GeneratingNotes => "Generating notes…",
            meetings::Status::Ready => "Ready",
            meetings::Status::NeedsAttention => "Needs attention",
        }
    }
}
