mod activity;
mod automatic_tuning;
mod backend;
mod boot_state;
mod cooling;
mod cooling_profiles;
mod daemon_core;
mod games;
mod host;
mod possibly_stale;
mod profiles;
mod programs;
mod queue;
mod recognition;
mod session;
mod startup;
mod steam_library;
mod switching;
mod tuning;
mod tuning_store;

#[cfg(test)]
mod activity_tests;
#[cfg(test)]
mod apply_tests;
#[cfg(test)]
mod cooling_control_tests;
#[cfg(test)]
mod cooling_tests;
#[cfg(test)]
mod current_profile_tests;
#[cfg(test)]
mod fan_curve_tests;
#[cfg(test)]
mod games_changed_tests;
#[cfg(test)]
mod games_tests;
#[cfg(test)]
mod harness;
#[cfg(test)]
mod manage_profile_tests;
#[cfg(test)]
mod overlapping_games_tests;
#[cfg(test)]
mod profile_tests;
#[cfg(test)]
mod profiles_changed_tests;
#[cfg(test)]
mod program_tests;
#[cfg(test)]
mod recognition_tests;
#[cfg(test)]
mod remembered_curve_tests;
#[cfg(test)]
mod restore_tests;
#[cfg(test)]
mod session_tests;
#[cfg(test)]
mod startup_cooling_tests;
#[cfg(test)]
mod startup_tests;
#[cfg(test)]
mod switching_tests;
#[cfg(test)]
mod tests;

use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use serde::{Deserialize, Serialize};
use users::os::unix::UserExt;
use users::{get_group_by_gid, get_group_by_name, get_user_by_uid, get_user_groups};

use prism_drivers::DriverService;
use prism_gpu::{GpuCommandError, GpuCommandErrorCode};
use prism_protocol::framing::{MAX_FRAME_BYTES, read_frame, write_frame};
use prism_protocol::{
    DEFAULT_SOCKET_PATH, DaemonRequest, DaemonResponse, SessionMessage, SessionRequest, Unsupported,
};

pub use backend::{DaemonBackend, RealDaemonBackend};
pub use daemon_core::{CoreDependencies, DaemonCore, HelloRefused, Welcomed};
pub use host::{
    Clock, HostShutdownSignal, SystemClock, SystemUserDirectory, SystemdShutdownSignal,
    UserDirectory, read_boot_id,
};
pub use session::{DaemonSession, SessionPeer, SessionPolicy};

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
struct PrismDaemonConfigFile {
    admin_group: Option<String>,
    admin_user: Option<String>,
}

type PrismDaemonConfig = SessionPolicy;

#[derive(Debug, Clone, Copy)]
struct PeerCredentials {
    uid: u32,
    gid: u32,
}

pub fn run() -> Result<(), String> {
    run_with(Arc::new(RealDaemonBackend::default()))
}

/// Runs the daemon on these GPUs, configured from the environment like the installed service.
pub fn run_with(backend: Arc<dyn DaemonBackend>) -> Result<(), String> {
    let stop_signals = block_stop_signals().map_err(|error| error.to_string())?;
    let paths = DaemonPaths::from_environment();
    let config = load_config(&paths).map_err(|error| error.to_string())?;
    let listener = bind_listener(&paths).map_err(|error| error.to_string())?;
    let core = DaemonCore::start(CoreDependencies {
        backend,
        drivers: DriverService::new(paths.driver_jobs_path.clone()),
        persistent_dir: paths.persistent_dir.clone(),
        boot_dir: paths.boot_dir.clone(),
        boot_id: read_boot_id(),
        clock: Arc::new(SystemClock),
        host_shutting_down: Arc::new(SystemdShutdownSignal),
        socket_path: paths.socket_path.clone(),
        host_root: PathBuf::from("/"),
        users: Arc::new(SystemUserDirectory),
    });

    let core = Arc::new(core);
    core.start_recognition()
        .map_err(|error| error.to_string())?;
    core.start_cooling().map_err(|error| error.to_string())?;
    core.start_telemetry().map_err(|error| error.to_string())?;
    if let Some(interval) = host::watchdog_interval() {
        let supervised = Arc::clone(&core);
        thread::Builder::new()
            .name("prismd-watchdog".into())
            .spawn(move || supervise_cooling(&supervised, interval))
            .map_err(|error| error.to_string())?;
    }
    let stopping = Arc::clone(&core);
    thread::Builder::new()
        .name("prismd-stop".into())
        .spawn(move || {
            let mut signal = 0;
            // SAFETY: the set holds the blocked stop signals and outlives the call.
            unsafe { libc::sigwait(&stop_signals, &mut signal) };
            stopping.stop();
            std::process::exit(0);
        })
        .map_err(|error| error.to_string())?;
    let starting = Arc::clone(&core);
    thread::Builder::new()
        .name("prismd-readiness".into())
        .spawn(move || {
            starting.wait_for_startup();
            if let Err(error) = host::notify_ready() {
                eprintln!("failed to notify systemd that startup finished: {error}");
            }
        })
        .map_err(|error| error.to_string())?;
    serve(listener, core, Arc::new(config));
    Ok(())
}

/// Tells the service manager the daemon is alive only while the curve loop, and each curve it
/// follows, keeps making progress. When pings stop, the service manager aborts the daemon and its
/// stop command returns abandoned curve fans to automatic cooling, without relying on the stalled
/// loop.
fn supervise_cooling(core: &DaemonCore, interval: std::time::Duration) {
    let ping_every = interval / 3;
    let window_ms = (interval / 2).as_millis() as u64;
    loop {
        let live = core.cooling_is_live(window_ms);
        // A stalled curve is a fault: whoever recovers after the watchdog stops the daemon must
        // not resume it.
        core.record_cooling_stall(!live);
        if live {
            if let Err(error) = host::notify(b"WATCHDOG=1") {
                eprintln!("failed to notify systemd's watchdog: {error}");
            }
        } else {
            eprintln!(
                "Prism's fan curve control has stalled; letting the watchdog stop the daemon"
            );
        }
        thread::sleep(ping_every);
    }
}

/// Returns fans that a stopped daemon left following a curve to the GPU's default automatic
/// cooling. The service manager runs this after the daemon stops for any reason.
pub fn restore_cooling() -> Result<(), String> {
    restore_cooling_with(&RealDaemonBackend::default())
}

pub fn restore_cooling_with(backend: &dyn DaemonBackend) -> Result<(), String> {
    let paths = DaemonPaths::from_environment();
    cooling::recover_abandoned_curves(backend, &paths.boot_dir, &read_boot_id());
    Ok(())
}

/// systemd stops the daemon with SIGTERM, including while the host shuts down. Blocking the stop
/// signals before any thread starts leaves them for one thread to receive, so the daemon can
/// record a clean shutdown before it exits.
fn block_stop_signals() -> std::io::Result<libc::sigset_t> {
    // SAFETY: the set is initialized by sigemptyset before use.
    unsafe {
        let mut signals: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut signals);
        libc::sigaddset(&mut signals, libc::SIGTERM);
        libc::sigaddset(&mut signals, libc::SIGINT);
        match libc::pthread_sigmask(libc::SIG_BLOCK, &signals, std::ptr::null_mut()) {
            0 => Ok(signals),
            error => Err(std::io::Error::from_raw_os_error(error)),
        }
    }
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
    core: &Arc<DaemonCore>,
    config: &PrismDaemonConfig,
    stream: &mut UnixStream,
) -> Result<(), String> {
    let mut reader = BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);
    // Bounded frames start with zero; legacy requests are unframed JSON.
    if reader
        .fill_buf()
        .map_err(|error| error.to_string())?
        .first()
        == Some(&0)
    {
        return handle_session_connection(core, config, stream, reader);
    }
    let mut request = Vec::new();
    reader
        .take((MAX_FRAME_BYTES + 1) as u64)
        .read_to_end(&mut request)
        .map_err(|error| error.to_string())?;

    if request.len() > MAX_FRAME_BYTES {
        return Err("daemon request too large".into());
    }

    let request: DaemonRequest =
        serde_json::from_slice(&request).map_err(|error| error.to_string())?;
    // A status probe needs no caller; any other request is checked against its caller.
    let response = if matches!(request, DaemonRequest::GetDaemonStatus) {
        core.handle(request)
    } else {
        match resolve_peer(config, stream) {
            Ok(peer) => core.handle_authorized(&peer, config, request),
            Err(error) => DaemonResponse::Error(error),
        }
    };
    let payload = serde_json::to_vec(&response).map_err(|error| error.to_string())?;

    stream
        .write_all(&payload)
        .and_then(|_| stream.flush())
        .map_err(|error| error.to_string())?;

    Ok(())
}

fn handle_session_connection(
    core: &Arc<DaemonCore>,
    config: &SessionPolicy,
    stream: &mut UnixStream,
    mut reader: BufReader<UnixStream>,
) -> Result<(), String> {
    let opening: SessionRequest = read_frame(&mut reader)
        .map_err(|error| error.to_string())?
        .ok_or("missing session opening request")?;
    let admitted = match opening.request {
        DaemonRequest::Hello(hello) => resolve_peer(config, stream)
            .map_err(HelloRefused::Unidentified)
            .and_then(|peer| core.hello(peer, config, hello)),
        _ => Err(HelloRefused::Unidentified(GpuCommandError::new(
            GpuCommandErrorCode::InvalidArgument,
            "the first session request must be Hello",
        ))),
    };
    let Welcomed {
        welcome,
        snapshot,
        mut session,
    } = match admitted {
        Ok(welcomed) => welcomed,
        Err(refused) => {
            write_frame(
                stream,
                &SessionMessage::Response {
                    id: opening.id,
                    response: refused.into(),
                },
            )
            .map_err(|error| error.to_string())?;
            return Ok(());
        }
    };
    write_frame(
        stream,
        &SessionMessage::Response {
            id: opening.id,
            response: DaemonResponse::Welcome(welcome),
        },
    )
    .map_err(|error| error.to_string())?;
    write_frame(stream, &SessionMessage::Snapshot { snapshot })
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(2)))
        .map_err(|error| error.to_string())?;
    let writer = Arc::new(Mutex::new(
        stream.try_clone().map_err(|error| error.to_string())?,
    ));
    let events = session.take_events();
    let event_writer = Arc::clone(&writer);
    let pushing = thread::spawn(move || {
        for event in events {
            if write_frame(
                &mut *event_writer.lock().unwrap(),
                &SessionMessage::Event { event },
            )
            .is_err()
            {
                break;
            }
        }
        let _ = event_writer
            .lock()
            .unwrap()
            .shutdown(std::net::Shutdown::Both);
    });
    let result = loop {
        match read_frame::<SessionFrame>(&mut reader) {
            Ok(Some(frame)) => {
                let request = match serde_json::from_value(frame.request) {
                    Ok(request) => SessionRequest {
                        id: frame.id,
                        request,
                    },
                    // A request this daemon can't read, such as one from a newer Prism.
                    Err(error) => {
                        write_frame(
                            &mut *writer.lock().unwrap(),
                            &SessionMessage::Response {
                                id: frame.id,
                                response: DaemonResponse::Unsupported(Unsupported {
                                    message: error.to_string(),
                                }),
                            },
                        )
                        .map_err(|error| error.to_string())?;
                        continue;
                    }
                };
                // Session-scoped requests, such as hold updates, are ordered on the connection
                // before dispatching later writes.
                if matches!(
                    request.request,
                    DaemonRequest::SetEditHolds(_) | DaemonRequest::SubscribeTelemetry
                ) {
                    let response = core.handle_session(&session, request.request);
                    write_frame(
                        &mut *writer.lock().unwrap(),
                        &SessionMessage::Response {
                            id: request.id,
                            response,
                        },
                    )
                    .map_err(|error| error.to_string())?;
                    continue;
                }
                let core = Arc::clone(core);
                let writer = Arc::clone(&writer);
                // The caller remains attached to this connection, and each request is checked
                // against the policy. Requests can finish independently, so a slow GPU cannot
                // delay EOF detection or other requests.
                let peer = session.peer().clone();
                let policy = session.policy().clone();
                thread::spawn(move || {
                    let response = core.handle_authorized(&peer, &policy, request.request);
                    let mut stream = writer.lock().unwrap();
                    if write_frame(
                        &mut *stream,
                        &SessionMessage::Response {
                            id: request.id,
                            response,
                        },
                    )
                    .is_err()
                    {
                        let _ = stream.shutdown(std::net::Shutdown::Both);
                    }
                });
            }
            Ok(None) => break Ok(()),
            Err(error) => break Err(error.to_string()),
        }
    };
    drop(session);
    let _ = pushing.join();
    result
}

/// A session request whose id can be read, before its request is.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionFrame {
    id: u64,
    request: serde_json::Value,
}

fn resolve_peer(
    config: &SessionPolicy,
    stream: &UnixStream,
) -> Result<SessionPeer, GpuCommandError> {
    let credentials = peer_credentials(stream)
        .map_err(|error| GpuCommandError::new(GpuCommandErrorCode::Io, error.to_string()))?;

    let user = get_user_by_uid(credentials.uid).ok_or_else(|| {
        GpuCommandError::new(
            GpuCommandErrorCode::NoPermission,
            format!("Unable to resolve user for uid {}", credentials.uid),
        )
    })?;

    let username = user.name().to_string_lossy().into_owned();
    let groups = if credentials.uid == 0 || config.admin_user.as_deref() == Some(&username) {
        vec![]
    } else {
        get_user_groups(user.name(), credentials.gid)
            .ok_or_else(|| {
                GpuCommandError::new(
                    GpuCommandErrorCode::NoPermission,
                    format!(
                        "Unable to resolve group membership for user {}",
                        user.name().to_string_lossy()
                    ),
                )
            })?
            .iter()
            .map(|group| group.name().to_string_lossy().into_owned())
            .collect()
    };

    let primary_group = get_group_by_gid(credentials.gid)
        .map(|group| group.name().to_string_lossy().into_owned())
        .unwrap_or_else(|| credentials.gid.to_string());

    Ok(SessionPeer {
        uid: credentials.uid,
        username,
        groups,
        primary_group,
        home: user.home_dir().to_path_buf(),
    })
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

/// Any local process may connect: prismd itself decides what each caller may do (ADR 0011).
fn bind_listener(paths: &DaemonPaths) -> Result<UnixListener, std::io::Error> {
    if let Some(parent) = paths.socket_path.parent() {
        fs::create_dir_all(parent)?;
    }

    if paths.socket_path.exists() {
        fs::remove_file(&paths.socket_path)?;
    }

    let listener = UnixListener::bind(&paths.socket_path)?;
    fs::set_permissions(&paths.socket_path, fs::Permissions::from_mode(0o666))?;

    Ok(listener)
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
