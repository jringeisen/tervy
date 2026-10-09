//! Opt-in ten-noisy-terminal acceptance probe. No timers or clocks in normal use.
use std::time::{Duration, Instant};
pub struct ManyProbe {
    pub requested: bool,
    pub attached: usize,
    pub started: Option<Instant>,
    pub last_frame: Option<Instant>,
    pub frames: Vec<f64>,
    pub switches: Vec<f64>,
    pub switch_count: usize,
    pub palette_started: bool,
    pub palette_ms: Option<f64>,
    pub fuzzy_matches: usize,
    pub pending_key: Option<(Instant, String)>,
    pub echoes: Vec<f64>,
    pub pids: Vec<u32>,
    pub shutdown: Option<Instant>,
}
impl ManyProbe {
    pub fn new() -> Self {
        Self {
            requested: false,
            attached: 0,
            started: None,
            last_frame: None,
            frames: vec![],
            switches: vec![],
            switch_count: 0,
            palette_started: false,
            palette_ms: None,
            fuzzy_matches: 0,
            pending_key: None,
            echoes: vec![],
            pids: vec![],
            shutdown: None,
        }
    }
    pub fn age(&self) -> Duration {
        self.started
            .map_or(Duration::ZERO, |started| started.elapsed())
    }
    pub fn report(&self, clean: bool) {
        fn percentile(samples: &[f64], q: f64) -> f64 {
            if samples.is_empty() {
                return 0.;
            }
            let mut values = samples.to_vec();
            values.sort_by(f64::total_cmp);
            values[((values.len() - 1) as f64 * q).round() as usize]
        }
        eprintln!(
            "devdesk-many {{\"sessions\":{},\"noisy_terminals\":10,\"palette_frame_ms\":{:.3},\"fuzzy_matches\":{},\"visible\":3,\"frame_samples\":{},\"frame_p95_ms\":{:.3},\"frame_max_ms\":{:.3},\"echo_samples_ms\":{:?},\"switch_samples_ms\":{:?},\"echo_p95_ms\":{:.3},\"shutdown_ms\":{:.3},\"clean_shutdown\":{},\"sids\":{:?}}}",
            self.attached,
            self.palette_ms.unwrap_or(-1.),
            self.fuzzy_matches,
            self.frames.len(),
            percentile(&self.frames, 0.95),
            percentile(&self.frames, 1.),
            self.echoes,
            self.switches,
            percentile(&self.echoes, 0.95),
            self.shutdown
                .map_or(0., |started| started.elapsed().as_secs_f64() * 1000.),
            clean,
            self.pids
        );
    }
}
