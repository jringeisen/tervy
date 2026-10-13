//! Startup instrumentation (implementation-plan.md task 0.6).
//!
//! `scripts/perf/startup.sh` launches Tervy with `TERVY_PERF_T0` set to the
//! launch time in nanoseconds since the Unix epoch. Tervy reports how long
//! it took to reach `main` and to render its first frame, then quits if
//! `TERVY_PERF_EXIT=1`. With the variables unset this module does nothing.

use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::App;

pub struct StartupTimer {
    launched_at_ns: u128,
    main_at_ns: u128,
    exit_after_first_frame: bool,
}

impl StartupTimer {
    pub fn from_env() -> Self {
        let main_at_ns = now_ns();
        let launched_at_ns = std::env::var("TERVY_PERF_T0")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(main_at_ns);
        Self {
            launched_at_ns,
            main_at_ns,
            exit_after_first_frame: std::env::var("TERVY_PERF_EXIT").as_deref() == Ok("1"),
        }
    }

    pub fn first_frame_rendered(self, cx: &mut App) {
        let to_ms = |ns: u128| ns as f64 / 1_000_000.0;
        let to_main_ms = to_ms(self.main_at_ns.saturating_sub(self.launched_at_ns));
        let to_first_frame_ms = to_ms(now_ns().saturating_sub(self.launched_at_ns));

        tracing::info!(to_main_ms, to_first_frame_ms, "startup");
        if std::env::var_os("TERVY_PERF_T0").is_some() {
            // A stable, greppable line for the perf scripts.
            eprintln!(
                "tervy-perf to_main_ms={to_main_ms:.1} to_first_frame_ms={to_first_frame_ms:.1}"
            );
        }
        if self.exit_after_first_frame {
            cx.quit();
        }
    }
}

fn now_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default()
}

/// Opt-in startup attribution without attaching a sampler/debugger, whose
/// dyld notifications can themselves delay framework loading on macOS.
pub fn phase(name: &'static str) -> Option<Phase> {
    std::env::var_os("TERVY_PERF_PHASES").map(|_| Phase {
        name,
        started: std::time::Instant::now(),
    })
}
pub struct Phase {
    name: &'static str,
    started: std::time::Instant,
}
impl Drop for Phase {
    fn drop(&mut self) {
        eprintln!(
            "tervy-phase {} {:.3} ms",
            self.name,
            self.started.elapsed().as_secs_f64() * 1000.
        );
    }
}
