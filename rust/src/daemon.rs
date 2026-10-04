mod backend;
mod daemon_core;
mod host;
mod queue;
mod tuning;

#[cfg(test)]
mod harness;
#[cfg(test)]
mod tests;

use std::env;
use std::ffi::CString;
use std::ffi::OsStr;
use std::fs;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use serde::{Deserialize, Serialize};
use users::{get_group_by_gid, get_group_by_name, get_user_by_uid, get_user_groups};

use crate::drivers::DriverService;
use crate::gpu::{GpuCommandError, GpuCommandErrorCode};
use crate::protocol::{DaemonRequest, DaemonResponse};

pub use backend::{DaemonBackend, RealDaemonBackend};
pub use daemon_core::{CoreDependencies, DaemonCore};
pub use host::{read_boot_id, Clock, HostShutdownSignal, SystemClock, SystemdShutdownSignal};

pub const DEFAULT_SOCKET_PATH: &str = "/run/prism/prismd.sock";
pub const DEFAULT_STATE_DIR: &str = "/var/lib/prism";
pub const DEFAULT_BOOT_STATE_DIR: &str = "/run/prism";
pub const DEFAULT_DRIVER_JOBS_PATH: &str = "/var/lib/prism/driver-jobs.json";
pub const DEFAULT_CONFIG_PATH: &str = "/etc/prism/prismd.toml";

#[derive(Debug, Clone)]
struct DaemonPaths {
    socket_path: PathBuf,
    persistent_dir: PathBuf,
    boot_dir: PathBuf,
    driver_jobs_path: PathBuf,
    config_path: PathBuf,
}

impl DaemonPaths {
    fn from_environment() -> Self {
        Self {
            socket_path: env_path("PRISM_SOCKET_PATH", DEFAULT_SOCKET_PATH),
            persistent_dir: env_path("PRISM_STATE_DIR", DEFAULT_STATE_DIR),
            boot_dir: env_path("PRISM_BOOT_STATE_DIR", DEFAULT_BOOT_STATE_DIR),
            driver_jobs_path: env_path("PRISM_DRIVER_JOBS_PATH", DEFAULT_DRIVER_JOBS_PATH),
            config_path: env_path("PRISM_CONFIG_PATH", DEFAULT_CONFIG_PATH),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrismDaemonConfigFile {
    admin_group: Option<String>,
    admin_user: Option<String>,
}

#[derive(Debug, Clone)]
struct PrismDaemonConfig {
    admin_group: String,
    admin_user: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct PeerCredentials {
    uid: u32,
    gid: u32,
}

pub fn run() -> Result<(), String> {
    let paths = DaemonPaths::from_environment();
    let config = load_config(&paths).map_err(|error| error.to_string())?;
    let listener = bind_listener(&paths, &config).map_err(|error| error.to_string())?;
    let core = DaemonCore::start(CoreDependencies {
        backend: Arc::new(RealDaemonBackend),
        drivers: DriverService::new(paths.driver_jobs_path.clone()),
        persistent_dir: paths.persistent_dir.clone(),
        boot_dir: paths.boot_dir.clone(),
        boot_id: read_boot_id(),
        clock: Arc::new(SystemClock),
        host_shutting_down: Arc::new(SystemdShutdownSignal),
        socket_path: paths.socket_path.clone(),
    });

    serve(listener, Arc::new(core), Arc::new(config));
    Ok(())
}

/// Serves each connection on its own thread, so a slow client never holds up another.
fn serve(listener: UnixListener, core: Arc<DaemonCore>, config: Arc<PrismDaemonConfig>) {
    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let core = Arc::clone(&core);
                let config = Arc::clone(&config);
                let spawned = thread::Builder::new()
                    .name("prismd-connection".into())
                    .spawn(move || {
                        if let Err(error) = handle_connection(&core, &config, &mut stream) {
                            eprintln!("failed to serve Prism daemon request: {error}");
                        }
                    });
                if let Err(error) = spawned {
                    eprintln!("failed to start serving Prism daemon connection: {error}");
                }
            }
            Err(error) => eprintln!("failed to accept Prism daemon connection: {error}"),
        }
    }
}

fn handle_connection(
    core: &DaemonCore,
    config: &PrismDaemonConfig,
    stream: &mut UnixStream,
) -> Result<(), String> {
    let mut request = Vec::new();
    stream
        .read_to_end(&mut request)
        .map_err(|error| error.to_string())?;

    let request: DaemonRequest =
        serde_json::from_slice(&request).map_err(|error| error.to_string())?;
    let response = match authorize_request(config, stream, &request) {
        Ok(()) => core.handle(request),
        Err(error) => DaemonResponse::Error(error),
    };
    let payload = serde_json::to_vec(&response).map_err(|error| error.to_string())?;

    stream
        .write_all(&payload)
        .and_then(|_| stream.flush())
        .map_err(|error| error.to_string())?;

    Ok(())
}

fn authorize_request(
    config: &PrismDaemonConfig,
    stream: &UnixStream,
    request: &DaemonRequest,
) -> Result<(), GpuCommandError> {
    if matches!(request, DaemonRequest::GetDaemonStatus) {
        return Ok(());
    }

    let credentials = peer_credentials(stream)
        .map_err(|error| GpuCommandError::new(GpuCommandErrorCode::Io, error.to_string()))?;

    if credentials.uid == 0 {
        return Ok(());
    }

    let user = get_user_by_uid(credentials.uid).ok_or_else(|| {
        GpuCommandError::new(
            GpuCommandErrorCode::NoPermission,
            format!("Unable to resolve user for uid {}", credentials.uid),
        )
    })?;

    if let Some(admin_user) = config.admin_user.as_deref() {
        if user.name() == OsStr::new(admin_user) {
            return Ok(());
        }
    }

    let groups = get_user_groups(user.name(), credentials.gid).ok_or_else(|| {
        GpuCommandError::new(
            GpuCommandErrorCode::NoPermission,
            format!(
                "Unable to resolve group membership for user {}",
                user.name().to_string_lossy()
            ),
        )
    })?;

    if groups
        .iter()
        .any(|group| group.name() == OsStr::new(&config.admin_group))
    {
        return Ok(());
    }

    let primary_group = get_group_by_gid(credentials.gid)
        .map(|group| group.name().to_string_lossy().into_owned())
        .unwrap_or_else(|| credentials.gid.to_string());

    Err(GpuCommandError::new(
        GpuCommandErrorCode::NoPermission,
        format!(
            "User {} is not allowed to access prismd (primary group: {primary_group})",
            user.name().to_string_lossy()
        ),
    ))
}

fn load_config(paths: &DaemonPaths) -> Result<PrismDaemonConfig, std::io::Error> {
    let raw = match fs::read_to_string(&paths.config_path) {
        Ok(contents) => toml::from_str::<PrismDaemonConfigFile>(&contents).map_err(|error| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => PrismDaemonConfigFile {
            admin_group: None,
            admin_user: None,
        },
        Err(error) => return Err(error),
    };

    Ok(PrismDaemonConfig {
        admin_group: raw.admin_group.unwrap_or_else(default_admin_group),
        admin_user: raw.admin_user,
    })
}

fn bind_listener(
    paths: &DaemonPaths,
    config: &PrismDaemonConfig,
) -> Result<UnixListener, std::io::Error> {
    if let Some(parent) = paths.socket_path.parent() {
        fs::create_dir_all(parent)?;
    }

    if paths.socket_path.exists() {
        fs::remove_file(&paths.socket_path)?;
    }

    let listener = UnixListener::bind(&paths.socket_path)?;
    let socket_mode = if config.admin_user.is_some() {
        0o666
    } else {
        0o660
    };
    fs::set_permissions(&paths.socket_path, fs::Permissions::from_mode(socket_mode))?;
    set_socket_group(&paths.socket_path, &config.admin_group)?;

    Ok(listener)
}

fn set_socket_group(socket_path: &Path, admin_group: &str) -> Result<(), std::io::Error> {
    let Some(group) = get_group_by_name(admin_group) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("group '{admin_group}' was not found"),
        ));
    };

    let path = CString::new(socket_path.as_os_str().as_bytes()).map_err(|error| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, error.to_string())
    })?;

    let result = unsafe { libc::chown(path.as_ptr(), u32::MAX, group.gid()) };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(())
}

fn peer_credentials(stream: &UnixStream) -> Result<PeerCredentials, std::io::Error> {
    let fd = stream.as_raw_fd();
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;

    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut credentials as *mut _ as *mut libc::c_void,
            &mut size,
        )
    };

    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(PeerCredentials {
        uid: credentials.uid,
        gid: credentials.gid,
    })
}

fn env_path(var_name: &str, default: &str) -> PathBuf {
    env::var_os(var_name)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

fn default_admin_group() -> String {
    if get_group_by_name("wheel").is_some() {
        "wheel".to_string()
    } else if get_group_by_name("sudo").is_some() {
        "sudo".to_string()
    } else {
        "root".to_string()
    }
}
