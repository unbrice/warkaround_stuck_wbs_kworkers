// Stuck Writeback Workaround Tool
//
// Monitors root-owned kworker/inode_switch_wbs threads and triggers a sync if they get stuck.
// This is a workaround for a kernel deadlock described in the README.md accompanying this repo.
//
// Usage: Run the binary directly.
mod system;

use anyhow::Context;
use glob_match::glob_match;
use log::{debug, error, info, warn};
use std::thread::sleep;
use std::time::Duration;
use system::{LiveSystem, System};

/// Polling interval when a kworker is running but not yet stuck.
const BUSY_POLLING: Duration = Duration::from_secs(1);
/// Polling interval after an error.
const IDLE_POLLING: Duration = Duration::from_secs(60);
/// On an active system, rescan all processes after this duration to safeguard against missed kernel events.
const MAX_MONITOR_DURATION: Duration = Duration::from_secs(60);
/// After a sync, we'll wait for this duration before checking again.
const EXPECTED_RECOVERY_TIME: Duration = Duration::from_secs(30);

/// Command-line arguments
#[derive(argh::FromArgs, Debug)]
/// Monitors kworker/inode_switch_wbs threads and triggers sync if stuck.
#[argh(help_triggers("-h", "--help"))]
struct Args {
    /// glob pattern for the kworker comm field.
    #[argh(option, default = "String::from(\"kworker/*inode_switch_wbs*\")")]
    process_glob: String,
    /// how long a matching worker can run before a sync is triggered.
    #[argh(
        option,
        from_str_fn(parse_duration),
        default = "chrono::Duration::seconds(30)"
    )]
    runtime_threshold: chrono::Duration,
    /// increase output verbosity (info).
    #[argh(switch, short = 'v')]
    verbose: bool,
    /// maximum output verbosity (debug).
    #[argh(switch, short = 'd')]
    debug: bool,
    /// disable timestamps in logs.
    #[argh(switch)]
    no_timestamps: bool,
}

impl Args {
    fn log_level(&self) -> log::LevelFilter {
        match (self.verbose, self.debug) {
            (false, false) => log::LevelFilter::Warn,
            (true, false) => log::LevelFilter::Info,
            (_, true) => log::LevelFilter::Debug,
        }
    }
}

fn parse_duration(s: &str) -> Result<chrono::Duration, String> {
    let d = humantime::parse_duration(s).map_err(|e| format!("Invalid duration: {e}"))?;
    chrono::Duration::from_std(d).map_err(|e| format!("Duration conversion error: {e}"))
}

fn workaround<T: System>(
    system: &T,
    process_glob: &str,
    runtime_threshold: &chrono::Duration,
) -> anyhow::Result<Duration> {
    let is_kworker = |p: &crate::system::ProcInfo| p.uid == 0 && glob_match(process_glob, &p.comm);

    let oldest_kworker = system
        .find_oldest_kworker(is_kworker)
        .context("Failed to find oldest kworker")?;

    if let Some(kworker) = oldest_kworker {
        let now = system.now();
        let oldest_runtime = now.signed_duration_since(kworker.starttime);
        debug!("Oldest kworker runtime: {}s", oldest_runtime.num_seconds());

        if oldest_runtime > *runtime_threshold {
            warn!(
                "Sync triggered: oldest kworker {} has been running for {}s (threshold: {}s)",
                kworker.comm,
                oldest_runtime.num_seconds(),
                runtime_threshold.num_seconds()
            );
            system.sync();
            Ok(EXPECTED_RECOVERY_TIME)
        } else {
            Ok(BUSY_POLLING)
        }
    } else {
        info!("No kworker found, waiting for new kworker via cn_proc");
        system
            .wait_for_kworker(is_kworker, MAX_MONITOR_DURATION)
            .context("wait_for_kworker error")?;
        Ok(Duration::from_secs(0))
    }
}

fn init_logger(args: &Args) -> anyhow::Result<()> {
    let log_level = args.log_level();
    let timestamp_precision = if args.no_timestamps {
        None
    } else {
        Some(env_logger::fmt::TimestampPrecision::Seconds)
    };
    env_logger::Builder::from_default_env()
        .filter_level(log_level)
        .format_timestamp(timestamp_precision)
        .format_target(false)
        .try_init()?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args: Args = argh::from_env();

    init_logger(&args)?;

    let system = LiveSystem;
    loop {
        let sleep_duration = match workaround(&system, &args.process_glob, &args.runtime_threshold)
        {
            Ok(duration) => duration,
            Err(e) => {
                error!("{e}");
                IDLE_POLLING
            }
        };
        sleep(sleep_duration);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::{IsKworkerFn, ProcInfo, System};
    use anyhow::Result;
    use std::cell::Cell;
    use std::time::Duration;

    struct MockSystem {
        kworker: Option<ProcInfo>,
        now: chrono::DateTime<chrono::Local>,
        sync_calls: Cell<usize>,
        wait_for_kworker_result: Result<(), String>,
    }

    impl Default for MockSystem {
        fn default() -> Self {
            Self {
                kworker: None,
                now: chrono::Local::now(),
                sync_calls: Cell::new(0),
                wait_for_kworker_result: Ok(()),
            }
        }
    }

    impl System for MockSystem {
        fn find_oldest_kworker<F: IsKworkerFn>(&self, is_kworker: F) -> Result<Option<ProcInfo>> {
            Ok(self.kworker.clone().filter(|p| is_kworker(p)))
        }

        fn now(&self) -> chrono::DateTime<chrono::Local> {
            self.now
        }

        fn wait_for_kworker<F: IsKworkerFn>(
            &self,
            _is_kworker: F,
            _timeout: Duration,
        ) -> Result<()> {
            self.wait_for_kworker_result.clone().map_err(|e| anyhow::anyhow!(e))
        }

        fn sync(&self) {
            self.sync_calls.set(self.sync_calls.get() + 1);
        }
    }

    #[test]
    fn test_monitor_and_sync_no_kworker() {
        let system = MockSystem::default();
        let threshold = chrono::Duration::seconds(30);

        let sleep_duration = workaround(&system, "kworker/*", &threshold).unwrap();
        assert_eq!(sleep_duration, Duration::from_secs(0));
        assert_eq!(system.sync_calls.get(), 0);
    }

    #[test]
    fn test_monitor_and_sync_kworker_below_threshold() {
        let now = chrono::Local::now();
        let proc = ProcInfo {
            uid: 0,
            comm: "kworker/0:1".to_string(),
            starttime: now - chrono::Duration::seconds(10),
        };
        let system = MockSystem {
            kworker: Some(proc),
            now,
            ..MockSystem::default()
        };
        let threshold = chrono::Duration::seconds(30);

        let sleep_duration = workaround(&system, "kworker/*", &threshold).unwrap();
        assert_eq!(sleep_duration, BUSY_POLLING);
        assert_eq!(system.sync_calls.get(), 0);
    }

    #[test]
    fn test_monitor_and_sync_kworker_above_threshold() {
        let now = chrono::Local::now();
        let proc = ProcInfo {
            uid: 0,
            comm: "kworker/0:1".to_string(),
            starttime: now - chrono::Duration::seconds(40),
        };
        let system = MockSystem {
            kworker: Some(proc),
            now,
            ..MockSystem::default()
        };
        let threshold = chrono::Duration::seconds(30);

        let sleep_duration = workaround(&system, "kworker/*", &threshold).unwrap();
        assert_eq!(sleep_duration, EXPECTED_RECOVERY_TIME);
        assert_eq!(system.sync_calls.get(), 1);
    }

    #[test]
    fn test_monitor_and_sync_wait_for_kworker_error() {
        let system = MockSystem {
            wait_for_kworker_result: Err("test error".to_string()),
            ..MockSystem::default()
        };
        let threshold = chrono::Duration::seconds(30);

        let result = workaround(&system, "kworker/*", &threshold);
        assert!(result.is_err());
        assert_eq!(system.sync_calls.get(), 0);
    }
}
