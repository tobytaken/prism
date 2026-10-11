use std::time::Duration;

use prism_gpu::{
    ControlSupport, GpuInventoryResponse, GpuPerformanceDevice, GpuPerformanceResponse,
};
use prism_protocol::{DaemonEvent, DaemonRequest, DaemonSnapshot, Incompatible, Welcome};

use crate::daemon::{ServiceAction, ServiceOutcome};
use crate::host::{PrismdHost, PrismdInstall, UnitState};
use crate::palette::{Command, PaletteState, Shortcut};
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
}

/// Something the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Send this request on the session with prismd.
    SendToPrismd(DaemonRequest),
    /// Start reading the GPUs in-process at 1 Hz, or resume it.
    StartSampler,
    /// Pause the in-process sampler, keeping what it loaded.
    StopSampler,
    /// Show the window, unminimized and focused, passing on the activation token that lets the
    /// desktop focus it.
    RaiseWindow {
        activation_token: Option<String>,
    },
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
    pub(crate) name: String,
    pub(crate) architecture: Option<String>,
    pub(crate) bus_id: Option<String>,
    pub(crate) history: TelemetryHistory,
    pub(crate) latest: Latest,
    /// The controls the latest reading said the GPU supports.
    pub(crate) capabilities: Vec<TuningControl>,
}

/// What the latest telemetry sample said about a GPU.
#[derive(Debug, Default)]
pub(crate) enum Latest {
    #[default]
    Waiting,
    Read(Readings),
    /// The sample had no readings for this GPU.
    Missing {
        reason: String,
    },
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
                let mut effects = vec![Effect::SendToPrismd(DaemonRequest::SubscribeTelemetry)];
                if std::mem::take(&mut self.sampling) {
                    effects.push(Effect::StopSampler);
                }
                effects
            }
            Input::PrismdNotConnected(failure) => {
                let unavailable = matches!(failure, SessionFailure::Unavailable { .. });
                let was_unavailable = matches!(
                    self.session,
                    Session::Failed(SessionFailure::Unavailable { .. })
                );
                self.session = Session::Failed(failure);
                let mut effects = Vec::new();
                if !std::mem::replace(&mut self.sampling, true) {
                    effects.push(Effect::StartSampler);
                }
                if unavailable && !was_unavailable {
                    effects.push(Effect::CheckPrismdHost);
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
                        self.service_error = Some(Self::service_failed(action, &message));
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
                    // Taken in as the screens that show them land.
                    DaemonEvent::StoreHealthChanged(_)
                    | DaemonEvent::TuningStateChanged(_)
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
                    id: device.id,
                    name: device.name,
                    architecture: device.architecture,
                    bus_id: device.bus_id,
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
                    let readings = Readings::from(&device.telemetry);
                    gpu.history.record(received_at, readings);
                    gpu.latest = Latest::Read(readings);
                    gpu.capabilities = TuningControl::ALL
                        .into_iter()
                        .filter(|control| control.supported_by(device))
                        .collect();
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
        }
    }

    /// Runs a command from the palette or a shortcut, unless it can't run now.
    fn run(&mut self, command: Option<Command>) -> Vec<Effect> {
        let Some(command) = command else {
            return Vec::new();
        };
        if self.unavailable(&command).is_some() {
            return Vec::new();
        }
        self.record_run(&command);
        self.dismiss();
        self.perform(command)
    }

    fn perform(&mut self, command: Command) -> Vec<Effect> {
        let daemon_was_open = self.daemon_open();
        let effects = match command {
            Command::GoToGpu(id) => {
                if self.gpus().iter().any(|gpu| gpu.id == id) {
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
                self.service = Some(action);
                self.service_error = None;
                vec![Effect::RunService(action)]
            }
            Command::CopyPrismdSetup => {
                self.setup_command().map(Effect::Copy).into_iter().collect()
            }
            Command::Quit => vec![Effect::Quit],
        };
        // Settings → Daemon shows why there's no session, as the host says it now.
        if !daemon_was_open && self.daemon_open() && !self.connected() {
            return [effects, vec![Effect::CheckPrismdHost]].concat();
        }
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
