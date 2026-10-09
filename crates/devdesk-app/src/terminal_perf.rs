//! Opt-in measurement: clocks and input injection are disabled normally.
//! Synthetic GPUI keystrokes exercise dispatch and native text input, then PTY
//! echo and painting. Physical keyboard delivery and scanout are excluded.
use crate::terminal_view::TerminalView;
use devdesk_term::snapshot::RenderSnapshot;
use gpui_kit::{Entity, Keystroke, Window};
use std::time::Instant;

pub struct TerminalProbe {
    pub scenario: String,
    started: Instant,
    spawn_started: Option<Instant>,
    first: bool,
    frames: u64,
    last: Option<Instant>,
    max_gap_ms: f64,
    pending: Option<(Instant, String)>,
    echoes: Vec<f64>,
    writes: Vec<f64>,
    injected: bool,
}
impl TerminalProbe {
    pub fn from_env() -> Option<Self> {
        let scenario = std::env::var("DEVDESK_PERF_SCENARIO").ok()?;
        if !matches!(
            scenario.as_str(),
            "echo" | "flood" | "seq" | "claude" | "codex"
        ) {
            return None;
        }
        Some(Self {
            scenario,
            started: Instant::now(),
            spawn_started: None,
            first: false,
            frames: 0,
            last: None,
            max_gap_ms: 0.,
            pending: None,
            echoes: Vec::new(),
            writes: Vec::new(),
            injected: false,
        })
    }
    pub fn on_paint(
        &mut self,
        snapshot: &RenderSnapshot,
        view: &Entity<TerminalView>,
        window: &mut Window,
    ) -> bool {
        self.frames += 1;
        let now = Instant::now();
        if let Some(last) = self.last.replace(now) {
            self.max_gap_ms = self.max_gap_ms.max((now - last).as_secs_f64() * 1000.);
        }
        if !self.first && snapshot.runs.iter().any(|run| !run.text.trim().is_empty()) {
            self.first = true;
            eprintln!(
                "devdesk-terminal-perf {{\"scenario\":\"{}\",\"first_content_frame_ms\":{:.3},\"pty_spawn_to_first_content_frame_ms\":{:.3}}}",
                self.scenario,
                self.started.elapsed().as_secs_f64() * 1000.,
                self.spawn_started
                    .unwrap_or(self.started)
                    .elapsed()
                    .as_secs_f64()
                    * 1000.
            );
            if matches!(self.scenario.as_str(), "claude" | "codex") {
                // Leave time for the external sampler to distinguish the CLI
                // tree from the already-finished environment resolver.
                window.on_next_frame(|window, cx| {
                    let window = window.window_handle();
                    cx.spawn(async move |cx| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_secs(1))
                            .await;
                        let _ = window.update(cx, |_, _, cx| cx.quit());
                    })
                    .detach();
                });
            }
        }
        let contains = |needle: &str| snapshot.runs.iter().any(|run| run.text.contains(needle));
        match self.scenario.as_str() {
            "echo" if contains("READY") => {
                if let Some((at, marker)) = &self.pending
                    && contains(marker)
                {
                    self.echoes.push(at.elapsed().as_secs_f64() * 1000.);
                    self.pending = None;
                }
                if self.pending.is_none() {
                    if self.echoes.len() == 20 {
                        eprintln!(
                            "devdesk-terminal-perf {{\"scenario\":\"echo\",\"gpui_key_to_painted_echo_ms\":{:?},\"gpui_key_to_pty_write_ms\":{:?}}}",
                            self.echoes, self.writes
                        );
                        return true;
                    }
                    // Unique lower-case letters cannot match the initial
                    // upper-case READY marker or any previous echoed input.
                    let marker = ((b'a' + self.echoes.len() as u8) as char).to_string();
                    self.pending = Some((Instant::now(), marker.clone()));
                    let view = view.clone();
                    let delay = std::time::Duration::from_micros(
                        400 + (self.echoes.len() as u64 * 417) % 8000,
                    );
                    window.on_next_frame(move |window, cx| {
                        let window = window.window_handle();
                        // Vary input phase relative to a frame; dispatching
                        // every key inside prepaint would hide frame wait.
                        // These one-shot timers exist only in this probe.
                        cx.spawn(async move |cx| {
                            cx.background_executor().timer(delay).await;
                            let _ = window.update(cx, |_, window, cx| {
                                view.update(cx, |view, _| {
                                    if let Some(probe) = view.probe.as_mut() {
                                        probe.begin_dispatch();
                                    }
                                });
                                window.dispatch_keystroke(
                                    Keystroke::parse(&marker).expect("a-t is a valid key"),
                                    cx,
                                );
                            });
                        })
                        .detach();
                    });
                }
            }
            "flood" => {
                if self.first && !self.injected {
                    self.injected = true;
                    window.on_next_frame(|window, cx| {
                        for key in ["q", "u", "e", "u", "e", "d", "enter"] {
                            window.dispatch_keystroke(
                                Keystroke::parse(key).expect("literal valid key"),
                                cx,
                            );
                        }
                    });
                }
                if contains("ECHO:queued") {
                    self.finish();
                    return true;
                }
            }
            "seq" if contains("SEQ_DONE") => {
                self.finish();
                return true;
            }
            _ => {}
        }
        false
    }
    fn begin_dispatch(&mut self) {
        if let Some((at, _)) = self.pending.as_mut() {
            *at = Instant::now();
        }
    }
    pub fn record_spawn_started(&mut self, started: Instant) {
        self.spawn_started = Some(started);
    }
    pub fn record_write(&mut self) {
        if self.scenario == "echo"
            && let Some((at, _)) = &self.pending
        {
            self.writes.push(at.elapsed().as_secs_f64() * 1000.);
        }
    }
    fn finish(&self) {
        eprintln!(
            "devdesk-terminal-perf {{\"scenario\":\"{}\",\"elapsed_ms\":{:.3},\"painted_frames\":{},\"max_paint_gap_ms\":{:.3},\"queued_input_seen\":{}}}",
            self.scenario,
            self.started.elapsed().as_secs_f64() * 1000.,
            self.frames,
            self.max_gap_ms,
            self.injected
        );
    }
}
