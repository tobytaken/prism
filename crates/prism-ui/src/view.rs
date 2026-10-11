//! What the view shows, derived from the core's state.

use crate::core::{
    Core, Discovery, Gpu, Latest, PrismdConnection, Section, Selection, TelemetrySource,
};
use crate::format::{Readout, short_bus_id, short_gpu_name};
use crate::telemetry::{Chart, Metric, Readings};

/// The sidebar: a block per GPU, then the section rows.
#[derive(Debug, Clone, PartialEq)]
pub struct Sidebar {
    pub gpus: Vec<SidebarGpu>,
    /// Why there are no GPU blocks, once the inventory has been read and has none.
    pub no_gpus: Option<ExplainedState>,
    pub section: Option<Section>,
}

/// A GPU's block in the sidebar, with its live vitals.
#[derive(Debug, Clone, PartialEq)]
pub struct SidebarGpu {
    pub id: String,
    /// The GPU's position in the inventory, such as "GPU 0".
    pub index: String,
    pub name: String,
    /// What tells this GPU apart from another of the same model: its bus id.
    pub identity: String,
    /// Index, name and identity together, such as "GPU 0 · RTX 5090 · 01:00.0".
    pub label: String,
    pub temperature: Readout,
    pub utilization: Readout,
    pub power_draw: Readout,
    pub fan_speed: Readout,
    /// Why some of the vitals are unknown, when they are.
    pub unknown_reason: Option<String>,
    pub selected: bool,
}

impl SidebarGpu {
    /// The vitals as a screen reader says them, with why any are unknown.
    pub fn vitals(&self) -> String {
        let vitals = format!(
            "temperature {}, utilization {}, power draw {}, fan speed {}",
            self.temperature.spoken(),
            self.utilization.spoken(),
            self.power_draw.spoken(),
            self.fan_speed.spoken()
        );
        match &self.unknown_reason {
            Some(reason) => format!("{vitals}. {reason}"),
            None => vitals,
        }
    }
}

/// What fills the content area.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    /// Prism hasn't read the GPU inventory yet.
    Searching,
    /// The inventory has no GPUs, for the reason given.
    NoGpus(ExplainedState),
    Gpu(Box<GpuWorkspace>),
    Section(Section),
}

/// The selected GPU's workspace.
#[derive(Debug, Clone, PartialEq)]
pub struct GpuWorkspace {
    pub id: String,
    pub title: String,
    /// Index, bus id and architecture, such as "GPU 0 · 01:00.0 · Blackwell".
    pub subtitle: String,
    /// Why there's no tuning access, for the inline "Tuning needs prismd" line; none with it.
    pub no_tuning_access: Option<String>,
    pub temperature: TemperatureTrack,
}

/// The workspace's Temperature track: the current reading, its chart and range statistics.
#[derive(Debug, Clone, PartialEq)]
pub struct TemperatureTrack {
    pub readout: Readout,
    /// Why the reading is unknown, when it is.
    pub unknown_reason: Option<String>,
    /// The range the chart and statistics cover, such as "5 m".
    pub range: &'static str,
    pub max: Readout,
    pub avg: Readout,
    pub min: Readout,
    pub chart: Chart,
}

/// A state with nothing to show, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplainedState {
    pub title: String,
    pub reason: String,
    pub hint: String,
}

/// The status bar's items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusBar {
    pub connection: String,
    /// Whether there is a session with prismd.
    pub connected: bool,
    pub telemetry: String,
}

impl Core {
    pub fn sidebar(&self) -> Sidebar {
        Sidebar {
            gpus: self
                .gpus()
                .iter()
                .enumerate()
                .map(|(index, gpu)| self.sidebar_gpu(index, gpu))
                .collect(),
            no_gpus: self.no_gpus(),
            section: match &self.selection {
                Some(Selection::Section(section)) => Some(*section),
                _ => None,
            },
        }
    }

    pub fn content(&self) -> Content {
        match &self.selection {
            Some(Selection::Section(section)) => Content::Section(*section),
            Some(Selection::Gpu(id)) => match self.gpus().iter().position(|gpu| &gpu.id == id) {
                Some(index) => Content::Gpu(Box::new(self.workspace(index, &self.gpus()[index]))),
                None => Content::Searching,
            },
            None => self.no_gpus().map_or(Content::Searching, Content::NoGpus),
        }
    }

    pub fn status_bar(&self) -> StatusBar {
        StatusBar {
            connection: self.connection_item(),
            connected: self.connected(),
            telemetry: if self.no_gpus().is_some() {
                "NO GPU TELEMETRY"
            } else {
                match self.telemetry.as_ref().map(|telemetry| telemetry.source) {
                    None => "WAITING FOR TELEMETRY",
                    Some(TelemetrySource::InProcess) => "MONITORING IN-PROCESS",
                    Some(TelemetrySource::Prismd) => "MONITORING THROUGH PRISMD",
                }
            }
            .into(),
        }
    }

    /// The status bar's prismd item: "PRISMD · " and the reason, or that it's connected.
    fn connection_item(&self) -> String {
        let state = match self.prismd() {
            PrismdConnection::Connected(welcome) if !welcome.can_tune => "not allowed",
            PrismdConnection::Connected(_) => "connected",
            other => other.reason().unwrap_or_default(),
        };
        format!("PRISMD · {}", state.to_uppercase())
    }

    pub(crate) fn gpus(&self) -> &[Gpu] {
        match &self.discovery {
            Discovery::Found { gpus, .. } => gpus,
            Discovery::Searching => &[],
        }
    }

    fn no_gpus(&self) -> Option<ExplainedState> {
        let Discovery::Found { gpus, warnings } = &self.discovery else {
            return None;
        };
        if !gpus.is_empty() {
            return None;
        }
        Some(if warnings.is_empty() {
            ExplainedState {
                title: "No GPUs found".into(),
                reason: "NVML reports no NVIDIA GPUs on this host.".into(),
                hint: "An empty list alone doesn't mean the host has no GPU. Check that the \
                       NVIDIA driver is loaded, then restart Prism."
                    .into(),
            }
        } else {
            ExplainedState {
                title: "No GPU telemetry".into(),
                reason: warnings.join(" "),
                hint: "Prism reads NVIDIA GPUs through NVML, which comes with the NVIDIA driver. \
                       Check that the driver is installed and loaded, then restart Prism."
                    .into(),
            }
        })
    }

    fn sidebar_gpu(&self, index: usize, gpu: &Gpu) -> SidebarGpu {
        let latest = latest_readings(gpu);
        SidebarGpu {
            id: gpu.id.clone(),
            label: gpu_label(index, gpu),
            index: index_label(index),
            name: short_gpu_name(&gpu.name).to_owned(),
            identity: gpu
                .bus_id
                .as_deref()
                .map(short_bus_id)
                .unwrap_or_default()
                .to_owned(),
            temperature: Readout::celsius(latest.temperature_c),
            utilization: Readout::percent(latest.utilization_percent),
            power_draw: Readout::watts(latest.power_draw_watts),
            fan_speed: Readout::percent(latest.fan_speed_percent),
            unknown_reason: unknown_reason(gpu, &Metric::ALL),
            selected: self.selection == Some(Selection::Gpu(gpu.id.clone())),
        }
    }

    fn workspace(&self, index: usize, gpu: &Gpu) -> GpuWorkspace {
        let index_label = index_label(index);
        let subtitle = [
            Some(index_label.as_str()),
            gpu.bus_id.as_deref().map(short_bus_id),
            gpu.architecture.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        GpuWorkspace {
            id: gpu.id.clone(),
            title: gpu.name.clone(),
            subtitle,
            no_tuning_access: self.tuning_access().reason().map(str::to_owned),
            temperature: self.temperature_track(gpu),
        }
    }

    fn temperature_track(&self, gpu: &Gpu) -> TemperatureTrack {
        let now = self
            .telemetry
            .as_ref()
            .map(|telemetry| telemetry.received_at)
            .unwrap_or_default();
        let metric = Metric::Temperature;
        let stats = gpu.history.stats(metric, self.range, now);
        let celsius = latest_readings(gpu).temperature_c;
        TemperatureTrack {
            readout: Readout::celsius(celsius),
            unknown_reason: unknown_reason(gpu, &[metric]),
            range: self.range.label(),
            max: Readout::celsius(stats.map(|stats| stats.max)),
            avg: Readout::celsius(stats.map(|stats| stats.avg.round() as u32)),
            min: Readout::celsius(stats.map(|stats| stats.min)),
            chart: gpu.history.chart(metric, self.range, now),
        }
    }
}

/// Index, name and identity together, such as "GPU 0 · RTX 5090 · 01:00.0".
pub(crate) fn gpu_label(index: usize, gpu: &Gpu) -> String {
    let index = index_label(index);
    let identity = gpu.bus_id.as_deref().map(short_bus_id).unwrap_or_default();
    [index.as_str(), short_gpu_name(&gpu.name), identity]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A GPU's position in the inventory, such as "GPU 0".
fn index_label(index: usize) -> String {
    format!("GPU {index}")
}

/// Why any of `metrics` is unknown in the GPU's latest reading. A track names no metric, since it
/// shows one; the sidebar names the ones it lacks.
fn unknown_reason(gpu: &Gpu, metrics: &[Metric]) -> Option<String> {
    let readings = match &gpu.latest {
        Latest::Waiting => return Some("Waiting for the first reading".into()),
        Latest::Missing { reason } => return Some(reason.clone()),
        Latest::Read(readings) => readings,
    };
    let unknown: Vec<_> = metrics
        .iter()
        .filter(|metric| readings.get(**metric).is_none())
        .map(|metric| metric.name())
        .collect();
    match unknown.as_slice() {
        [] => None,
        [_] if metrics.len() == 1 => Some("Not reported by this GPU".into()),
        names => {
            let names = names.join(", ");
            let mut first = names.chars();
            let capitalized = first
                .next()
                .map(|letter| letter.to_uppercase().chain(first).collect::<String>())
                .unwrap_or_default();
            Some(format!("{capitalized}: not reported by this GPU"))
        }
    }
}

fn latest_readings(gpu: &Gpu) -> Readings {
    match gpu.latest {
        Latest::Read(readings) => readings,
        Latest::Waiting | Latest::Missing { .. } => Readings::default(),
    }
}
