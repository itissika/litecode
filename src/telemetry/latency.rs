//! Latency probes for the turn-start and per-step hot path.
//!
//! Kept permanently so turn latency can be measured while optimizing. Off by
//! default and close to free when off: every probe first asks the subscriber
//! whether target `litecode::latency` is enabled at DEBUG (a cached callsite
//! interest check) and never calls `Instant::now()` otherwise.
//!
//! Turn it on with an EnvFilter spec, e.g.
//!
//! ```text
//! LITECODE_LOG=info,litecode::latency=debug
//! ```
//!
//! or the same spec as the log level in Settings (applies live, unless
//! `LITECODE_LOG` is set). Lines go to the serve process stderr and the Web
//! log panel. The JSON file sink (`litecode.log`) stays at info and does not
//! carry them.
//!
//! Every line reads `latency seg=<name> ms=<f64>`, plus `step=` for per-step
//! segments and `parts="a=1.2 b=0.3"` for segments split into sub-stages.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

/// Log target of every latency line.
pub const TARGET: &str = "litecode::latency";

/// Whether latency lines are wanted right now.
#[inline]
pub fn enabled() -> bool {
    tracing::enabled!(target: "litecode::latency", tracing::Level::DEBUG)
}

/// A start instant only when enabled; pair with [`report_since`].
#[inline]
pub fn now() -> Option<Instant> {
    enabled().then(Instant::now)
}

fn ms(d: Duration) -> f64 {
    (d.as_secs_f64() * 10_000.0).round() / 10.0
}

fn emit(seg: &str, elapsed: Duration, step: Option<u64>, parts: &str) {
    let ms = ms(elapsed);
    match (step, parts.is_empty()) {
        (Some(step), true) => {
            tracing::debug!(target: "litecode::latency", seg = %seg, ms, step, "latency")
        }
        (Some(step), false) => tracing::debug!(
            target: "litecode::latency",
            seg = %seg,
            ms,
            step,
            parts = %parts.trim_end(),
            "latency"
        ),
        (None, true) => tracing::debug!(target: "litecode::latency", seg = %seg, ms, "latency"),
        (None, false) => tracing::debug!(
            target: "litecode::latency",
            seg = %seg,
            ms,
            parts = %parts.trim_end(),
            "latency"
        ),
    }
}

/// Report time elapsed since `start` (from [`now`]) under `seg`.
pub fn report_since(seg: &'static str, start: Option<Instant>, step: Option<u64>) {
    if let Some(start) = start {
        emit(seg, start.elapsed(), step, "");
    }
}

/// Times a scope and reports when dropped.
#[must_use = "a Lap reports when it is dropped"]
pub struct Lap {
    seg: &'static str,
    step: Option<u64>,
    start: Option<Instant>,
}

/// Start timing `seg`; the line is written when the returned guard drops.
#[inline]
pub fn lap(seg: &'static str) -> Lap {
    Lap {
        seg,
        step: None,
        start: now(),
    }
}

/// [`lap`] tagged with the agent step.
#[inline]
pub fn lap_step(seg: &'static str, step: u64) -> Lap {
    Lap {
        seg,
        step: Some(step),
        start: now(),
    }
}

impl Lap {
    /// End the lap now (same as dropping it).
    pub fn done(self) {}
}

impl Drop for Lap {
    fn drop(&mut self) {
        if let Some(start) = self.start.take() {
            emit(self.seg, start.elapsed(), self.step, "");
        }
    }
}

/// Time one closure under `seg`.
#[inline]
pub fn time<T>(seg: &'static str, f: impl FnOnce() -> T) -> T {
    let _lap = lap(seg);
    f()
}

/// One segment split into sequential sub-stages, written as a single line:
/// `latency seg=seam ms=12.3 step=1 parts="bash=0.1 subagent=0.0 ..."`.
pub struct Stages {
    seg: &'static str,
    step: Option<u64>,
    start: Option<Instant>,
    last: Option<Instant>,
    parts: String,
}

impl Stages {
    pub fn new(seg: &'static str, step: Option<u64>) -> Self {
        let start = now();
        Self {
            seg,
            step,
            start,
            last: start,
            parts: String::new(),
        }
    }

    /// Close the stage that ran since the previous mark (or the start).
    #[inline]
    pub fn mark(&mut self, part: &str) {
        if let Some(last) = self.last {
            let at = Instant::now();
            let _ = write!(self.parts, "{part}={} ", ms(at - last));
            self.last = Some(at);
        }
    }

    /// Append a pre-formatted fragment (e.g. an [`Acc`] summary).
    pub fn note(&mut self, part: &str, value: impl std::fmt::Display) {
        if self.start.is_some() {
            let _ = write!(self.parts, "{part}={value} ");
        }
    }
}

impl Drop for Stages {
    fn drop(&mut self) {
        if let Some(start) = self.start.take() {
            emit(self.seg, start.elapsed(), self.step, &self.parts);
        }
    }
}

/// Count and total time of many short calls (per-symbol checks, git shows).
#[derive(Debug, Default, Clone, Copy)]
pub struct Acc {
    on: bool,
    count: u32,
    total: Duration,
}

impl Acc {
    pub fn new() -> Self {
        Self {
            on: enabled(),
            ..Self::default()
        }
    }

    #[inline]
    pub fn start(&self) -> Option<Instant> {
        self.on.then(Instant::now)
    }

    #[inline]
    pub fn stop(&mut self, start: Option<Instant>) {
        if let Some(start) = start {
            self.count += 1;
            self.total += start.elapsed();
        }
    }

    #[inline]
    pub fn time<T>(&mut self, f: impl FnOnce() -> T) -> T {
        let start = self.start();
        let out = f();
        self.stop(start);
        out
    }

    pub fn is_on(&self) -> bool {
        self.on
    }
}

impl std::fmt::Display for Acc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}ms", self.count, ms(self.total))
    }
}

#[cfg(test)]
mod tests {
    

}
