use std::ffi::OsStr;
use std::fs;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";

pub trait Clock: Send + Sync {
    fn now_unix_ms(&self) -> u64;
    /// Startup deadlines must not move when the host's wall clock changes.
    fn now_monotonic_ms(&self) -> u64 {
        self.now_unix_ms()
    }
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_monotonic_ms(&self) -> u64 {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis() as u64
    }

    fn now_unix_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// The host's accounts, by uid.
pub trait UserDirectory: Send + Sync {
    /// The name of the account that currently has this uid, if any.
    fn username(&self, uid: u32) -> Option<String>;
}

pub struct SystemUserDirectory;

impl UserDirectory for SystemUserDirectory {
    fn username(&self, uid: u32) -> Option<String> {
        users::get_user_by_uid(uid).map(|user| user.name().to_string_lossy().into_owned())
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

/// systemd holds services ordered after prismd until this notification arrives.
/// An interactive daemon invocation has no notification socket.
pub(super) fn notify_ready() -> std::io::Result<()> {
    match std::env::var_os("NOTIFY_SOCKET") {
        Some(path) => notify_ready_to(&path),
        None => Ok(()),
    }
}

fn notify_ready_to(path: &OsStr) -> std::io::Result<()> {
    let address = if let Some(name) = path.as_bytes().strip_prefix(b"@") {
        SocketAddr::from_abstract_name(name)?
    } else {
        SocketAddr::from_pathname(path)?
    };
    UnixDatagram::unbound()?.send_to_addr(b"READY=1", &address)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemd_readiness_supports_filesystem_and_abstract_notification_sockets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notify.sock");
        let filesystem = UnixDatagram::bind(&path).unwrap();
        let name = format!("prism-ready-{}", std::process::id());
        let address = SocketAddr::from_abstract_name(name.as_bytes()).unwrap();
        let abstract_socket = UnixDatagram::bind_addr(&address).unwrap();
        for (socket, path) in [
            (filesystem, path.into_os_string()),
            (abstract_socket, format!("@{name}").into()),
        ] {
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                .unwrap();
            notify_ready_to(&path).unwrap();
            let mut message = [0; 64];
            let size = socket.recv(&mut message).unwrap();
            assert_eq!(&message[..size], b"READY=1");
        }
    }
}
