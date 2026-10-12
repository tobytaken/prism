use std::time::Duration;

use prism_gpu::{
    ControlSupport, GpuDevice, GpuInventoryResponse, GpuPerformanceDevice, GpuPerformanceResponse,
};
use std::collections::BTreeMap;

use prism_protocol::{
    DaemonEvent, DaemonRequest, DaemonResponse, DaemonSnapshot, GpuCoolingControl, GpuTuningState,
    Incompatible, ProfileSettingKey, ProfilesState, Welcome,
};

use crate::cooling::FanMode;
use crate::daemon::{ServiceAction, ServiceOutcome};
use crate::drafts::{DialogState, Drafts};
use crate::host::{PrismdHost, PrismdInstall, UnitState};
use crate::notifications::{Notification, NotificationId, Notifications, Subject};
use crate::palette::{Command, PaletteState, Shortcut};
use crate::profiles::ProfileApplying;
use crate::telemetry::{Readings, TelemetryHistory, TelemetryRange};

/// The user's choice of System, Light or Dark; System follows the desktop's preference.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AppearanceMode {
    #[default]
    System,
    Light,
    Dark,
}

/// Where a telemetry sample came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetrySource {
    /// Prism read the GPUs itself, because there is no prismd connection.
    InProcess,
    /// prismd read the GPUs and pushed the sample to this Prism's session.
    Prismd,
}

/// Whether this Prism is connected to prismd and, if not, why.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PrismdConnection {
    /// Prism has only just started trying.
    #[default]
    Connecting,
    /// A session is open, with what prismd said when it welcomed it.
    Connected(Welcome),
    NotConnected(NotConnected),
}

/// Why there is no session with prismd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotConnected {
    NotInstalled,
    /// Installed, but not enabled or stopped, or not answering yet.
    NotRunning,
    /// systemd gave up on prismd, with its `Result`, such as "exit-code" or "watchdog".
    Failed {
        result: String,
    },
    /// prismd speaks a protocol this Prism can't.
    Incompatible(Incompatible),
}

/// Why an attempt to open a session with prismd failed, or the session ended, as connecting saw
/// it. What the host says about prismd tells the reasons apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionFailure {
    /// prismd's socket isn't there, or nothing answered on it, as `message` says.
    Unavailable { message: String },
    /// prismd speaks a protocol this Prism can't.
    Incompatible(Incompatible),
}

/// The session as connecting last reported it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum Session {
    #[default]
    Connecting,
    Open {
        welcome: Welcome,
        /// The socket the session is on.
        socket: String,
    },
    Failed(SessionFailure),
}

impl PrismdConnection {
    /// The connection the session and what the host says about prismd make. A session overrides
    /// the host; without one, an unavailable prismd is told apart by the host, and taken for not
    /// running until the host has been checked.
    pub(crate) fn of(session: &Session, host: Option<&PrismdHost>) -> Self {
        let failure = match session {
            Session::Connecting => return Self::Connecting,
            Session::Open { welcome, .. } => return Self::Connected(welcome.clone()),
            Session::Failed(failure) => failure,
        };
        Self::NotConnected(match failure {
            SessionFailure::Incompatible(incompatible) => {
                NotConnected::Incompatible(incompatible.clone())
            }
            SessionFailure::Unavailable { .. } => match host.map(|host| &host.install) {
                Some(PrismdInstall::Native(UnitState::NotFound))
                | Some(PrismdInstall::Flatpak {
                    run_dir_visible: false,
                }) => NotConnected::NotInstalled,
                Some(PrismdInstall::Native(UnitState::Failed { result })) => NotConnected::Failed {
                    result: result.clone(),
                },
                _ => NotConnected::NotRunning,
            },
        })
    }

    /// The reason in short, such as "not running", or none with a session.
    pub fn reason(&self) -> Option<&'static str> {
        Some(match self {
            Self::Connected(_) => return None,
            Self::Connecting => "connecting",
            Self::NotConnected(NotConnected::NotInstalled) => "not installed",
            Self::NotConnected(NotConnected::NotRunning) => "not running",
            Self::NotConnected(NotConnected::Failed { .. }) => "failed",
            Self::NotConnected(NotConnected::Incompatible(_)) => "incompatible",
        })
    }
}

/// Whether this Prism can change GPU tuning at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuningAccess {
    Granted,
    /// Every control is read-only, for this short reason, such as "not running".
    ReadOnly {
        reason: String,
    },
}

impl TuningAccess {
    /// What the prismd connection allows. Tuning goes only through a session whose user prismd
    /// lets tune: there is no other write path, even for a Prism run as root (ADR 0010).
    pub fn of(connection: &PrismdConnection) -> Self {
        let reason = match connection {
            PrismdConnection::Connected(welcome) if welcome.can_tune => return Self::Granted,
            PrismdConnection::Connected(welcome) => {
                welcome.reason.as_deref().unwrap_or("not allowed")
            }
            other => other.reason().unwrap_or_default(),
        };
        Self::ReadOnly {
            reason: reason.to_owned(),
        }
    }

    /// Why tuning is read-only, or none with tuning access.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Granted => None,
            Self::ReadOnly { reason } => Some(reason),
        }
    }
}

/// A tuning control a GPU can offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuningControl {
    PowerLimit,
    GraphicsClockOffset,
    MemoryClockOffset,
    Cooling,
}

impl TuningControl {
    pub const ALL: [Self; 4] = [
        Self::PowerLimit,
        Self::GraphicsClockOffset,
        Self::MemoryClockOffset,
        Self::Cooling,
    ];

    /// Whether the GPU, as read, has the GPU control capability for this control.
    fn supported_by(self, gpu: &GpuPerformanceDevice) -> bool {
        match self {
            Self::PowerLimit => gpu.power.support == ControlSupport::Supported,
            Self::GraphicsClockOffset => gpu.clocks.graphics.support == ControlSupport::Supported,
            Self::MemoryClockOffset => gpu.clocks.memory.support == ControlSupport::Supported,
            Self::Cooling => gpu.cooling.is_supported,
        }
    }
}

/// A part of Prism the sidebar leads to, other than a GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Games,
    Drivers,
    Settings,
}

/// A section of Settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SettingsSection {
    #[default]
    Appearance,
    Startup,
    Notifications,
    Daemon,
    About,
}

impl SettingsSection {
    pub const ALL: [Self; 5] = [
        Self::Appearance,
        Self::Startup,
        Self::Notifications,
        Self::Daemon,
        Self::About,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Startup => "Startup",
            Self::Notifications => "Notifications",
            Self::Daemon => "Daemon",
            Self::About => "About",
        }
    }
}

/// How long a lost prismd connection has to stay lost before Prism notifies, so a daemon restart,
/// such as during an upgrade, doesn't.
const CONNECTION_LOST_GRACE: Duration = Duration::from_secs(10);

/// A one-shot timer the core starts with [`Effect::StartTimer`], for [`Input::TimerElapsed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Timer {
    /// The wait before notifying about a lost prismd connection.
    ConnectionLost,
}

/// Something that happened, for the core to take in.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// Prism started, with the appearance mode loaded from its settings.
    Launched { appearance: AppearanceMode },
    /// The in-process GPU inventory was read.
    InventoryRead(GpuInventoryResponse),
    /// A session with prismd opened on `socket`, starting from `snapshot`.
    PrismdConnected {
        welcome: Welcome,
        snapshot: DaemonSnapshot,
        socket: String,
    },
    /// There is no session with prismd: an attempt failed, or the session ended.
    PrismdNotConnected(SessionFailure),
    /// The host said this about prismd, when asked with [`Effect::CheckPrismdHost`].
    PrismdHostChecked(PrismdHost),
    /// A service action asked for with [`Effect::RunService`] ended.
    PrismdServiceDone {
        action: ServiceAction,
        outcome: ServiceOutcome,
    },
    /// prismd pushed an event to the session, which reached the UI thread at `received_at` on
    /// the GUI's monotonic clock.
    PrismdEvent {
        event: DaemonEvent,
        received_at: Duration,
    },
    /// Every GPU was read once, and the sample reached the UI thread at `received_at` on the
    /// GUI's monotonic clock.
    TelemetrySampled {
        gpus: GpuPerformanceResponse,
        source: TelemetrySource,
        received_at: Duration,
    },
    /// The user asked for something.
    Intent(Intent),
    /// Prism was launched again, and that launch handed over to this one, with the activation
    /// token it was started with, if any.
    LaunchedAgain { activation_token: Option<String> },
    /// Prism's window gained or lost focus.
    WindowFocusChanged(bool),
    /// A timer started with [`Effect::StartTimer`] ran out.
    TimerElapsed(Timer),
    /// The user clicked a notification, and the notification server handed over this activation
    /// token, if any.
    NotificationClicked {
        id: NotificationId,
        activation_token: Option<String>,
    },
    /// The notification server closed a notification: the user dismissed it, or it expired.
    NotificationClosed(NotificationId),
    /// prismd answered the command sent with [`Effect::CommandPrismd`] as `id`.
    PrismdAnswered {
        id: CommandId,
        response: DaemonResponse,
    },
}

/// Which command sent to prismd an answer belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CommandId(pub u64);

/// Something the user asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    /// Show this GPU, by id, in the workspace.
    SelectGpu(String),
    GoTo(Section),
    /// Open Settings at this section.
    GoToSettings(SettingsSection),
    /// Ask systemd to enable or restart prismd.
    RunService(ServiceAction),
    /// Put this text, such as a command to run, on the clipboard.
    Copy(String),
    CloseWindow,
    OpenPalette,
    /// Search the palette's commands for this query.
    FilterPalette(String),
    HighlightNext,
    HighlightPrevious,
    /// Run the palette's highlighted command.
    RunHighlighted,
    /// Run the palette's command at this index among its entries.
    RunPaletteEntry(usize),
    /// Close the topmost overlay.
    Dismiss,
    Press(Shortcut),
    /// Show this range in every track's chart and statistics.
    ShowRange(TelemetryRange),
    /// Open the hardware details, or collapse them again.
    ToggleHardwareDetails,
    /// Stage this value for the selected GPU's setting, such as a power limit in watts or a clock
    /// offset in MHz.
    Stage(ProfileSettingKey, i64),
    /// Stage an explicit request for the selected GPU's setting's default.
    UseDefault(ProfileSettingKey),
    /// Stage the power limit drawn at this height on the selected GPU's power chart, from the top
    /// (0) to the bottom (1), as dragging its line does.
    DragPowerLimit(f32),
    /// Stage this fan control mode for the fans the selected GPU's cooling controls set: every
    /// fan, or the one picked while each fan has its own settings.
    StageFanMode(FanMode),
    /// Stage this fixed speed, in percent, for the fans the selected GPU's cooling controls set.
    StageFanSpeed(i64),
    /// Show and set this fan of the selected GPU on its own, by index, which gives it its own
    /// settings once one changes; or, with none, every fan together, which stages one shared
    /// cooling configuration in place of per-fan settings.
    PickFans(Option<u32>),
    /// Stage the selected GPU's remembered fan curve, with its fan arrangement.
    RecallRememberedCurve,
    /// Stage the selected GPU's fan curve with this point moved to where it's drawn at `x` across
    /// and `y` down the fan curve editor, from 0 to 1, as dragging it does.
    DragCurvePoint {
        point: usize,
        x: f32,
        y: f32,
    },
    /// Stage the selected GPU's fan curve with this point moved by this many °C and %, as the
    /// arrow keys do.
    MoveCurvePoint {
        point: usize,
        temperature_by: i64,
        speed_by: i64,
    },
    /// Collapse the fan curve editor, or open it again.
    ToggleFanCurve,
    /// Apply this profile, by id, at once: unless its GPU has unapplied tuning changes, which
    /// Prism first asks to discard.
    ApplyProfile(String),
    /// Answer the open profile dialog: discard the GPU's unapplied tuning changes and apply the
    /// profile, or adjust the profile's out-of-range values and apply it.
    ConfirmProfile,
    /// Send the selected GPU's unapplied tuning changes to prismd as one Apply.
    ApplyChanges,
    /// Drop the selected GPU's unapplied tuning changes.
    DiscardChanges,
    /// Close Prism, dropping every GPU's unapplied tuning changes, as the close confirmation
    /// offers.
    DiscardAndClose,
}

/// Something the core asks the caller to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Start connecting to prismd, and keep reconnecting whenever there is no session.
    ConnectPrismd,
    /// Ask the host about prismd, for [`Input::PrismdHostChecked`]: systemd natively, or whether
    /// `/run/prism` is visible in Flatpak, and the distribution.
    CheckPrismdHost,
    /// Ask systemd to do this with prismd.service, under polkit, for
    /// [`Input::PrismdServiceDone`].
    RunService(ServiceAction),
    /// Put this text on the clipboard.
    Copy(String),
    /// Send this request on the session with prismd; its answer isn't needed.
    SendToPrismd(DaemonRequest),
    /// Send this request on the session with prismd, and hand its answer back as
    /// [`Input::PrismdAnswered`] with `id`. With no session it's dropped unanswered.
    CommandPrismd {
        id: CommandId,
        request: DaemonRequest,
    },
    /// Start reading the GPUs in-process at 1 Hz, or resume it.
    StartSampler,
    /// Pause the in-process sampler, keeping what it loaded.
    StopSampler,
    /// Show the window, unminimized and focused, passing on the activation token that lets the
    /// desktop focus it.
    RaiseWindow {
        activation_token: Option<String>,
    },
    /// Start this timer, or start it over if it's running.
    StartTimer {
        timer: Timer,
        after: Duration,
    },
    /// Stop this timer, if it's running.
    StopTimer(Timer),
    /// Send this desktop notification, in place of the one it replaces.
    Notify(Notification),
    /// Close this notification, if the user hasn't yet.
    WithdrawNotification(NotificationId),
    Quit,
}

/// Prism's UI state, changed only through [`Core::handle`].
#[derive(Debug, Default)]
pub struct Core {
    pub(crate) appearance: AppearanceMode,
    pub(crate) discovery: Discovery,
    pub(crate) selection: Option<Selection>,
    /// The section Settings shows, kept while Settings is closed.
    pub(crate) settings_section: SettingsSection,
    pub(crate) telemetry: Option<TelemetryState>,
    pub(crate) range: TelemetryRange,
    /// Whether the workspace's hardware details are open.
    pub(crate) hardware_open: bool,
    /// Whether the user collapsed the fan curve editor; it starts open.
    pub(crate) fan_curve_collapsed: bool,
    /// The fan the cooling controls show and set on their own, by index, until every fan is
    /// picked again.
    pub(crate) picked_fan: Option<u32>,
    pub(crate) palette: Option<PaletteState>,
    /// Commands run lately, the latest first.
    pub(crate) recent: Vec<Command>,
    pub(crate) session: Session,
    /// What the host last said about prismd, since the last session.
    pub(crate) host: Option<PrismdHost>,
    /// The service action running, while one is.
    pub(crate) service: Option<ServiceAction>,
    /// Why the last service action failed, until the next one or a session.
    pub(crate) service_error: Option<String>,
    /// Whether the in-process sampler is reading.
    sampling: bool,
    /// A session ended and hasn't come back, and Prism hasn't notified about it yet.
    lost_unnotified: bool,
    notifications: Notifications,
    /// Each GPU's tuning state as prismd last pushed it, by GPU id, while there is a session.
    pub(crate) tuning: BTreeMap<String, GpuTuningState>,
    /// Prism's control of each GPU's fans as prismd last pushed it, by GPU id, while there is a
    /// session.
    pub(crate) cooling: BTreeMap<String, GpuCoolingControl>,
    pub(crate) drafts: Drafts,
    /// The caller's profiles, with each GPU's startup and current profile, as prismd last pushed
    /// them, while there is a session.
    pub(crate) profiles: ProfilesState,
    /// The profile application waiting for prismd's answer, on each GPU that has one.
    pub(crate) profile_applications: BTreeMap<String, ProfileApplying>,
    /// What the latest profile application on each GPU put into effect and what it left, until
    /// the GPU is edited or another profile is chosen.
    pub(crate) profile_outcomes: BTreeMap<String, String>,
    /// The dialog over the window, while one is open.
    pub(crate) dialog: Option<DialogState>,
    revision: u64,
}

/// What Prism knows about which GPUs the host has.
#[derive(Debug, Default)]
pub(crate) enum Discovery {
    #[default]
    Searching,
    /// The inventory was read, with any warnings about GPUs it couldn't read.
    Found {
        gpus: Vec<Gpu>,
        warnings: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Selection {
    Gpu(String),
    Section(Section),
}

/// Where the latest telemetry sample came from, and when it arrived.
#[derive(Debug)]
pub(crate) struct TelemetryState {
    pub(crate) source: TelemetrySource,
    pub(crate) received_at: Duration,
}

/// A GPU found in the inventory, with its telemetry history.
#[derive(Debug)]
pub(crate) struct Gpu {
    pub(crate) id: String,
    /// What the inventory says about the GPU.
    pub(crate) device: GpuDevice,
    pub(crate) history: TelemetryHistory,
    pub(crate) latest: Latest,
    /// The controls the latest reading said the GPU supports.
    pub(crate) capabilities: Vec<TuningControl>,
}

impl Gpu {
    /// The values the GPU allows for a setting, as its latest reading or else its inventory says.
    pub(crate) fn range(&self, key: ProfileSettingKey) -> Option<(i64, i64)> {
        let device = match &self.latest {
            Latest::Read(device) => Some(device),
            Latest::Waiting | Latest::Missing { .. } => None,
        };
        let control = &self.device.control;
        let (read, inventory) = match key {
            ProfileSettingKey::PowerLimitWatts => {
                let power = device.map(|device| &device.power);
                let watts = |watts: Option<u32>| watts.map(i64::from);
                (
                    power.map(|power| (watts(power.min_limit_watts), watts(power.max_limit_watts))),
                    (
                        watts(control.power_limit.min_watts),
                        watts(control.power_limit.max_watts),
                    ),
                )
            }
            ProfileSettingKey::GraphicsOffsetMhz | ProfileSettingKey::MemoryOffsetMhz => {
                let graphics = key == ProfileSettingKey::GraphicsOffsetMhz;
                let offset = device.map(|device| {
                    if graphics {
                        &device.clocks.graphics
                    } else {
                        &device.clocks.memory
                    }
                });
                let mhz = |mhz: Option<i32>| mhz.map(i64::from);
                let offsets = &control.offsets;
                (
                    offset.map(|offset| (mhz(offset.min_offset_mhz), mhz(offset.max_offset_mhz))),
                    if graphics {
                        (mhz(offsets.min_graphics_mhz), mhz(offsets.max_graphics_mhz))
                    } else {
                        (mhz(offsets.min_memory_mhz), mhz(offsets.max_memory_mhz))
                    },
                )
            }
        };
        let (read_min, read_max) = read.unwrap_or_default();
        let min = read_min.or(inventory.0)?;
        let max = read_max.or(inventory.1)?;
        (min <= max).then_some((min, max))
    }

    /// What a default instruction for the setting resolves to, as far as Prism knows: the GPU's
    /// reported default power limit, or no offset (ADR 0001).
    pub(crate) fn default_of(&self, key: ProfileSettingKey) -> Option<i64> {
        match key {
            ProfileSettingKey::PowerLimitWatts => match &self.latest {
                Latest::Read(device) => device.power.default_limit_watts,
                Latest::Waiting | Latest::Missing { .. } => None,
            }
            .or(self.device.control.power_limit.default_watts)
            .map(i64::from),
            ProfileSettingKey::GraphicsOffsetMhz | ProfileSettingKey::MemoryOffsetMhz => Some(0),
        }
    }
}

/// What the latest telemetry sample said about a GPU.
#[derive(Debug, Default)]
pub(crate) enum Latest {
    #[default]
    Waiting,
    /// The GPU's readings, current tuning values and GPU control capabilities.
    Read(Box<GpuPerformanceDevice>),
    /// The sample had no readings for this GPU.
    Missing { reason: String },
}

impl Core {
    /// Takes in one input and returns the effects it calls for.
    pub fn handle(&mut self, input: Input) -> Vec<Effect> {
        self.revision += 1;
        match input {
            Input::Launched { appearance } => {
                self.appearance = appearance;
                // The sampler waits until there's no session, so NVML is never loaded while
                // prismd is there from the start.
                vec![Effect::ConnectPrismd]
            }
            Input::InventoryRead(inventory) => {
                // Read before a session opened; prismd's inventory is the one in use.
                if !self.connected() {
                    self.found(inventory);
                }
                Vec::new()
            }
            Input::PrismdConnected {
                welcome,
                snapshot,
                socket,
            } => {
                self.session = Session::Open { welcome, socket };
                // A session overrides whatever the host said.
                self.host = None;
                self.service_error = None;
                self.found(snapshot.inventory);
                self.tuning = snapshot
                    .tuning_states
                    .into_iter()
                    .map(|state| (state.gpu_id.clone(), state))
                    .collect();
                self.cooling = snapshot
                    .cooling_controls
                    .into_iter()
                    .map(|control| (control.gpu_id.clone(), control))
                    .collect();
                self.profiles = snapshot.profiles;
                let mut effects = vec![Effect::SendToPrismd(DaemonRequest::SubscribeTelemetry)];
                // The new session holds edits only once told.
                effects.extend(self.sync_edit_holds());
                if std::mem::take(&mut self.sampling) {
                    effects.push(Effect::StopSampler);
                }
                if std::mem::take(&mut self.lost_unnotified) {
                    effects.push(Effect::StopTimer(Timer::ConnectionLost));
                }
                // What it said no longer holds.
                if let Some(id) = self.notifications.withdraw(&Subject::Prismd) {
                    effects.push(Effect::WithdrawNotification(id));
                }
                effects
            }
            Input::PrismdNotConnected(failure) => {
                let unavailable = matches!(failure, SessionFailure::Unavailable { .. });
                let was_unavailable = matches!(
                    self.session,
                    Session::Failed(SessionFailure::Unavailable { .. })
                );
                let lost = self.connected();
                self.session = Session::Failed(failure);
                self.tuning.clear();
                self.cooling.clear();
                self.drafts_lost_session();
                self.profiles_lost_session();
                let mut effects = Vec::new();
                if !std::mem::replace(&mut self.sampling, true) {
                    effects.push(Effect::StartSampler);
                }
                if unavailable && !was_unavailable {
                    effects.push(Effect::CheckPrismdHost);
                }
                if lost {
                    // Only a connection still lost when this runs out is worth notifying about.
                    self.lost_unnotified = true;
                    effects.push(Effect::StartTimer {
                        timer: Timer::ConnectionLost,
                        after: CONNECTION_LOST_GRACE,
                    });
                }
                effects
            }
            Input::PrismdServiceDone { action, outcome } => {
                self.service = None;
                match outcome {
                    // Polkit's prompt was canceled: nothing changed, and nothing went wrong.
                    ServiceOutcome::Canceled => return Vec::new(),
                    ServiceOutcome::Done => self.service_error = None,
                    ServiceOutcome::Failed { message } => {
                        self.service_error = Some(action.failure(&message));
                    }
                }
                if self.connected() {
                    Vec::new()
                } else {
                    vec![Effect::CheckPrismdHost]
                }
            }
            Input::PrismdHostChecked(host) => {
                // A session overrides it, and it may be stale by the time the session ends.
                if !self.connected() {
                    self.host = Some(host);
                }
                Vec::new()
            }
            Input::PrismdEvent { event, received_at } => {
                match event {
                    DaemonEvent::InventoryChanged(inventory) => self.found(inventory),
                    DaemonEvent::TelemetrySampled(sample) => {
                        self.take_sample(&sample, TelemetrySource::Prismd, received_at)
                    }
                    DaemonEvent::TuningStateChanged(state) => {
                        let gpu_id = state.gpu_id.clone();
                        self.tuning.insert(gpu_id.clone(), state);
                        self.tuning_pushed(&gpu_id);
                    }
                    DaemonEvent::CoolingControlChanged(control) => {
                        self.cooling.insert(control.gpu_id.clone(), control);
                    }
                    DaemonEvent::ProfilesChanged(profiles) => self.profiles = profiles,
                    // Taken in as the screens that show them land.
                    DaemonEvent::StoreHealthChanged(_)
                    | DaemonEvent::RunningGamesChanged(_)
                    | DaemonEvent::AutomaticSwitchingChanged(_) => {}
                }
                Vec::new()
            }
            Input::TelemetrySampled {
                gpus: sample,
                source,
                received_at,
            } => {
                // The sampler may have read once more before it stopped.
                if !(source == TelemetrySource::InProcess && self.connected()) {
                    self.take_sample(&sample, source, received_at);
                }
                Vec::new()
            }
            Input::Intent(intent) => self.intent(intent),
            Input::LaunchedAgain { activation_token } => {
                vec![Effect::RaiseWindow { activation_token }]
            }
            Input::WindowFocusChanged(focused) => {
                self.notifications.focus_changed(focused);
                Vec::new()
            }
            Input::TimerElapsed(Timer::ConnectionLost) => {
                // It may have run out on its way in, just as the session came back.
                if !std::mem::take(&mut self.lost_unnotified) || self.connected() {
                    return Vec::new();
                }
                let body = format!(
                    "{}. Tuning is read-only until Prism reconnects.",
                    self.daemon_settings().title
                );
                self.notifications
                    .send(
                        Subject::Prismd,
                        "Prism lost its connection to prismd".into(),
                        body,
                    )
                    .map(Effect::Notify)
                    .into_iter()
                    .collect()
            }
            Input::NotificationClicked {
                id,
                activation_token,
            } => {
                let mut effects = match self.notifications.closed(id) {
                    Some(Subject::Prismd) => {
                        self.perform(Command::GoToSettings(SettingsSection::Daemon))
                    }
                    Some(Subject::Gpu(id)) => self.perform(Command::GoToGpu(id)),
                    // Withdrawn on its way in: the window is still what the user asked for.
                    None => Vec::new(),
                };
                effects.push(Effect::RaiseWindow { activation_token });
                effects
            }
            Input::NotificationClosed(id) => {
                self.notifications.closed(id);
                Vec::new()
            }
            Input::PrismdAnswered { id, response } => self.answered(id, response),
        }
    }

    pub(crate) fn connected(&self) -> bool {
        matches!(self.session, Session::Open { .. })
    }

    /// Takes in the GPUs an inventory lists. A GPU listed before keeps its telemetry history, so
    /// history stays one series when the source changes.
    fn found(&mut self, inventory: GpuInventoryResponse) {
        let mut known = match std::mem::take(&mut self.discovery) {
            Discovery::Found { gpus, .. } => gpus,
            Discovery::Searching => Vec::new(),
        };
        let gpus: Vec<_> = inventory
            .gpus
            .into_iter()
            .map(|device| {
                let (history, latest, capabilities) =
                    match known.iter().position(|gpu| gpu.id == device.id) {
                        Some(index) => {
                            let gpu = known.swap_remove(index);
                            (gpu.history, gpu.latest, gpu.capabilities)
                        }
                        None => (TelemetryHistory::default(), Latest::Waiting, Vec::new()),
                    };
                Gpu {
                    id: device.id.clone(),
                    device,
                    history,
                    latest,
                    capabilities,
                }
            })
            .collect();
        if self.selection.is_none() {
            self.selection = gpus.first().map(|gpu| Selection::Gpu(gpu.id.clone()));
        }
        self.discovery = Discovery::Found {
            gpus,
            warnings: inventory.warnings,
        };
    }

    fn take_sample(
        &mut self,
        sample: &GpuPerformanceResponse,
        source: TelemetrySource,
        received_at: Duration,
    ) {
        self.sampled(sample, received_at);
        self.telemetry = Some(TelemetryState {
            source,
            received_at,
        });
    }

    fn sampled(&mut self, sample: &GpuPerformanceResponse, received_at: Duration) {
        let Discovery::Found { gpus, .. } = &mut self.discovery else {
            return;
        };
        for gpu in gpus {
            match sample.gpus.iter().find(|device| device.id == gpu.id) {
                Some(device) => {
                    gpu.history
                        .record(received_at, Readings::from(&device.telemetry));
                    gpu.capabilities = TuningControl::ALL
                        .into_iter()
                        .filter(|control| control.supported_by(device))
                        .collect();
                    gpu.latest = Latest::Read(Box::new(device.clone()));
                }
                None => {
                    gpu.history.record(received_at, Readings::default());
                    let reason = if sample.warnings.is_empty() {
                        "Missing from the latest reading".to_owned()
                    } else {
                        sample.warnings.join(" ")
                    };
                    gpu.latest = Latest::Missing { reason };
                    gpu.capabilities.clear();
                }
            }
        }
    }

    fn intent(&mut self, intent: Intent) -> Vec<Effect> {
        match intent {
            Intent::SelectGpu(id) => self.perform(Command::GoToGpu(id)),
            Intent::GoTo(section) => self.perform(Command::GoTo(section)),
            Intent::GoToSettings(section) => self.perform(Command::GoToSettings(section)),
            Intent::RunService(action) => self.perform(Command::RunService(action)),
            Intent::Copy(text) => vec![Effect::Copy(text)],
            Intent::CloseWindow => self.perform(Command::Quit),
            Intent::OpenPalette | Intent::Press(Shortcut::OpenPalette) => {
                self.open_palette();
                Vec::new()
            }
            Intent::FilterPalette(query) => {
                self.filter_palette(query);
                Vec::new()
            }
            Intent::HighlightNext => {
                self.move_highlight(1);
                Vec::new()
            }
            Intent::HighlightPrevious => {
                self.move_highlight(-1);
                Vec::new()
            }
            Intent::RunHighlighted => self.run(self.palette_command(None)),
            Intent::RunPaletteEntry(index) => self.run(self.palette_command(Some(index))),
            Intent::Dismiss | Intent::Press(Shortcut::Dismiss) => {
                self.dismiss();
                Vec::new()
            }
            Intent::Press(shortcut) => self.run(self.shortcut_command(shortcut)),
            Intent::ShowRange(range) => self.perform(Command::ShowRange(range)),
            Intent::ToggleHardwareDetails => {
                self.hardware_open = !self.hardware_open;
                Vec::new()
            }
            Intent::Stage(key, value) => self.stage(key, value),
            Intent::UseDefault(key) => self.use_default(key),
            Intent::DragPowerLimit(y) => self.drag_power_limit(y),
            Intent::StageFanMode(mode) => self.stage_fan_mode(mode),
            Intent::StageFanSpeed(speed_percent) => self.stage_fan_speed(speed_percent),
            Intent::PickFans(index) => self.pick_fans(index),
            Intent::RecallRememberedCurve => self.perform(Command::RecallRememberedCurve),
            Intent::DragCurvePoint { point, x, y } => self.drag_curve_point(point, x, y),
            Intent::MoveCurvePoint {
                point,
                temperature_by,
                speed_by,
            } => self.move_curve_point(point, temperature_by, speed_by),
            Intent::ToggleFanCurve => self.toggle_fan_curve(),
            Intent::ApplyProfile(profile_id) => self.perform(Command::ApplyProfile(profile_id)),
            Intent::ConfirmProfile => self.confirm_profile(),
            Intent::ApplyChanges => self.perform(Command::ApplyChanges),
            Intent::DiscardChanges => self.perform(Command::DiscardChanges),
            Intent::DiscardAndClose => self.quit(),
        }
    }

    /// Runs a command from the palette or a shortcut, unless it can't run now.
    fn run(&mut self, command: Option<Command>) -> Vec<Effect> {
        let Some(command) = command else {
            return Vec::new();
        };
        if self.unavailable(&command).is_some() || self.blocked_by_dialog(&command) {
            return Vec::new();
        }
        self.record_run(&command);
        self.palette = None;
        self.perform(command)
    }

    fn perform(&mut self, command: Command) -> Vec<Effect> {
        let daemon_was_open = self.daemon_open();
        let effects = match command {
            Command::GoToGpu(id) => {
                if self.gpus().iter().any(|gpu| gpu.id == id) {
                    // Another GPU's controls start with every fan.
                    if self.selection.as_ref() != Some(&Selection::Gpu(id.clone())) {
                        self.picked_fan = None;
                    }
                    self.selection = Some(Selection::Gpu(id));
                }
                Vec::new()
            }
            Command::GoTo(section) => {
                self.selection = Some(Selection::Section(section));
                Vec::new()
            }
            Command::GoToSettings(section) => {
                self.selection = Some(Selection::Section(Section::Settings));
                self.settings_section = section;
                Vec::new()
            }
            Command::RunService(action) => {
                if self.service.is_some() || self.service_action() != Some(action) {
                    return Vec::new();
                }
                // Any earlier error stays until this one ends, since a canceled prompt changes
                // nothing.
                self.service = Some(action);
                vec![Effect::RunService(action)]
            }
            Command::ShowRange(range) => {
                self.range = range;
                Vec::new()
            }
            Command::ApplyChanges => self.apply_changes(),
            Command::DiscardChanges => self.discard_changes(),
            Command::RecallRememberedCurve => self.recall_remembered_curve(),
            Command::ApplyProfile(profile_id) => self.choose_profile(profile_id),
            Command::CopyPrismdSetup => {
                self.setup_command().map(Effect::Copy).into_iter().collect()
            }
            Command::Quit => self.close_or_ask(),
        };
        // Settings → Daemon shows why there's no session, as the host says it now.
        if !daemon_was_open && self.daemon_open() && !self.connected() {
            return [effects, vec![Effect::CheckPrismdHost]].concat();
        }
        effects
    }

    /// Quits, which ends the session and with it the edit holds of any drafts.
    pub(crate) fn quit(&mut self) -> Vec<Effect> {
        // Clicking one would lead nowhere once Prism is gone.
        let mut effects: Vec<_> = self
            .notifications
            .withdraw_all()
            .into_iter()
            .map(Effect::WithdrawNotification)
            .collect();
        effects.push(Effect::Quit);
        effects
    }

    /// Whether Settings → Daemon is showing.
    fn daemon_open(&self) -> bool {
        self.selection == Some(Selection::Section(Section::Settings))
            && self.settings_section == SettingsSection::Daemon
    }

    pub fn prismd(&self) -> PrismdConnection {
        PrismdConnection::of(&self.session, self.host.as_ref())
    }

    pub fn tuning_access(&self) -> TuningAccess {
        TuningAccess::of(&self.prismd())
    }

    /// Whether the GPU's control can be changed: only with tuning access and the GPU control
    /// capability for it.
    pub fn editable(&self, gpu_id: &str, control: TuningControl) -> bool {
        self.tuning_access().reason().is_none()
            && self
                .gpus()
                .iter()
                .any(|gpu| gpu.id == gpu_id && gpu.capabilities.contains(&control))
    }

    /// The section Settings shows when it's open.
    pub fn settings_section(&self) -> SettingsSection {
        self.settings_section
    }

    pub fn appearance(&self) -> AppearanceMode {
        self.appearance
    }

    /// Changes with every input, so the view can tell when to redraw what it derives.
    pub fn revision(&self) -> u64 {
        self.revision
    }
}
