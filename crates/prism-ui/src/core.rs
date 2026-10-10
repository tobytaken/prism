use std::time::Duration;

use prism_gpu::{GpuInventoryResponse, GpuPerformanceResponse};

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
}

/// Something the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// Show this GPU, by id, in the workspace.
    SelectGpu(String),
    GoTo(Section),
    CloseWindow,
}

/// Something the core asks the caller to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Start reading the GPUs in-process at 1 Hz.
    StartSampler,
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
            Intent::SelectGpu(id) => {
                let Discovery::Found { gpus, .. } = &self.discovery else {
                    return Vec::new();
                };
                if gpus.iter().any(|gpu| gpu.id == id) {
                    self.selection = Some(Selection::Gpu(id));
                }
                Vec::new()
            }
            Intent::GoTo(section) => {
                self.selection = Some(Selection::Section(section));
                Vec::new()
            }
            Intent::CloseWindow => vec![Effect::Quit],
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
