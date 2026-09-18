//! Opt-in stage timings for local creation diagnostics; never enabled by default.
use std::time::Instant;

pub(crate) struct Stages {
    enabled: bool,
    previous: Instant,
}

impl Stages {
    pub(crate) fn new() -> Self {
        Self {
            enabled: std::env::var_os("NDSTOOL_PROFILE_CREATE").is_some(),
            previous: Instant::now(),
        }
    }

    pub(crate) fn mark(&mut self, stage: &str) {
        let now = Instant::now();
        if self.enabled {
            eprintln!(
                "create-profile {stage}: {:.3} ms",
                now.duration_since(self.previous).as_secs_f64() * 1000.0
            );
        }
        self.previous = now;
    }
}
