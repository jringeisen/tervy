//! Configured-process policy; the supervisor performs all I/O. Times are
//! monotonic durations supplied by its Clock, so backoff never follows wall time.
use crate::config::{ProcessSpec, Project};
use crate::env::ShellEnvironment;
use crate::supervisor::SessionStatus;
use devdesk_term::SpawnSpec;
use std::collections::VecDeque;
use std::time::Duration;

pub fn launch(
    project: &Project,
    spec: &ProcessSpec,
    environment: &ShellEnvironment,
    lines: usize,
) -> SpawnSpec {
    let cwd = spec
        .cwd
        .as_ref()
        .map(|path| {
            if path.is_absolute() {
                path.clone()
            } else if let Ok(rest) = path.strip_prefix("~/") {
                std::path::PathBuf::from(
                    environment
                        .variables
                        .get("HOME")
                        .cloned()
                        .unwrap_or_default(),
                )
                .join(rest)
            } else {
                project.path.join(path)
            }
        })
        .unwrap_or_else(|| project.path.clone());
    let mut env = environment.variables.clone();
    env.extend(spec.env.clone());
    SpawnSpec {
        program: environment.shell.clone(),
        args: if spec.shell.as_deref() == Some("interactive") {
            vec!["-i".into(), "-c".into(), spec.command.clone()]
        } else {
            vec!["-c".into(), spec.command.clone()]
        },
        cwd,
        env,
        scrollback_lines: lines,
    }
}

pub struct Runtime {
    /// Immutable for the current launch; hot-reloaded settings apply on restart.
    pub spec: ProcessSpec,
    pub state: SessionStatus,
    pub retry_at: Option<Duration>,
    pub last_exit: Option<i32>,
    requested: bool,
    outcome: Option<SessionStatus>,
    failures: VecDeque<Duration>,
    retries: u32,
    running_since: Option<Duration>,
}
impl Runtime {
    pub fn new(spec: ProcessSpec) -> Self {
        Self {
            spec,
            state: SessionStatus::Stopped,
            retry_at: None,
            last_exit: None,
            requested: false,
            outcome: None,
            failures: VecDeque::new(),
            retries: 0,
            running_since: None,
        }
    }
    pub fn start(&mut self, manual: bool) {
        if manual {
            self.failures.clear();
            self.retries = 0;
        }
        self.requested = false;
        self.running_since = None;
        self.outcome = None;
        self.retry_at = None;
        self.state = SessionStatus::Starting;
    }
    pub fn spawned(&mut self, now: Duration) {
        self.running_since = Some(now);
        if !self.requested && self.outcome.is_none() {
            self.state = SessionStatus::Running;
        }
    }
    pub fn stop(&mut self) {
        self.requested = true;
        self.retry_at = None;
        self.state = SessionStatus::Stopping;
    }
    /// Leader exit is not completion. Stay Stopping until the supervisor has
    /// verified an empty session and joined its drained reader.
    pub fn exited(&mut self, code: Option<i32>) {
        self.last_exit = code;
        self.outcome = Some(if code == Some(0) {
            SessionStatus::Stopped
        } else {
            SessionStatus::Failed(format!(
                "Exited with {}",
                code.map_or_else(|| "signal".into(), |code| code.to_string())
            ))
        });
        self.state = SessionStatus::Stopping;
    }
    /// Infrastructure failure requires manual intervention, not crash retries.
    pub fn fail(&mut self, reason: String) {
        self.requested = false;
        self.outcome = None;
        self.retry_at = None;
        self.state = SessionStatus::Failed(reason);
    }
    pub fn spawn_failed(&mut self, reason: String, now: Duration) {
        self.outcome = Some(SessionStatus::Failed(reason));
        self.cleaned(now);
    }
    pub fn cleaned(&mut self, now: Duration) {
        self.retry_at = None;
        self.state = if self.requested {
            SessionStatus::Stopped
        } else {
            self.outcome.take().unwrap_or(SessionStatus::Stopped)
        };
        if !matches!(self.state, SessionStatus::Failed(_))
            || self.spec.restart.policy != "on-failure"
        {
            return;
        }
        if self
            .running_since
            .take()
            .is_some_and(|start| now.saturating_sub(start) >= Duration::from_secs(60))
        {
            self.failures.clear();
            self.retries = 0;
        }
        while self
            .failures
            .front()
            .is_some_and(|old| now.saturating_sub(*old) >= Duration::from_secs(60))
        {
            self.failures.pop_front();
        }
        self.failures.push_back(now);
        if self.failures.len() >= 5 {
            self.state = SessionStatus::Failed(
                "Crash loop: five failures in 60 seconds; Start to try again".into(),
            );
        } else if self.retries < self.spec.restart.max_retries {
            self.retry_at = Some(now + Duration::from_secs((1u64 << self.retries.min(5)).min(30)));
            self.retries += 1;
        }
    }
    pub fn retry_due(&self, now: Duration) -> bool {
        self.retry_at.is_some_and(|due| now >= due)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec() -> ProcessSpec {
        crate::config::parse(&format!("[[projects]]\nid='p'\nname='P'\npath='{}'\n[[projects.processes]]\nid='server'\nname='Server'\ncommand='exit 1'\nrestart={{policy='on-failure',max_retries=10}}",std::env::temp_dir().display()), &std::env::temp_dir()).unwrap().projects.remove(0).processes.remove(0)
    }
    #[test]
    fn lifecycle_waits_for_cleanup_and_requested_stops_do_not_restart() {
        let mut p = Runtime::new(spec());
        assert_eq!(p.state, SessionStatus::Stopped);
        p.start(true);
        assert_eq!(p.state, SessionStatus::Starting);
        p.stop();
        p.spawned(Duration::ZERO);
        assert_eq!(p.state, SessionStatus::Stopping);
        p.exited(Some(2));
        assert_eq!(p.state, SessionStatus::Stopping);
        p.cleaned(Duration::ZERO);
        assert_eq!(p.state, SessionStatus::Stopped);
        assert!(p.retry_at.is_none());
        p.start(true);
        p.spawned(Duration::ZERO);
        assert_eq!(p.state, SessionStatus::Running);
        p.exited(Some(0));
        assert_eq!(p.state, SessionStatus::Stopping);
        p.cleaned(Duration::ZERO);
        assert_eq!(p.state, SessionStatus::Stopped);
        p.start(true);
        p.spawn_failed("not found".into(), Duration::ZERO);
        assert!(matches!(p.state, SessionStatus::Failed(_)));
        assert_eq!(p.retry_at, Some(Duration::from_secs(1)));
        p.stop();
        p.cleaned(Duration::ZERO);
        assert_eq!(p.state, SessionStatus::Stopped);
        assert!(p.retry_at.is_none());
        p.start(true);
        p.fail("reader panicked".into());
        assert_eq!(p.state, SessionStatus::Failed("reader panicked".into()));
        assert!(p.retry_at.is_none());
    }
    #[test]
    fn backoff_caps_and_crash_loop_gives_up_then_manual_start_resets() {
        let mut p = Runtime::new(spec());
        for n in 0..5 {
            p.start(n == 0);
            p.spawned(Duration::from_secs(n));
            p.exited(Some(9));
            p.cleaned(Duration::from_secs(n));
            if n < 4 {
                assert_eq!(p.retry_at, Some(Duration::from_secs(n + (1 << n))));
            }
        }
        assert!(p.retry_at.is_none());
        assert!(matches!(&p.state,SessionStatus::Failed(reason) if reason.contains("Crash loop")));
        p.start(true);
        p.exited(None);
        p.cleaned(Duration::from_secs(10));
        assert_eq!(p.retry_at, Some(Duration::from_secs(11)));
        // Space failures apart to exercise the 30s cap without the crash-loop guard.
        for n in 1..8 {
            p.start(false);
            p.spawn_failed("fail".into(), Duration::from_secs(10 + n * 61));
        }
        assert_eq!(p.retry_at, Some(Duration::from_secs(10 + 7 * 61 + 30)));
    }
}
