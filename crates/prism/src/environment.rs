//! What the composition root is wired to. Production wires the real parts; tests and the
//! simulator wire fakes.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use prism_gpu::GpuProviders;

use crate::activation::DesktopActivation;
use crate::file_chooser::{FileChooser, PortalFileChooser};
use crate::notifications::{DesktopNotifier, Notifier};
use crate::prismd::{PrismdConnector, SocketConnector};
use crate::prismd_host::{HostPrismdService, PrismdService};
use crate::settings::{SettingsFile, SettingsStore};

/// Everything [`App`](crate::App) reaches outside itself through.
pub struct Environment {
    /// Where in-process GPU inventory and telemetry come from.
    pub gpus: GpuProviders,
    pub prismd: Box<dyn PrismdConnector>,
    pub host: HostServices,
    pub settings: Box<dyn SettingsStore>,
    pub clock: Arc<dyn Clock>,
    pub sampling: Sampling,
}

impl Environment {
    /// prismd's socket, the real GPUs through NVML, the user's settings file and the system
    /// clock.
    pub fn production() -> Self {
        Self {
            gpus: GpuProviders::nvml(),
            prismd: Box::new(SocketConnector::production()),
            host: HostServices::default(),
            settings: Box::new(SettingsFile::in_user_config()),
            clock: Arc::new(SystemClock::default()),
            sampling: Sampling::Every(Duration::from_secs(1)),
        }
    }
}

/// The host's desktop services, such as the systemctl probe, notifications and the
/// single-instance bus. Each arrives with the screen or behavior that needs it.
pub struct HostServices {
    /// What the host says about prismd, and systemd enabling or restarting it.
    pub prismd: Box<dyn PrismdService>,
    /// Where later launches of Prism hand over to this one.
    pub launches: Box<dyn Launches>,
    /// How the window is raised and focused.
    pub activation: Box<dyn WindowActivation>,
    /// Where desktop notifications go.
    pub notifier: Box<dyn Notifier>,
    /// Where the user chooses a program file to add as a game.
    pub file_chooser: Box<dyn FileChooser>,
}

impl Default for HostServices {
    /// The host's systemd and os-release, no later launch reaches this Prism, the window is
    /// raised through the desktop, notifications go to the session's notification server, and
    /// program files are chosen through the desktop portal.
    fn default() -> Self {
        Self {
            prismd: Box::new(HostPrismdService::production()),
            launches: Box::new(NoLaunches),
            activation: Box::new(DesktopActivation),
            notifier: Box::new(DesktopNotifier::session()),
            file_chooser: Box::new(PortalFileChooser::session()),
        }
    }
}

/// Later launches of Prism, which hand over to the running one and exit.
pub trait Launches {
    /// Calls `launched` with each later launch's activation token, from any thread.
    fn listen(&self, launched: Box<dyn Fn(Option<String>) + Send + Sync>);
}

/// No later launch reaches this Prism, such as when there's no session bus.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoLaunches;

impl Launches for NoLaunches {
    fn listen(&self, _: Box<dyn Fn(Option<String>) + Send + Sync>) {}
}

/// Asks the desktop to focus the window, and hears when it gains or loses focus.
pub trait WindowActivation {
    /// Focuses `window`, which is already shown, with the activation token that lets the desktop
    /// do so, if there is one.
    fn activate(&self, window: &slint::Window, activation_token: Option<&str>);

    /// Calls `changed` on the UI thread whenever `window` gains or loses focus.
    fn watch_focus(&self, window: &slint::Window, changed: Box<dyn Fn(bool)>);
}

/// The GUI's monotonic clock.
pub trait Clock: Send + Sync {
    /// The time since Prism started.
    fn now(&self) -> Duration;

    /// The wall clock when Prism started, in Unix milliseconds, which Activity tells times from.
    fn started_unix_ms(&self) -> u64;
}

/// The system's monotonic clock, counting from when it was made.
#[derive(Debug, Clone, Copy)]
pub struct SystemClock {
    started: Instant,
    started_unix_ms: u64,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            started_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_millis().try_into().unwrap_or(u64::MAX)),
        }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.started.elapsed()
    }

    fn started_unix_ms(&self) -> u64 {
        self.started_unix_ms
    }
}

/// When the in-process sampler reads the GPUs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sampling {
    /// On its own thread, once every interval.
    Every(Duration),
    /// Only when [`App::sample_gpus`](crate::App::sample_gpus) is called while the core has
    /// sampling on, on the calling thread, so a test decides when each sample arrives.
    OnDemand,
}
