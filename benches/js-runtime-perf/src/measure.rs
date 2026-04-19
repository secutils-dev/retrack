//! Measurement primitives: a hdrhistogram-backed latency recorder plus a
//! portable peak-RSS probe.
//!
//! We deliberately avoid criterion/divan here. Scenarios run at millisecond
//! scale, one script execution per observation is already a stable unit, and
//! we want direct control over warmup/iteration counts and the eventual JSON
//! shape (see `report.rs`). `hdrhistogram` provides well-tested percentile
//! math; everything else is plain `std`.

use anyhow::Context;
use hdrhistogram::Histogram;
use serde::Serialize;
use std::time::{Duration, Instant};

/// Single scenario measurement summary written into the JSON report.
#[derive(Debug, Clone, Serialize)]
pub struct ScenarioResult {
    pub p50_us: u64,
    pub p90_us: u64,
    pub p99_us: u64,
    pub max_us: u64,
    pub mean_us: u64,
    pub stddev_us: u64,
    pub throughput_ops_per_sec: f64,
    pub iterations: u64,
    pub warmup: u64,
    pub peak_rss_delta_kb: i64,
}

/// Records latency samples and wall-clock throughput for a single scenario.
pub struct Recorder {
    histogram: Histogram<u64>,
    wall_clock: Duration,
    iterations: u64,
    warmup: u64,
    rss_start_kb: i64,
    rss_peak_kb: i64,
}

impl Recorder {
    pub fn new(iterations: u64, warmup: u64) -> anyhow::Result<Self> {
        // Tracks 1 µs … 60 s at 3 significant digits (~0.1% resolution).
        let histogram = Histogram::<u64>::new_with_bounds(1, 60_000_000, 3)
            .context("failed to build hdrhistogram")?;
        let rss_start_kb = peak_rss_kb();

        Ok(Self {
            histogram,
            wall_clock: Duration::ZERO,
            iterations,
            warmup,
            rss_start_kb,
            rss_peak_kb: rss_start_kb,
        })
    }

    /// Time a single measurement and add it to the histogram. Sets the wall
    /// clock and updates the peak-RSS watermark.
    pub fn observe(&mut self, duration: Duration) -> anyhow::Result<()> {
        let us = duration.as_micros().min(u64::from(u32::MAX) as u128) as u64;
        self.histogram
            .record(us.max(1))
            .context("histogram record failed")?;
        self.wall_clock += duration;
        self.rss_peak_kb = self.rss_peak_kb.max(peak_rss_kb());
        Ok(())
    }

    pub fn finalise(self) -> ScenarioResult {
        let throughput = if self.wall_clock.as_secs_f64() > 0.0 {
            self.iterations as f64 / self.wall_clock.as_secs_f64()
        } else {
            0.0
        };

        ScenarioResult {
            p50_us: self.histogram.value_at_quantile(0.50),
            p90_us: self.histogram.value_at_quantile(0.90),
            p99_us: self.histogram.value_at_quantile(0.99),
            max_us: self.histogram.max(),
            mean_us: self.histogram.mean() as u64,
            stddev_us: self.histogram.stdev() as u64,
            throughput_ops_per_sec: throughput,
            iterations: self.iterations,
            warmup: self.warmup,
            peak_rss_delta_kb: (self.rss_peak_kb - self.rss_start_kb).max(0),
        }
    }
}

/// Stopwatch helper so callers don't have to import `Instant` directly.
pub fn now() -> Instant {
    Instant::now()
}

/// Reads the current peak resident set size in kilobytes via `getrusage`.
///
/// macOS returns bytes in `ru_maxrss`; Linux returns kilobytes. Every other
/// target falls back to zero, which is acceptable for the "warn-only"
/// reporting mode - RSS just shows up as a constant 0 delta in the report.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn peak_rss_kb() -> i64 {
    use std::mem::MaybeUninit;

    unsafe {
        let mut ru: MaybeUninit<libc::rusage> = MaybeUninit::uninit();
        if libc::getrusage(libc::RUSAGE_SELF, ru.as_mut_ptr()) != 0 {
            return 0;
        }
        let ru = ru.assume_init();
        // `ru_maxrss` is `c_long`, which is `i64` on 64-bit targets but `i32`
        // on 32-bit. The cast is a no-op on 64-bit platforms and portable.
        #[allow(clippy::unnecessary_cast)]
        let raw = ru.ru_maxrss as i64;
        if cfg!(target_os = "macos") {
            raw / 1024
        } else {
            raw
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn peak_rss_kb() -> i64 {
    0
}
