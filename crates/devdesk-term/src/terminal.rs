//! The only cross-thread data lock is alacritty's fair terminal mutex.
//!
//! Parser callbacks run WITH that lock held. The proxy only sends nonblocking
//! messages or sets atomics; query replies are completed by the consumer after
//! releasing the parser's lock. Output never travels through a UI channel.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoopSender, Msg};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Term, TermMode};
use async_channel::{Receiver, Sender};

use crate::snapshot::{Palette, RenderSnapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dimensions {
    pub columns: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

impl Default for Dimensions {
    fn default() -> Self {
        Self {
            columns: 100,
            rows: 30,
            cell_width: 8,
            cell_height: 18,
        }
    }
}

impl Dimensions {
    pub fn window_size(self) -> WindowSize {
        WindowSize {
            num_lines: self.rows,
            num_cols: self.columns,
            cell_width: self.cell_width,
            cell_height: self.cell_height,
        }
    }
}

impl alacritty_terminal::grid::Dimensions for Dimensions {
    fn total_lines(&self) -> usize {
        self.rows as usize
    }
    fn screen_lines(&self) -> usize {
        self.rows as usize
    }
    fn columns(&self) -> usize {
        self.columns as usize
    }
}

#[derive(Clone, Debug)]
pub enum TerminalSignal {
    Wakeup,
    ChildExit(std::process::ExitStatus),
    Exit,
}

/// Lifecycle messages are low volume (two per instance). Wakeups are bounded
/// by the dirty flag, even if the UI is busy. Other terminal events have a
/// separate bounded queue so hostile title/clipboard output cannot grow it.
/// Queries have a separate priority queue (512); query floods can drop replies
/// rather than block the parser or grow memory. CPR/DA bypass it once bound.
pub struct TerminalEvents {
    pub signals: Receiver<TerminalSignal>,
    pub events: Receiver<Event>,
    pub queries: Receiver<Event>,
}

pub type LifecycleObserver = Box<dyn Fn(TerminalSignal) + Send + Sync>;

#[derive(Clone)]
pub struct EventProxy {
    dirty: Arc<AtomicBool>,
    activity_enabled: Arc<AtomicBool>,
    last_output_ms: Arc<std::sync::atomic::AtomicU64>,
    pub(crate) alive: Arc<AtomicBool>,
    signals: Sender<TerminalSignal>,
    events: Sender<Event>,
    queries: Sender<Event>,
    input: Arc<OnceLock<EventLoopSender>>,
    lifecycle: Arc<OnceLock<LifecycleObserver>>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        match event {
            Event::Wakeup => {
                if self.activity_enabled.load(Ordering::Relaxed) {
                    self.last_output_ms.store(activity_ms(), Ordering::Relaxed);
                }
                self.wakeup();
            }
            Event::ChildExit(status) => {
                self.alive.store(false, Ordering::Release);
                if let Some(notify) = self.lifecycle.get() {
                    notify(TerminalSignal::ChildExit(status));
                }
                let _ = self.signals.try_send(TerminalSignal::ChildExit(status));
            }
            Event::Exit => {
                if let Some(notify) = self.lifecycle.get() {
                    notify(TerminalSignal::Exit);
                }
                let _ = self.signals.try_send(TerminalSignal::Exit);
            }
            Event::PtyWrite(text) => {
                // CPR/DA write-back is nonblocking and never waits for a frame.
                if let Some(input) = self.input.get() {
                    let _ = input.send(Msg::Input(text.into_bytes().into()));
                } else {
                    let _ = self.queries.try_send(Event::PtyWrite(text));
                }
            }
            event @ (Event::ColorRequest(..) | Event::TextAreaSizeRequest(_)) => {
                let _ = self.queries.try_send(event);
            }
            // Never grant clipboard read access, including to a focused pane.
            Event::ClipboardLoad(..) => {}
            Event::ClipboardStore(_, ref text) if text.len() > 100_000 => {}
            Event::MouseCursorDirty | Event::CursorBlinkingChange => {}
            event => {
                let _ = self.events.try_send(event);
            }
        }
    }
}

impl EventProxy {
    pub(crate) fn new() -> (Self, TerminalEvents) {
        let (signals, signal_rx) = async_channel::unbounded();
        let (events, event_rx) = async_channel::bounded(64);
        let (queries, query_rx) = async_channel::bounded(512);
        (
            Self {
                dirty: Arc::new(AtomicBool::new(false)),
                activity_enabled: Arc::new(AtomicBool::new(false)),
                last_output_ms: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                alive: Arc::new(AtomicBool::new(true)),
                signals,
                events,
                queries,
                input: Arc::new(OnceLock::new()),
                lifecycle: Arc::new(OnceLock::new()),
            },
            TerminalEvents {
                signals: signal_rx,
                events: event_rx,
                queries: query_rx,
            },
        )
    }

    pub(crate) fn set_input(&self, input: EventLoopSender) {
        let _ = self.input.set(input);
    }

    pub(crate) fn set_lifecycle(&self, notify: LifecycleObserver) {
        let _ = self.lifecycle.set(notify);
    }

    fn wakeup(&self) {
        if !self.dirty.swap(true, Ordering::AcqRel) {
            let _ = self.signals.try_send(TerminalSignal::Wakeup);
        }
    }
}

#[derive(Clone)]
pub struct TerminalHandle {
    pub term: Arc<FairMutex<Term<EventProxy>>>,
    pub(crate) input: EventLoopSender,
    pub(crate) proxy: EventProxy,
}

impl TerminalHandle {
    /// Codex alone needs a monotonic output age for inferred-status expiry.
    /// The parser updates one atomic per output batch, never the actor mailbox.
    /// UI invalidation does not count as output and no timer runs here.
    pub fn track_output_activity(&self) {
        self.proxy
            .last_output_ms
            .store(activity_ms(), Ordering::Relaxed);
        self.proxy.activity_enabled.store(true, Ordering::Release);
    }
    pub fn output_age(&self) -> std::time::Duration {
        std::time::Duration::from_millis(
            activity_ms().saturating_sub(self.proxy.last_output_ms.load(Ordering::Relaxed)),
        )
    }
    pub fn write(&self, bytes: impl Into<Vec<u8>>) {
        let bytes = bytes.into();
        if !bytes.is_empty() {
            let _ = self.input.send(Msg::Input(bytes.into()));
        }
    }

    pub fn resize(&self, dimensions: Dimensions) {
        self.term.lock().resize(dimensions);
        let _ = self.input.send(Msg::Resize(dimensions.window_size()));
        self.invalidate();
    }

    pub fn mode(&self) -> TermMode {
        *self.term.lock().mode()
    }

    pub fn invalidate(&self) {
        self.proxy.wakeup();
    }

    /// Clear BEFORE copying. Output arriving during/after the copy must be
    /// allowed to enqueue another wakeup (architecture §5, task 1.16).
    pub fn snapshot(&self, palette: &Palette) -> RenderSnapshot {
        self.proxy.dirty.store(false, Ordering::Release);
        let term = self.term.lock();
        RenderSnapshot::from_term(&term, palette)
    }

    /// Query replies run outside parser callbacks. No recursive Term lock and
    /// no UI lookup from the PTY thread. Call promptly on the async event path,
    /// rather than waiting for the next rendered frame.
    pub fn answer_query(&self, event: &Event, palette: &Palette, size: Dimensions) -> bool {
        match event {
            Event::PtyWrite(text) => self.write(text.as_bytes().to_vec()),
            Event::ColorRequest(index, format) => {
                let color =
                    self.term.lock().colors()[*index].unwrap_or_else(|| palette.indexed(*index));
                self.write(format(color).into_bytes());
            }
            Event::TextAreaSizeRequest(format) => {
                self.write(format(size.window_size()).into_bytes())
            }
            _ => return false,
        }
        true
    }
}

fn activity_ms() -> u64 {
    static ORIGIN: OnceLock<std::time::Instant> = OnceLock::new();
    ORIGIN
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_after_snapshot_clear_enqueues_a_followup_wakeup() {
        use std::collections::HashMap;
        use std::time::{Duration, Instant};
        let (session, events) = crate::TerminalSession::spawn(
            crate::SpawnSpec {
                program: "/bin/cat".into(),
                args: vec![],
                cwd: std::env::temp_dir(),
                env: HashMap::new(),
                scrollback_lines: 100,
            },
            Dimensions::default(),
        )
        .unwrap();
        session.handle.invalidate();
        while events.signals.try_recv().is_ok() {}
        // Hold Term to pause the real snapshot after its clear but before its
        // copy. The second wakeup must survive that snapshot's completion.
        let guard = session.handle.term.lock();
        let handle = session.handle.clone();
        let snapshot = std::thread::spawn(move || handle.snapshot(&Palette::default()));
        let deadline = Instant::now() + Duration::from_secs(2);
        while session.handle.proxy.dirty.load(Ordering::Acquire) {
            if Instant::now() >= deadline {
                drop(guard);
                snapshot.join().unwrap();
                panic!("snapshot must clear dirty before waiting for Term");
            }
            std::thread::yield_now();
        }
        session.handle.invalidate();
        drop(guard);
        snapshot.join().unwrap();
        assert!(session.handle.proxy.dirty.load(Ordering::Acquire));
        assert!(matches!(
            events.signals.try_recv(),
            Ok(TerminalSignal::Wakeup)
        ));
        for _ in 0..100_000 {
            session.handle.invalidate();
        }
        assert!(events.signals.is_empty());
    }

    #[test]
    fn output_activity_counts_coalesced_output_but_not_view_invalidation() {
        let (proxy, _events) = EventProxy::new();
        proxy.activity_enabled.store(true, Ordering::Relaxed);
        proxy.last_output_ms.store(u64::MAX, Ordering::Relaxed);
        proxy.wakeup();
        assert_eq!(proxy.last_output_ms.load(Ordering::Relaxed), u64::MAX);
        // Dirty is already set: a coalesced output batch must still renew age.
        proxy.send_event(Event::Wakeup);
        assert_ne!(proxy.last_output_ms.load(Ordering::Relaxed), u64::MAX);
    }
    #[test]
    fn metadata_flood_is_bounded_and_cannot_hide_exit() {
        let (proxy, events) = EventProxy::new();
        for _ in 0..1000 {
            proxy.send_event(Event::Title("title".into()));
        }
        assert_eq!(events.events.len(), 64);
        proxy.send_event(Event::PtyWrite("reply".into()));
        assert!(matches!(events.queries.try_recv(), Ok(Event::PtyWrite(_))));
        proxy.send_event(Event::Exit);
        assert!(matches!(
            events.signals.try_recv(),
            Ok(TerminalSignal::Exit)
        ));
    }
}

#[cfg(test)]
mod fixtures {
    use super::*;
    use crate::keys::{self, Key, KeyEvent, KeyInput, Modifiers};
    use alacritty_terminal::vte::ansi::Processor;

    fn hex(value: &str) -> Vec<u8> {
        value
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }

    #[test]
    fn recorded_agent_startup_queries_and_shift_enter_replay_offline() {
        for fixture in [
            include_str!("../tests/fixtures/claude-startup.json"),
            include_str!("../tests/fixtures/codex-startup.json"),
        ] {
            let fixture: serde_json::Value = serde_json::from_str(fixture).unwrap();
            let size = Dimensions {
                columns: 120,
                rows: 40,
                cell_width: 8,
                cell_height: 18,
            };
            let palette = Palette::default();
            for exchange in fixture["exchanges"].as_array().unwrap() {
                let (proxy, events) = EventProxy::new();
                let mut term = Term::new(
                    alacritty_terminal::term::Config {
                        kitty_keyboard: true,
                        ..Default::default()
                    },
                    &size,
                    proxy,
                );
                let mut parser: Processor = Processor::new();
                parser.advance(&mut term, &hex(exchange["query_hex"].as_str().unwrap()));
                let response = match events
                    .queries
                    .try_recv()
                    .expect("query must produce a reply")
                {
                    Event::PtyWrite(text) => text,
                    Event::ColorRequest(index, format) => format(palette.indexed(index)),
                    Event::TextAreaSizeRequest(format) => format(size.window_size()),
                    event => panic!("unexpected query event: {event:?}"),
                };
                assert_eq!(
                    response.as_bytes(),
                    hex(exchange["reply_hex"].as_str().unwrap()),
                    "{}",
                    fixture["agent"]
                );
                assert!(events.queries.is_empty());
            }
            for key in fixture["keys"].as_array().unwrap() {
                let input = KeyInput {
                    key: Key::Enter,
                    modifiers: Modifiers {
                        shift: true,
                        ..Modifiers::default()
                    },
                    event: KeyEvent::Press,
                    text: None,
                    shifted: None,
                    base_layout: None,
                };
                assert_eq!(
                    keys::encode(&input, TermMode::NONE, false),
                    hex(key["legacy_hex"].as_str().unwrap())
                );
                assert_eq!(
                    keys::encode(&input, TermMode::DISAMBIGUATE_ESC_CODES, false),
                    hex(key["kitty_flags_1_hex"].as_str().unwrap())
                );
            }
        }
    }
}

#[cfg(test)]
mod policy {
    use super::*;
    use alacritty_terminal::term::{Config, Osc52};
    use alacritty_terminal::vte::ansi::Processor;
    #[test]
    fn osc52_reads_are_denied_even_if_the_parser_is_configured_to_request_them() {
        let (proxy, events) = EventProxy::new();
        let mut term = Term::new(
            Config {
                osc52: Osc52::CopyPaste,
                ..Config::default()
            },
            &Dimensions::default(),
            proxy,
        );
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, b"\x1b]52;c;?\x07");
        assert!(events.events.is_empty());
        assert!(events.queries.is_empty());
        parser.advance(&mut term, b"\x1b]52;c;aGVsbG8=\x07");
        assert!(
            matches!(events.events.try_recv(), Ok(Event::ClipboardStore(_, text)) if text == "hello")
        );
    }
    #[test]
    fn titles_have_a_bound_before_queueing_and_saving_to_the_title_stack() {
        let (proxy, events) = EventProxy::new();
        let mut term = Term::new(Default::default(), &Dimensions::default(), proxy);
        let mut parser: Processor = Processor::new();
        parser.advance(
            &mut term,
            format!("\x1b]0;{}\x07\x1b[22;0t", "界".repeat(10_000)).as_bytes(),
        );
        assert!(
            matches!(events.events.try_recv(), Ok(Event::Title(title)) if title.chars().count() == 200)
        );
        parser.advance(&mut term, b"\x1b]0;short\x07\x1b[23;0t");
        let _ = events.events.try_recv();
        assert!(
            matches!(events.events.try_recv(), Ok(Event::Title(title)) if title.chars().count() == 200)
        );
    }
}
