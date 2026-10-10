use std::time::Duration;

use prism_gpu::{GpuInventoryResponse, GpuPerformanceResponse};

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
}

/// A part of Prism the sidebar leads to, other than a GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Games,
    Drivers,
    Settings,
}

/// Something that happened, for the core to take in.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// Prism started, with the appearance mode loaded from its settings.
    Launched { appearance: AppearanceMode },
    /// The in-process GPU inventory was read.
    InventoryRead(GpuInventoryResponse),
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
    /// Start reading the GPUs in-process at 1 Hz.
    StartSampler,
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
    pub(crate) telemetry: Option<TelemetryState>,
    pub(crate) range: TelemetryRange,
    pub(crate) palette: Option<PaletteState>,
    /// Commands run lately, the latest first.
    pub(crate) recent: Vec<Command>,
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
                vec![Effect::StartSampler]
            }
            Input::InventoryRead(inventory) => {
                let gpus: Vec<_> = inventory
                    .gpus
                    .into_iter()
                    .map(|device| Gpu {
                        id: device.id,
                        name: device.name,
                        architecture: device.architecture,
                        bus_id: device.bus_id,
                        history: TelemetryHistory::default(),
                        latest: Latest::Waiting,
                    })
                    .collect();
                if self.selection.is_none() {
                    self.selection = gpus.first().map(|gpu| Selection::Gpu(gpu.id.clone()));
                }
                self.discovery = Discovery::Found {
                    gpus,
                    warnings: inventory.warnings,
                };
                Vec::new()
            }
            Input::TelemetrySampled {
                gpus: sample,
                source,
                received_at,
            } => {
                self.sampled(&sample, received_at);
                self.telemetry = Some(TelemetryState {
                    source,
                    received_at,
                });
                Vec::new()
            }
            Input::Intent(intent) => self.intent(intent),
            Input::LaunchedAgain { activation_token } => {
                vec![Effect::RaiseWindow { activation_token }]
            }
        }
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
                }
                None => {
                    gpu.history.record(received_at, Readings::default());
                    let reason = if sample.warnings.is_empty() {
                        "Missing from the latest reading".to_owned()
                    } else {
                        sample.warnings.join(" ")
                    };
                    gpu.latest = Latest::Missing { reason };
                }
            }
        }
    }

    fn intent(&mut self, intent: Intent) -> Vec<Effect> {
        match intent {
            Intent::SelectGpu(id) => self.perform(Command::GoToGpu(id)),
            Intent::GoTo(section) => self.perform(Command::GoTo(section)),
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
        self.ran(&command);
        self.perform(command)
    }

    fn perform(&mut self, command: Command) -> Vec<Effect> {
        match command {
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
            Command::Quit => vec![Effect::Quit],
        }
    }

    pub fn appearance(&self) -> AppearanceMode {
        self.appearance
    }

    /// Changes with every input, so the view can tell when to redraw what it derives.
    pub fn revision(&self) -> u64 {
        self.revision
    }
}
