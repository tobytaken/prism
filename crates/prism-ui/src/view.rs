//! What the view shows, derived from the core's state.

use crate::core::{Core, Discovery, Gpu, Latest, Section, Selection, TelemetrySource};
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
    pub selected: bool,
}

impl SidebarGpu {
    /// The vitals as a screen reader says them.
    pub fn vitals(&self) -> String {
        format!(
            "temperature {}, utilization {}, power draw {}, fan speed {}",
            self.temperature.spoken(),
            self.utilization.spoken(),
            self.power_draw.spoken(),
            self.fan_speed.spoken()
        )
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
            connection: "PRISMD NOT CONNECTED".into(),
            telemetry: if self.no_gpus().is_some() {
                "NO GPU TELEMETRY"
            } else {
                match self.telemetry.as_ref().map(|telemetry| telemetry.source) {
                    None => "WAITING FOR TELEMETRY",
                    Some(TelemetrySource::InProcess) => "MONITORING IN-PROCESS",
                }
            }
            .into(),
        }
    }

    fn gpus(&self) -> &[Gpu] {
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
        let index_label = format!("GPU {index}");
        let name = short_gpu_name(&gpu.name).to_owned();
        let identity = gpu.bus_id.as_deref().map(short_bus_id).unwrap_or_default();
        SidebarGpu {
            id: gpu.id.clone(),
            label: [index_label.as_str(), &name, identity]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · "),
            index: index_label,
            name,
            identity: identity.to_owned(),
            temperature: Readout::celsius(latest.temperature_c),
            utilization: Readout::percent(latest.utilization_percent),
            power_draw: Readout::watts(latest.power_draw_watts),
            fan_speed: Readout::percent(latest.fan_speed_percent),
            selected: self.selection == Some(Selection::Gpu(gpu.id.clone())),
        }
    }

    fn workspace(&self, index: usize, gpu: &Gpu) -> GpuWorkspace {
        let index_label = format!("GPU {index}");
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
            unknown_reason: match (&gpu.latest, celsius) {
                (Latest::Waiting, _) => Some("Waiting for the first reading".into()),
                (Latest::Missing { reason }, _) => Some(reason.clone()),
                (Latest::Read(_), None) => Some("Not reported by this GPU".into()),
                (Latest::Read(_), Some(_)) => None,
            },
            range: self.range.label(),
            max: Readout::celsius(stats.map(|stats| stats.max)),
            avg: Readout::celsius(stats.map(|stats| stats.avg.round() as u32)),
            min: Readout::celsius(stats.map(|stats| stats.min)),
            chart: gpu.history.chart(metric, self.range, now),
        }
    }
}

fn latest_readings(gpu: &Gpu) -> Readings {
    match gpu.latest {
        Latest::Read(readings) => readings,
        Latest::Waiting | Latest::Missing { .. } => Readings::default(),
    }
}
