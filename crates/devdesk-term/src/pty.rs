//! One owner per PTY instance. Handles can be cloned for input and rendering;
//! dropping the owner stops the child and joins the reader OFF the UI thread.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;

use alacritty_terminal::event_loop::{EventLoop, Msg};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Osc52, Term};
use alacritty_terminal::tty::{self, Shell};

use crate::terminal::{Dimensions, EventProxy, TerminalEvents, TerminalHandle};

pub struct SpawnSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
    pub scrollback_lines: usize,
}

pub struct TerminalSession {
    pub handle: TerminalHandle,
    pub pid: u32,
    pub owned: crate::kill::OwnedSession,
    master: Option<std::fs::File>,
    reader: Option<
        JoinHandle<(
            EventLoop<tty::Pty, EventProxy>,
            alacritty_terminal::event_loop::State,
        )>,
    >,
}

impl TerminalSession {
    /// Spawning and PTY construction do OS I/O; call on a worker, never in a
    /// GPUI update. The explicit program avoids alacritty's macOS login default.
    pub fn spawn(spec: SpawnSpec, size: Dimensions) -> io::Result<(Self, TerminalEvents)> {
        Self::spawn_observed(spec, size, None)
    }

    /// The observer receives only two lifecycle transitions, never output or
    /// wakeups. It must send nonblocking messages and never lock the grid.
    pub fn spawn_observed(
        spec: SpawnSpec,
        size: Dimensions,
        observer: Option<crate::terminal::LifecycleObserver>,
    ) -> io::Result<(Self, TerminalEvents)> {
        Self::spawn_reusing(spec, size, observer, None, "")
    }

    /// Reattach only after the old session is empty and finish() joined its
    /// reader. Each instance gets fresh event/input routing; only Term persists.
    pub fn spawn_reusing(
        mut spec: SpawnSpec,
        size: Dimensions,
        observer: Option<crate::terminal::LifecycleObserver>,
        history: Option<TerminalHandle>,
        separator: &str,
    ) -> io::Result<(Self, TerminalEvents)> {
        if size.columns < 2 || size.rows < 2 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "terminal needs at least 2 rows and columns",
            ));
        }
        spec.env.insert("TERM".into(), "xterm-256color".into());
        spec.env.insert("COLORTERM".into(), "truecolor".into());
        spec.env.insert("TERM_PROGRAM".into(), "DevDesk".into());
        let options = tty::Options {
            shell: Some(Shell::new(spec.program, spec.args)),
            working_directory: Some(spec.cwd),
            env: spec.env,
            drain_on_exit: true,
        };
        let pty = tty::new(&options, size.window_size(), 0)?;
        let pid = pty.child().id();
        let owned = crate::kill::OwnedSession::capture(pid)?;
        let master = pty.file().try_clone()?;
        let (proxy, events) = EventProxy::new();
        if let Some(observer) = observer {
            proxy.set_lifecycle(observer);
        }
        let term = if let Some(history) = history {
            let term = history.term;
            {
                let mut grid = term.lock();
                grid.replace_event_proxy(proxy.clone());
                crate::restart::prepare(&mut grid, separator);
            }
            term
        } else {
            Arc::new(FairMutex::new(Term::new(
                Config {
                    scrolling_history: spec.scrollback_lines,
                    kitty_keyboard: true,
                    osc52: Osc52::OnlyCopy,
                    ..Config::default()
                },
                &size,
                proxy.clone(),
            )))
        };
        let event_loop = EventLoop::new(term.clone(), proxy.clone(), pty, true, false)?;
        let input = event_loop.channel();
        proxy.set_input(input.clone());
        let reader = event_loop.spawn();
        let handle = TerminalHandle { term, input, proxy };
        Ok((
            Self {
                handle,
                pid,
                owned,
                master: Some(master),
                reader: Some(reader),
            },
            events,
        ))
    }

    pub fn signal(&self, signal: i32) -> io::Result<()> {
        if let Some(master) = &self.master {
            self.owned.signal(signal, master)
        } else {
            Ok(())
        }
    }

    pub fn foreground_job(&self) -> Option<String> {
        self.master
            .as_ref()
            .and_then(|master| self.owned.foreground_job(master))
    }

    /// Worker-only handoff. The caller has verified an empty session and final
    /// Exit. Joining here prevents two parsers from ever sharing a process grid.
    pub fn finish(mut self) -> io::Result<TerminalHandle> {
        if let Some(reader) = self.reader.take() {
            reader
                .join()
                .map_err(|_| io::Error::other("PTY reader panicked"))?;
        }
        self.master.take();
        Ok(self.handle.clone())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        // Transfer the open master and join handle to a worker. No process
        // queries, signal syscalls or waits run in a GPUI entity's destructor.
        let master = self.master.take();
        let owned = self.owned;
        let handle = self.handle.clone();
        if let Some(reader) = self.reader.take() {
            let _ = std::thread::Builder::new()
                .name("devdesk-pty-reap".into())
                .spawn(move || {
                    // Normal stops have already finished the supervisor's
                    // graceful escalation. Abandoned owners (spawn result
                    // delivery failure, deadline, headless caller) still sweep
                    // the verified session before closing its master.
                    if let Some(master) = &master {
                        let _ = owned.signal(libc::SIGHUP, master);
                        let _ = owned.signal(libc::SIGKILL, master);
                    }
                    let _ = handle.input.send(Msg::Shutdown);
                    let _ = reader.join();
                });
        }
    }
}
