use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";

pub trait Clock: Send + Sync {
    fn now_unix_ms(&self) -> u64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// Whether the host is shutting down, which is the only time a daemon stop counts as clean.
pub trait HostShutdownSignal: Send + Sync {
    fn host_shutting_down(&self) -> bool;
}

pub struct SystemdShutdownSignal;

impl HostShutdownSignal for SystemdShutdownSignal {
    fn host_shutting_down(&self) -> bool {
        Command::new("systemctl")
            .arg("is-system-running")
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim() == "stopping")
            .unwrap_or(false)
    }
}

/// The kernel's identifier for the current host boot, or an empty string when unreadable.
pub fn read_boot_id() -> String {
    match fs::read_to_string(BOOT_ID_PATH) {
        Ok(boot_id) => boot_id.trim().to_string(),
        Err(error) => {
            eprintln!("failed to read the kernel boot id from {BOOT_ID_PATH}: {error}");
            String::new()
        }
    }
}
