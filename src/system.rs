//! Provides abstractions for system interactions, allowing for easier testing and mocking.
use anyhow::{Context, Result};
use cnproc::{PidEvent, PidMonitor};
use log::debug;
use procfs::process::{all_processes, Process};
use procfs::WithCurrentSystemInfo;

/// Contains essential information about a process for the purpose of this tool.
#[derive(Debug, Clone)]
pub struct ProcInfo {
    /// The user ID of the process.
    pub uid: u32,
    /// The time the process started.
    pub starttime: chrono::DateTime<chrono::Local>,
    /// The command associated with the process.
    pub comm: String,
}

/// A predicate used to identify `kworker` processes that should be monitored.
///
/// This trait is used as a bound for the `is_kworker` closure, allowing for more structured
/// and testable code.
pub trait IsKworkerFn: Fn(&ProcInfo) -> bool {}
impl<T: Fn(&ProcInfo) -> bool> IsKworkerFn for T {}

/// Defines the contract for system-level operations required by the workaround.
///
/// This trait allows for a mock implementation to be used during testing, isolating the core
/// logic from actual system calls.
pub trait System {
    /// Finds the oldest running process that matches the given predicate.
    fn find_oldest_kworker<F: IsKworkerFn>(&self, is_kworker: F) -> Result<Option<ProcInfo>>;
    /// Returns the current system time.
    fn now(&self) -> chrono::DateTime<chrono::Local>;
    /// Blocks until a new `kworker` process appears or a timeout occurs.
    ///
    /// This method uses the `cnproc` kernel connector to avoid busy-polling, which is more
    /// efficient. The `timeout` ensures that even on a busy system where kernel events might be
    /// missed, a full process scan is periodically performed.
    fn wait_for_kworker<F: IsKworkerFn>(
        &self,
        is_kworker: F,
        timeout: std::time::Duration,
    ) -> Result<()>;
    /// Triggers a system-wide `sync` to flush filesystem buffers.
    fn sync(&self);
}

/// The production implementation of the `System` trait, interacting with the live system.
pub struct LiveSystem;

fn to_proc_info(p: Process) -> Result<ProcInfo> {
    let stat = p.stat().context("failed to read process stat")?;
    let uid = p.uid().context("failed to read process uid")?;
    let starttime = stat
        .starttime()
        .get()
        .context("failed to get process start time")?;
    Ok(ProcInfo {
        uid,
        comm: stat.comm,
        starttime,
    })
}

impl System for LiveSystem {
    fn find_oldest_kworker<F: IsKworkerFn>(&self, is_kworker: F) -> Result<Option<ProcInfo>> {
        let processes = all_processes().context("failed to list all processes")?;
        let oldest_kworker = processes
            .filter_map(Result::ok)
            .filter_map(|p| to_proc_info(p).ok())
            .filter(is_kworker)
            .min_by_key(|p| p.starttime);
        Ok(oldest_kworker)
    }

    fn now(&self) -> chrono::DateTime<chrono::Local> {
        chrono::Local::now()
    }

    fn wait_for_kworker<F: IsKworkerFn>(
        &self,
        is_kworker: F,
        timeout: std::time::Duration,
    ) -> Result<()> {
        let mut monitor =
            PidMonitor::new().context("failed to create process event monitor (cnproc)")?;
        let start = std::time::Instant::now();
        loop {
            // On a busy system, the kernel may drop netlink events. To safeguard against this,
            // we'll periodically re-scan the full process list.
            if start.elapsed() >= timeout {
                debug!("wait_for_kworker timed out after {timeout:?}, forcing a full process scan");
                return Ok(());
            }

            let event = monitor
                .recv()
                .context("failed to receive process event from kernel")?;

            let pid = match event {
                PidEvent::Exec { process_pid, .. } => process_pid,
                PidEvent::Fork { child_pid, .. } => child_pid,
                _ => continue,
            };

            if let Ok(proc) = Process::new(pid) {
                if let Ok(info) = to_proc_info(proc) {
                    if is_kworker(&info) {
                        debug!(
                            "Detected matching kworker (pid {}, comm: '{}'), returning",
                            pid, info.comm
                        );
                        return Ok(());
                    }
                }
            }
        }
    }

    fn sync(&self) {
        rustix::fs::sync();
    }
}
