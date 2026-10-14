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

/// The ids of every group the user belongs to, including `gid`, or `None` if the host can't say.
/// This replaces `users::get_user_groups`, which pads its answer with group 0 (RUSTSEC-2025-0040),
/// so everyone looked like a member of `root`.
pub fn user_group_ids(username: &OsStr, gid: libc::gid_t) -> Option<Vec<libc::gid_t>> {
    group_ids_growing_from(username, gid, 64)
}

fn group_ids_growing_from(
    username: &OsStr,
    gid: libc::gid_t,
    capacity: usize,
) -> Option<Vec<libc::gid_t>> {
    let name = std::ffi::CString::new(username.as_bytes()).ok()?;
    let mut groups: Vec<libc::gid_t> = vec![0; capacity];
    loop {
        let mut count = libc::c_int::try_from(groups.len()).ok()?;
        // SAFETY: `count` is the buffer's length, and getgrouplist writes at most that many ids.
        let found =
            unsafe { libc::getgrouplist(name.as_ptr(), gid, groups.as_mut_ptr(), &mut count) };
        // When the buffer is too small, getgrouplist returns -1 and sets `count` to the size it
        // needs, so only a -1 that didn't ask for more room is a failure.
        let needed = usize::try_from(count).ok()?;
        if found >= 0 {
            groups.truncate(needed);
            groups.sort_unstable();
            groups.dedup();
            return Some(groups);
        }
        if needed <= groups.len() {
            return None;
        }
        groups.resize(needed, 0);
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
    notify(b"READY=1")
}

/// Sends a notification to systemd, if the daemon runs under it.
pub(super) fn notify(message: &[u8]) -> std::io::Result<()> {
    match std::env::var_os("NOTIFY_SOCKET") {
        Some(path) => notify_to(&path, message),
        None => Ok(()),
    }
}

/// How often systemd expects to hear from the daemon, when it watches it.
pub(super) fn watchdog_interval() -> Option<std::time::Duration> {
    let pid = std::env::var("WATCHDOG_PID").ok();
    if pid.is_some_and(|pid| pid.parse() != Ok(std::process::id())) {
        return None;
    }
    let usec: u64 = std::env::var("WATCHDOG_USEC").ok()?.parse().ok()?;
    (usec > 0).then(|| std::time::Duration::from_micros(usec))
}

fn notify_to(path: &OsStr, message: &[u8]) -> std::io::Result<()> {
    let address = if let Some(name) = path.as_bytes().strip_prefix(b"@") {
        SocketAddr::from_abstract_name(name)?
    } else {
        SocketAddr::from_pathname(path)?
    };
    UnixDatagram::unbound()?.send_to_addr(message, &address)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The groups `id -G` reports for the user running the tests.
    fn id_groups() -> Vec<libc::gid_t> {
        let output = Command::new("id").arg("-G").output().unwrap();
        let mut groups: Vec<libc::gid_t> = String::from_utf8(output.stdout)
            .unwrap()
            .split_whitespace()
            .map(|gid| gid.parse().unwrap())
            .collect();
        groups.sort_unstable();
        groups.dedup();
        groups
    }

    #[test]
    fn a_users_groups_are_exactly_their_memberships() {
        let user = users::get_user_by_uid(users::get_current_uid()).unwrap();
        let groups = user_group_ids(user.name(), user.primary_group_id()).unwrap();
        assert_eq!(groups, id_groups());
    }

    #[test]
    fn group_lookup_grows_its_buffer_when_a_user_has_more_groups_than_it_holds() {
        let user = users::get_user_by_uid(users::get_current_uid()).unwrap();
        let groups = group_ids_growing_from(user.name(), user.primary_group_id(), 0).unwrap();
        assert_eq!(groups, id_groups());
    }

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
            notify_to(&path, b"READY=1").unwrap();
            let mut message = [0; 64];
            let size = socket.recv(&mut message).unwrap();
            assert_eq!(&message[..size], b"READY=1");
        }
    }
}
