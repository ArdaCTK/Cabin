use std::{
    path::Path,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use sysinfo::{Disks, System};

#[derive(Debug, Clone, Default)]
pub struct SystemSnapshot {
    pub cpu: Option<f32>,
    pub gpu: Option<f32>,
    pub ram: Option<f32>,
    pub swap: Option<f32>,
    pub disk: Option<f32>,
}

pub struct SystemMonitor {
    system: System,
    disks: Disks,
    snapshot: SystemSnapshot,
    last_update: Instant,
    /// Set to false permanently the first time nvidia-smi fails so we stop
    /// spawning expensive child processes on non-NVIDIA systems.
    gpu_available: Arc<AtomicBool>,
    /// sysinfo requires two CPU refresh calls separated by a sleep to produce
    /// a meaningful usage percentage.  We prime this during `new()`.
    cpu_primed: bool,
}

impl SystemMonitor {
    pub fn new() -> Self {
        // Prime the CPU sampler: sysinfo needs two refresh calls with a sleep
        // in between to produce a non-zero CPU usage reading.
        let mut system = System::new_all();
        system.refresh_cpu();

        thread::sleep(Duration::from_millis(210));
        system.refresh_cpu();
        system.refresh_memory();

        Self {
            system,
            disks: Disks::new_with_refreshed_list(),
            snapshot: SystemSnapshot::default(),
            // Offset into the past so the first call to `refresh_if_due` fires immediately.
            last_update: Instant::now() - Duration::from_secs(10),
            gpu_available: Arc::new(AtomicBool::new(true)),
            cpu_primed: true,
        }
    }

    /// Refreshes system metrics if more than 900 ms have elapsed since the
    /// last refresh.  Call this once per render frame.
    pub fn refresh_if_due(&mut self, current_dir: &Path) {
        if self.last_update.elapsed() < Duration::from_millis(900) {
            return;
        }
        self.do_refresh(current_dir);
    }

    fn do_refresh(&mut self, current_dir: &Path) {
        if !self.cpu_primed {
            self.system.refresh_cpu();
            self.cpu_primed = true;
            return;
        }

        self.system.refresh_cpu();
        self.system.refresh_memory();
        self.disks = Disks::new_with_refreshed_list();

        let cpu = self.system.global_cpu_info().cpu_usage().clamp(0.0, 100.0);
        self.snapshot.cpu = Some(cpu);
        self.snapshot.ram = percent(self.system.used_memory(), self.system.total_memory());
        self.snapshot.swap = percent(self.system.used_swap(), self.system.total_swap());
        self.snapshot.disk = self
            .disk_for_path(current_dir)
            .and_then(|d| percent(d.total_space() - d.available_space(), d.total_space()));

        // Only query GPU if we have not already seen a failure.
        if self.gpu_available.load(Ordering::Relaxed) {
            match query_gpu_usage() {
                Some(v) => self.snapshot.gpu = Some(v),
                None => {
                    // Mark as permanently unavailable — no more child process forks.
                    self.gpu_available.store(false, Ordering::Relaxed);
                    self.snapshot.gpu = None;
                }
            }
        }

        self.last_update = Instant::now();
    }

    pub fn snapshot(&self) -> &SystemSnapshot {
        &self.snapshot
    }

    fn disk_for_path(&self, dir: &Path) -> Option<&sysinfo::Disk> {
        self.disks
            .list()
            .iter()
            .filter(|d| dir.starts_with(d.mount_point()))
            .max_by_key(|d| d.mount_point().as_os_str().len())
            .or_else(|| self.disks.list().first())
    }
}

fn percent(used: u64, total: u64) -> Option<f32> {
    if total == 0 {
        return None;
    }
    Some((used as f32 / total as f32) * 100.0)
}

/// Queries the first GPU's utilisation via nvidia-smi.  Returns `None` on any
/// error so the caller can permanently disable the feature.
fn query_gpu_usage() -> Option<f32> {
    let output = Command::new("nvidia-smi")
        .args(["--query-gpu=utilization.gpu", "--format=csv,noheader,nounits"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .and_then(|l| l.trim().parse::<f32>().ok())
}
