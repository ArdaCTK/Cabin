use std::{
    path::Path,
    process::Command,
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
}

impl SystemMonitor {
    pub fn new() -> Self {
        let mut system = System::new_all();
        system.refresh_cpu();
        system.refresh_memory();

        Self {
            system,
            disks: Disks::new_with_refreshed_list(),
            snapshot: SystemSnapshot::default(),
            last_update: Instant::now() - Duration::from_secs(10),
        }
    }

    pub fn refresh_if_due(&mut self, current_dir: &Path) {
        if self.last_update.elapsed() < Duration::from_millis(900) {
            return;
        }
        self.refresh(current_dir);
    }

    fn refresh(&mut self, current_dir: &Path) {
        self.system.refresh_cpu();
        self.system.refresh_memory();
        self.disks = Disks::new_with_refreshed_list();

        self.snapshot.cpu = Some(self.system.global_cpu_info().cpu_usage().clamp(0.0, 100.0));
        self.snapshot.ram = percent(self.system.used_memory(), self.system.total_memory());
        self.snapshot.swap = percent(self.system.used_swap(), self.system.total_swap());
        self.snapshot.disk = self
            .disk_for_path(current_dir)
            .and_then(|disk| percent(disk.total_space() - disk.available_space(), disk.total_space()));
        self.snapshot.gpu = query_gpu_usage();

        self.last_update = Instant::now();
    }

    pub fn snapshot(&self) -> &SystemSnapshot {
        &self.snapshot
    }

    fn disk_for_path(&self, current_dir: &Path) -> Option<&sysinfo::Disk> {
        self.disks
            .list()
            .iter()
            .filter(|disk| current_dir.starts_with(disk.mount_point()))
            .max_by_key(|disk| disk.mount_point().as_os_str().len())
            .or_else(|| self.disks.list().first())
    }
}

fn percent(used: u64, total: u64) -> Option<f32> {
    if total == 0 {
        return None;
    }

    Some((used as f32 / total as f32) * 100.0)
}

fn query_gpu_usage() -> Option<f32> {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=utilization.gpu",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .next()
        .and_then(|line| line.trim().parse::<f32>().ok())
}
