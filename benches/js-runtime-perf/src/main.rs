//! Retrack JS runtime performance harness.
//!
//! Runs a fixed catalogue of scenarios against the real
//! [`retrack::js_runtime::JsRuntime`] and writes a single JSON document with
//! per-scenario latency percentiles, throughput, and peak RSS delta. The
//! output is consumed by `scripts/analyze-perf.ts`, which appends one line
//! per run to `.perf/history.jsonl`.
//!
//! This harness is deliberately independent of the Secutils harness - Retrack
//! is a standalone project and owns its own performance history. See
//! `AGENTS.md` for the user-facing contract; this file is the driver.

mod measure;
mod report;
mod scenarios;

use anyhow::Context;
use clap::Parser;
use report::Report;
use std::{path::PathBuf, process::ExitCode};

/// CLI arguments for the perf driver.
#[derive(Parser, Debug, Clone)]
#[command(
    name = "js-runtime-perf",
    about = "Measure Retrack JS runtime performance across a fixed scenario catalogue."
)]
struct Args {
    /// Comma-separated list of scenarios to run, or `all` for every scenario.
    #[arg(long, default_value = "all")]
    scenarios: String,

    /// Number of measured iterations per scenario (after warmup).
    #[arg(long, default_value_t = 500)]
    iterations: u64,

    /// Number of warmup iterations per scenario that are discarded.
    #[arg(long, default_value_t = 50)]
    warmup: u64,

    /// Number of concurrent tasks for the `concurrent_extractors_8x` scenario.
    #[arg(long, default_value_t = 8)]
    concurrency: u64,

    /// Output file path for the JSON report.
    #[arg(long, default_value = "/tmp/perf.json")]
    output: PathBuf,
}

fn main() -> ExitCode {
    // Retrack's JsRuntime is long-lived (single worker thread + persistent isolate),
    // so fd pressure is far lower than in the Secutils harness. We still raise the
    // limit defensively so the harness is robust across platforms and future
    // scenario additions.
    if let Err(err) = raise_fd_limit() {
        eprintln!("warning: failed to raise RLIMIT_NOFILE: {err}");
    }

    let args = Args::parse();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(std::cmp::max(
            2,
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
        ))
        .enable_all()
        .build()
        .context("Failed to build driver Tokio runtime")
    {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("{err:?}");
            return ExitCode::FAILURE;
        }
    };

    match runtime.block_on(run(args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("perf driver failed: {err:?}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> anyhow::Result<()> {
    let selected = parse_scenarios(&args.scenarios);
    let mut report = Report::new();

    for name in scenarios::ALL {
        if !selected.iter().any(|s| s == "all" || s == name) {
            continue;
        }

        eprintln!("▶ {name}");
        let result = scenarios::run(name, args.iterations, args.warmup, args.concurrency)
            .await
            .with_context(|| format!("scenario `{name}` failed"))?;
        eprintln!(
            "  p50={:>6}µs  p90={:>6}µs  p99={:>6}µs  max={:>7}µs  ops/s={:>8.1}  rss_delta_kb={:>6}",
            result.p50_us,
            result.p90_us,
            result.p99_us,
            result.max_us,
            result.throughput_ops_per_sec,
            result.peak_rss_delta_kb
        );
        report.add(name, result);
    }

    report.write(&args.output).context("writing JSON report")?;
    eprintln!("✓ wrote report to {}", args.output.display());
    Ok(())
}

fn parse_scenarios(spec: &str) -> Vec<String> {
    spec.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Raise the soft `RLIMIT_NOFILE` to the hard limit. On macOS the hard limit is
/// reported as `RLIM_INFINITY` but the kernel silently caps at `OPEN_MAX`
/// (10240), so we cap the request at a safe value to avoid `EINVAL`.
#[cfg(unix)]
fn raise_fd_limit() -> Result<(), std::io::Error> {
    use std::io::Error;

    let mut rlim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    let rc = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) };
    if rc != 0 {
        return Err(Error::last_os_error());
    }

    let target = {
        #[cfg(target_os = "macos")]
        {
            std::cmp::min(rlim.rlim_max, 10_240)
        }
        #[cfg(not(target_os = "macos"))]
        {
            rlim.rlim_max
        }
    };

    if rlim.rlim_cur >= target {
        return Ok(());
    }

    rlim.rlim_cur = target;
    let rc = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &rlim) };
    if rc != 0 {
        return Err(Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(unix))]
fn raise_fd_limit() -> Result<(), std::io::Error> {
    Ok(())
}
