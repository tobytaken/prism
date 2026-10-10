//! What the composition root is wired to. Production wires the real parts; tests and the
//! simulator wire fakes.

use std::sync::Arc;
use std::time::{Duration, Instant};

use prism_gpu::GpuProviders;

use crate::activation::DesktopActivation;
use crate::settings::{SettingsFile, SettingsStore};

/// Everything [`App`](crate::App) reaches outside itself through.
pub struct Environment {
    /// Where in-process GPU inventory and telemetry come from.
    pub gpus: GpuProviders,
    pub prismd: PrismdConnector,
    pub host: HostServices,
    pub settings: Box<dyn SettingsStore>,
    pub clock: Arc<dyn Clock>,
    pub sampling: Sampling,
}

impl Environment {
    /// The real GPUs through NVML, the user's settings file and the system clock.
    pub fn production() -> Self {
        Self {
            gpus: GpuProviders::nvml(),
            prismd: PrismdConnector::Placeholder,
            host: HostServices::default(),
            settings: Box::new(SettingsFile::in_user_config()),
            clock: Arc::new(SystemClock::default()),
            sampling: Sampling::Every(Duration::from_secs(1)),
        }
    }
}

/// How Prism reaches prismd.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PrismdConnector {
    /// Never connects, so Prism always monitors in-process. The real connector arrives with the
    /// GUI connection (#120, step 7).
    #[default]
    Placeholder,
}

/// The host's desktop services, such as the systemctl probe, notifications and the
/// single-instance bus. Each arrives with the screen or behavior that needs it.
pub struct HostServices {
    /// Where later launches of Prism hand over to this one.
    pub launches: Box<dyn Launches>,
    /// How the window is raised and focused.
    pub activation: Box<dyn WindowActivation>,
}

impl Default for HostServices {
    /// No later launch reaches this Prism, and the window is raised through the desktop.
    fn default() -> Self {
        Self {
            launches: Box::new(NoLaunches),
            activation: Box::new(DesktopActivation),
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

/// Asks the desktop to focus the window.
pub trait WindowActivation {
    /// Focuses `window`, which is already shown, with the activation token that lets the desktop
    /// do so, if there is one.
    fn activate(&self, window: &slint::Window, activation_token: Option<&str>);
}

/// The GUI's monotonic clock.
pub trait Clock: Send + Sync {
    /// The time since Prism started.
    fn now(&self) -> Duration;
}

/// The system's monotonic clock, counting from when it was made.
#[derive(Debug, Clone, Copy)]
pub struct SystemClock {
    started: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.started.elapsed()
    }
}

/// When the in-process sampler reads the GPUs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sampling {
    /// On its own thread, once every interval.
    Every(Duration),
    /// Only when [`App::sample_gpus`](crate::App::sample_gpus) is called, on the calling thread,
    /// so a test decides when each sample arrives.
    OnDemand,
}
