//! System monitoring for the stuck writeback workaround tool.
use anyhow::{Context, Result};
use cnproc::{PidEvent, PidMonitor};
use log::debug;
use procfs::process::{all_processes, Process};
use procfs::WithCurrentSystemInfo;

/// Information about a process.
#[derive(Debug, Clone)]
pub struct ProcInfo {
    /// The user ID of the process.
    pub uid: u32,
    /// The start time of the process.
    pub starttime: chrono::DateTime<chrono::Local>,
    /// The command name of the process.
    pub comm: String,
}

/// A predicate for identifying kworker processes.
pub trait IsKworkerFn: Fn(&ProcInfo) -> bool {}
impl<T: Fn(&ProcInfo) -> bool> IsKworkerFn for T {}

/// A trait for system interactions, allowing for mock implementations for testing.
pub trait System {
    /// Finds the oldest process matching the predicate.
    fn find_oldest_kworker<F: IsKworkerFn>(&self, is_kworker: F) -> Result<Option<ProcInfo>>;
    /// Returns the current time.
    fn now(&self) -> chrono::DateTime<chrono::Local>;
    /// Waits for a new kworker process to appear using the `cnproc` kernel connector.
    ///
    /// This avoids polling to prevent waking an idle system. On an active system, the `timeout`
    /// forces a periodic full process scan to safeguard against missed kernel events.
    fn wait_for_kworker<F: IsKworkerFn>(
        &self,
        is_kworker: F,
        timeout: std::time::Duration,
    ) -> Result<()>;
    /// Triggers a system-wide sync.
    fn sync(&self);
}

/// The live system implementation.
pub struct LiveSystem;

fn to_proc_info(p: Process) -> Result<ProcInfo> {
    let stat = p.stat().context("Failed to get stat")?;
    let uid = p.uid().context("Failed to get uid")?;
    let starttime = stat.starttime().get().context("Failed to get starttime")?;
    Ok(ProcInfo {
        uid,
        comm: stat.comm,
        starttime,
    })
}

impl System for LiveSystem {
    fn find_oldest_kworker<F: IsKworkerFn>(&self, is_kworker: F) -> Result<Option<ProcInfo>> {
        let processes = all_processes().context("Failed to get all processes")?;
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
        let mut monitor = PidMonitor::new().context("Failed to create PidMonitor")?;
        let start = std::time::Instant::now();
        loop {
            // Rescan after a timeout on an active system to safeguard against missed kernel events.
            if start.elapsed() >= timeout {
                debug!("wait_for_kworker used for {timeout:?}, checking full list of processes");
                return Ok(());
            }

            let event = monitor
                .recv()
                .context("PidMonitor socket closed unexpectedly")?;

            let pid = match event {
                PidEvent::Exec { process_pid, .. } => process_pid,
                PidEvent::Fork { child_pid, .. } => child_pid,
                _ => continue,
            };

            if let Ok(proc) = Process::new(pid) {
                if let Ok(info) = to_proc_info(proc) {
                    if is_kworker(&info) {
                        debug!(
                            "Found kworker (pid {}, comm: {}), returning",
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
