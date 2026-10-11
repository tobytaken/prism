//! What the view shows, derived from the core's state.

use std::time::Duration;

use prism_gpu::{ComputeApi, GpuDevice, GpuPerformanceDevice};
use prism_protocol::ProfileSetting;

use crate::core::{
    Core, Discovery, Gpu, Latest, PrismdConnection, Section, Selection, TelemetrySource,
    TuningControl,
};
use crate::daemon::Fact;
use crate::format::{Readout, UNKNOWN, pending, short_bus_id, short_gpu_name, signed};
use crate::palette::Command;
use crate::telemetry::{Chart, LimitKind, LimitLine, Metric, Readings, TelemetryRange};

/// The readings the sidebar shows for each GPU.
const VITALS: [Metric; 4] = [
    Metric::Temperature,
    Metric::Utilization,
    Metric::PowerDraw,
    Metric::FanSpeed,
];

const WAITING: &str = "Waiting for the first reading";
const NOT_REPORTED: &str = "Not reported by this GPU";

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
    /// The fixed strip under the header with the GPU's unapplied tuning changes.
    pub changes: ChangeStrip,
    /// Why there's no tuning access, for the inline "Tuning needs prismd" line; none with it.
    pub no_tuning_access: Option<String>,
    /// The range the range picker has picked, which every track shows.
    pub range: TelemetryRange,
    /// One track per metric, top to bottom in [`TrackKind::ALL`]'s order.
    pub tracks: Vec<Track>,
    pub hardware: HardwareDetails,
}

impl GpuWorkspace {
    /// The track of this kind.
    pub fn track(&self, kind: TrackKind) -> &Track {
        self.tracks
            .iter()
            .find(|track| track.kind == kind)
            .expect("the workspace has every track")
    }
}

/// The fixed strip under the workspace's header: "No unapplied changes", or how many there are,
/// with Discard and Apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeStrip {
    /// Such as "No unapplied changes" or "2 unapplied changes".
    pub summary: String,
    /// Whether the GPU has unapplied tuning changes.
    pub pending: bool,
    /// Why the last Apply left changes unapplied, until the next edit.
    pub problem: Option<String>,
    /// Why Apply can't run now, while there are changes it can't apply.
    pub apply_unavailable: Option<String>,
    /// Why Discard can't run now, while there are changes it can't discard.
    pub discard_unavailable: Option<String>,
}

/// A full-width track of the workspace, one per metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Temperature,
    Utilization,
    /// Power draw, with the configured and enforced power limits.
    Power,
    /// The measured graphics and memory clocks, with the clock offsets.
    Clocks,
    /// Fan speed, with each fan's own speed.
    Fans,
}

impl TrackKind {
    /// Every track, top to bottom.
    pub const ALL: [Self; 5] = [
        Self::Temperature,
        Self::Utilization,
        Self::Power,
        Self::Clocks,
        Self::Fans,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Temperature => "Temperature",
            Self::Utilization => "Utilization",
            Self::Power => "Power",
            Self::Clocks => "Clocks",
            Self::Fans => "Fans",
        }
    }

    /// The metric the track's readout and statistics show.
    pub fn metric(self) -> Metric {
        match self {
            Self::Temperature => Metric::Temperature,
            Self::Utilization => Metric::Utilization,
            Self::Power => Metric::PowerDraw,
            Self::Clocks => Metric::GraphicsClock,
            Self::Fans => Metric::FanSpeed,
        }
    }
}

/// A track: its current reading and details on the left, its charts in the middle, and the
/// max/avg/min of its metric over the range on the right.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub kind: TrackKind,
    pub readout: Readout,
    /// Why the reading is unknown, when it is.
    pub unknown_reason: Option<String>,
    /// Further current values under the readout, such as the power limits.
    pub details: Vec<Detail>,
    /// The range the charts and statistics cover, such as "5 m".
    pub range: &'static str,
    pub max: Readout,
    pub avg: Readout,
    pub min: Readout,
    /// Side by side, left to right.
    pub charts: Vec<TrackChart>,
    /// The control that tunes what the track shows, such as the power limit, if it has one.
    pub stepper: Option<Stepper>,
    /// Whether the fan curve belongs to this track.
    pub holds_fan_curve: bool,
}

impl Track {
    /// What a screen reader calls the readout, such as "Power draw reading".
    pub fn readout_label(&self) -> String {
        format!("{} reading", capitalized(self.kind.metric().name()))
    }

    /// What a screen reader calls the max, avg and min, such as "Maximum power draw".
    pub fn statistic_labels(&self) -> [String; 3] {
        let metric = self.kind.metric().name();
        ["Maximum", "Average", "Minimum"].map(|statistic| format!("{statistic} {metric}"))
    }
}

/// A tuning value stepped through its allowed range, such as the power limit, with the value in
/// effect and any pending one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stepper {
    /// What a screen reader calls it, such as "Power limit".
    pub name: String,
    /// The value it steps from: the pending one, or else the one in effect.
    pub value: i64,
    /// That value as a screen reader says it, such as "450 W".
    pub value_text: String,
    pub min: i64,
    pub max: i64,
    pub step: i64,
    /// The value in effect, as shown.
    pub readout: Readout,
    /// The pending value as shown next to the control, such as "→ 450 W", while one is staged.
    pub pending: Option<String>,
    /// Why it can't be changed, when it can't.
    pub read_only: Option<String>,
}

/// A current value a track shows under its readout, such as the power limit.
#[derive(Debug, Clone, PartialEq)]
pub struct Detail {
    /// The label as shown, such as "limit".
    pub label: String,
    /// What a screen reader calls it, such as "Power limit".
    pub name: String,
    pub readout: Readout,
    /// Why the value is unknown, when it is.
    pub unknown_reason: Option<String>,
    /// The limit it is, which is drawn across the chart, when it's one.
    pub limit: Option<LimitKind>,
}

/// One of a track's charts: a metric's readings over the range, and any limits across it.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackChart {
    pub metric: Metric,
    /// What tells it apart from the track's other chart, such as "MEMORY".
    pub label: Option<&'static str>,
    /// Whether the area under the series is filled, as utilization's is.
    pub area: bool,
    pub chart: Chart,
    pub limits: Vec<LimitLine>,
}

/// The workspace's hardware details: the GPU inventory's entry for the GPU and its GPU control
/// capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HardwareDetails {
    /// Whether they're open; they start collapsed.
    pub open: bool,
    pub inventory: Vec<Fact>,
    pub capabilities: Vec<Fact>,
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
    /// Whether any GPU has unapplied tuning changes, which hold automatic changes: EDIT HOLD.
    pub edit_hold: bool,
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
            edit_hold: self.edit_hold(),
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
            name: short_gpu_name(&gpu.device.name).to_owned(),
            identity: gpu
                .device
                .bus_id
                .as_deref()
                .map(short_bus_id)
                .unwrap_or_default()
                .to_owned(),
            temperature: Readout::celsius(latest.temperature_c),
            utilization: Readout::percent(latest.utilization_percent),
            power_draw: Readout::watts(latest.power_draw_watts),
            fan_speed: Readout::percent(latest.fan_speed_percent),
            unknown_reason: unknown_reason(gpu, &VITALS),
            selected: self.selection == Some(Selection::Gpu(gpu.id.clone())),
        }
    }

    fn workspace(&self, index: usize, gpu: &Gpu) -> GpuWorkspace {
        let index_label = index_label(index);
        let subtitle = [
            Some(index_label.as_str()),
            gpu.device.bus_id.as_deref().map(short_bus_id),
            gpu.device.architecture.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        GpuWorkspace {
            id: gpu.id.clone(),
            title: gpu.device.name.clone(),
            subtitle,
            changes: self.change_strip(),
            no_tuning_access: self.tuning_access().reason().map(str::to_owned),
            range: self.range,
            tracks: TrackKind::ALL
                .into_iter()
                .map(|kind| self.track(gpu, kind))
                .collect(),
            hardware: HardwareDetails {
                open: self.hardware_open,
                inventory: inventory_facts(&gpu.device),
                capabilities: capability_facts(gpu),
            },
        }
    }

    /// When the latest sample arrived, which the ranges look back from.
    fn now(&self) -> Duration {
        self.telemetry
            .as_ref()
            .map(|telemetry| telemetry.received_at)
            .unwrap_or_default()
    }

    fn track(&self, gpu: &Gpu, kind: TrackKind) -> Track {
        let metric = kind.metric();
        let now = self.now();
        let chart = |metric, label, also_fit: &[Option<u32>]| TrackChart {
            metric,
            label,
            area: metric == Metric::Utilization,
            chart: gpu.history.chart(
                metric,
                self.range,
                now,
                &also_fit.iter().flatten().copied().collect::<Vec<_>>(),
            ),
            limits: Vec::new(),
        };
        let (details, charts) = match kind {
            TrackKind::Temperature | TrackKind::Utilization => {
                (Vec::new(), vec![chart(metric, None, &[])])
            }
            TrackKind::Power => {
                let power = latest_device(gpu).map(|device| &device.power);
                let configured = power.and_then(|power| power.current_limit_watts);
                let enforced = power.and_then(|power| power.enforced_limit_watts);
                let staged = self.staged_power_limit(gpu);
                let mut draw = TrackChart {
                    metric,
                    label: None,
                    area: false,
                    chart: self.power_chart(gpu),
                    limits: Vec::new(),
                };
                draw.limits = [
                    (LimitKind::Configured, configured),
                    (LimitKind::Enforced, enforced),
                    (LimitKind::Pending, staged),
                ]
                .into_iter()
                .filter_map(|(kind, watts)| {
                    watts.map(|watts| LimitLine {
                        kind,
                        y: draw.chart.y(watts),
                    })
                })
                .collect();
                let said = power.and_then(|power| {
                    gpu_said(
                        power.current_value_error.as_deref(),
                        power.reason.as_deref(),
                    )
                });
                let limit = |kind, label: &str, name: &str, watts| Detail {
                    label: label.into(),
                    name: name.into(),
                    readout: Readout::watts(watts),
                    unknown_reason: value_reason(gpu, watts.is_some(), said),
                    limit: Some(kind),
                };
                (
                    vec![
                        limit(LimitKind::Configured, "limit", "Power limit", configured),
                        limit(
                            LimitKind::Enforced,
                            "enforced",
                            "Enforced power limit",
                            enforced,
                        ),
                    ],
                    vec![draw],
                )
            }
            TrackKind::Clocks => {
                let memory = latest_readings(gpu).memory_clock_mhz;
                let clocks = latest_device(gpu).map(|device| &device.clocks);
                let offset =
                    |label: &str, name: &str, offset: Option<&prism_gpu::ClockOffsetInfo>| {
                        let mhz = offset.and_then(|offset| offset.current_offset_mhz);
                        let said = offset.and_then(|offset| {
                            gpu_said(
                                offset.current_value_error.as_deref(),
                                offset.reason.as_deref(),
                            )
                        });
                        Detail {
                            label: label.into(),
                            name: name.into(),
                            readout: Readout::offset_megahertz(mhz),
                            unknown_reason: value_reason(gpu, mhz.is_some(), said),
                            limit: None,
                        }
                    };
                (
                    vec![
                        Detail {
                            label: "memory".into(),
                            name: "Memory clock".into(),
                            readout: Readout::megahertz(memory),
                            unknown_reason: unknown_reason(gpu, &[Metric::MemoryClock]),
                            limit: None,
                        },
                        offset(
                            "graphics offset",
                            "Graphics clock offset",
                            clocks.map(|clocks| &clocks.graphics),
                        ),
                        offset(
                            "memory offset",
                            "Memory clock offset",
                            clocks.map(|clocks| &clocks.memory),
                        ),
                    ],
                    vec![
                        chart(
                            Metric::GraphicsClock,
                            Some("GRAPHICS"),
                            &[gpu.device.clocks.graphics_max_mhz],
                        ),
                        chart(
                            Metric::MemoryClock,
                            Some("MEMORY"),
                            &[gpu.device.clocks.memory_max_mhz],
                        ),
                    ],
                )
            }
            TrackKind::Fans => {
                let fans = latest_device(gpu)
                    .map(|device| device.cooling.fans.as_slice())
                    .unwrap_or_default();
                let details = fans
                    .iter()
                    .map(|fan| Detail {
                        label: format!("fan {}", fan.index),
                        name: format!("Fan {} speed", fan.index),
                        readout: Readout::percent(fan.current_speed_percent),
                        unknown_reason: value_reason(
                            gpu,
                            fan.current_speed_percent.is_some(),
                            None,
                        ),
                        limit: None,
                    })
                    .collect();
                (details, vec![chart(metric, None, &[])])
            }
        };
        let stats = gpu.history.stats(metric, self.range, now);
        let value = |value: Option<u32>| readout(metric, value);
        Track {
            stepper: (kind == TrackKind::Power).then(|| self.power_limit_stepper(gpu)),
            kind,
            readout: value(latest_readings(gpu).get(metric)),
            unknown_reason: unknown_reason(gpu, &[metric]),
            details,
            range: self.range.label(),
            max: value(stats.map(|stats| stats.max)),
            avg: value(stats.map(|stats| stats.avg.round() as u32)),
            min: value(stats.map(|stats| stats.min)),
            charts,
            holds_fan_curve: kind == TrackKind::Fans,
        }
    }
}

impl Core {
    /// The power chart's scale fits the readings, the limits in effect and the whole allowed
    /// range, but not a pending limit, so the scale stays put while the limit's line is dragged.
    pub(crate) fn power_chart(&self, gpu: &Gpu) -> Chart {
        let power = latest_device(gpu).map(|device| &device.power);
        let control = &gpu.device.control.power_limit;
        let fit: Vec<_> = [
            power.and_then(|power| power.current_limit_watts),
            power.and_then(|power| power.enforced_limit_watts),
            power
                .and_then(|power| power.max_limit_watts)
                .or(control.max_watts),
            power
                .and_then(|power| power.min_limit_watts)
                .or(control.min_watts),
        ]
        .into_iter()
        .flatten()
        .collect();
        gpu.history
            .chart(Metric::PowerDraw, self.range, self.now(), &fit)
    }

    /// The power limit the GPU's draft stages, if it stages one.
    fn staged_power_limit(&self, gpu: &Gpu) -> Option<u32> {
        match self
            .drafts
            .by_gpu
            .get(&gpu.id)?
            .settings
            .power_limit_watts?
        {
            ProfileSetting::Value { value } => u32::try_from(value).ok(),
            ProfileSetting::Default {} => None,
        }
    }

    /// The power limit's stepper: the value in effect, any pending one, and why it's read-only
    /// when it is.
    fn power_limit_stepper(&self, gpu: &Gpu) -> Stepper {
        let in_effect = self.power_limit_in_effect(gpu);
        let staged = self.staged_power_limit(gpu);
        let range = gpu.power_limit_range();
        let read_only =
            match (self.tuning_access().reason(), &gpu.latest) {
                (Some(reason), _) => Some(reason.to_owned()),
                (None, Latest::Waiting) => Some(WAITING.into()),
                (None, Latest::Missing { reason }) => Some(reason.clone()),
                (None, Latest::Read(device)) => {
                    if !self.editable(&gpu.id, TuningControl::PowerLimit) {
                        Some(device.power.reason.clone().unwrap_or_else(|| {
                            "The power limit isn't supported by this GPU".into()
                        }))
                    } else if range.is_none() {
                        Some("The allowed power limits aren't reported by this GPU".into())
                    } else {
                        None
                    }
                }
            };
        let (min, max) = range.unwrap_or_default();
        let watts =
            |value: Option<i64>| Readout::watts(value.and_then(|value| value.try_into().ok()));
        let value = staged.map(i64::from).or(in_effect);
        Stepper {
            name: "Power limit".into(),
            value: value.unwrap_or(min),
            value_text: watts(value).spoken(),
            min,
            max,
            step: 1,
            readout: watts(in_effect),
            pending: staged.map(|staged| pending(&Readout::watts(Some(staged)))),
            read_only,
        }
    }

    /// The selected GPU's change strip.
    fn change_strip(&self) -> ChangeStrip {
        let draft = self.selected_draft();
        let count = draft.map_or(0, |draft| draft.len());
        ChangeStrip {
            summary: match count {
                0 => "No unapplied changes".into(),
                1 => "1 unapplied change".into(),
                count => format!("{count} unapplied changes"),
            },
            pending: count > 0,
            problem: draft.and_then(|draft| draft.problem.clone()),
            apply_unavailable: draft.and_then(|_| self.changes_unavailable(&Command::ApplyChanges)),
            discard_unavailable: draft
                .and_then(|_| self.changes_unavailable(&Command::DiscardChanges)),
        }
    }
}

/// A reading of `metric` in its unit.
fn readout(metric: Metric, value: Option<u32>) -> Readout {
    match metric {
        Metric::Temperature => Readout::celsius(value),
        Metric::Utilization | Metric::FanSpeed => Readout::percent(value),
        Metric::PowerDraw => Readout::watts(value),
        Metric::GraphicsClock | Metric::MemoryClock => Readout::megahertz(value),
    }
}

/// What the GPU said about a current tuning value it didn't report: the error reading it, or why
/// the control is unavailable.
fn gpu_said<'a>(value_error: Option<&'a str>, control_reason: Option<&'a str>) -> Option<&'a str> {
    value_error.or(control_reason)
}

/// Why a value the latest reading carries is unknown: why there's no reading, or what the GPU
/// said about the value, if anything.
fn value_reason(gpu: &Gpu, known: bool, said: Option<&str>) -> Option<String> {
    match &gpu.latest {
        Latest::Waiting => Some(WAITING.into()),
        Latest::Missing { reason } => Some(reason.clone()),
        Latest::Read(_) if known => None,
        Latest::Read(_) => Some(said.unwrap_or(NOT_REPORTED).into()),
    }
}

fn fact(label: &str, value: impl Into<String>) -> Fact {
    Fact {
        label: label.into(),
        value: value.into(),
    }
}

/// The parts that are known, joined, such as "CUDA 13.2 · capability 12.0"; unknown with none.
fn joined(parts: impl IntoIterator<Item = Option<String>>) -> String {
    let parts: Vec<_> = parts.into_iter().flatten().collect();
    if parts.is_empty() {
        UNKNOWN.into()
    } else {
        parts.join(" · ")
    }
}

/// What the GPU inventory says about the GPU.
fn inventory_facts(device: &GpuDevice) -> Vec<Fact> {
    let known = |value: &Option<String>| value.clone().unwrap_or_else(|| UNKNOWN.into());
    let pcie = match (device.pcie.gen_max, device.pcie.width_max) {
        (Some(generation), Some(width)) => format!("Gen {generation} ×{width}"),
        (Some(generation), None) => format!("Gen {generation}"),
        (None, Some(width)) => format!("×{width}"),
        (None, None) => UNKNOWN.into(),
    };
    let compute = &device.compute;
    let api = compute.api.map(|api| {
        let name = match api {
            ComputeApi::Cuda => "CUDA",
            ComputeApi::Rocm => "ROCm",
        };
        match &compute.api_version {
            Some(version) => format!("{name} {version}"),
            None => name.into(),
        }
    });
    vec![
        fact("Model", device.name.clone()),
        fact("Architecture", known(&device.architecture)),
        fact("UUID", device.id.clone()),
        fact("Bus", known(&device.bus_id)),
        fact("PCIe", pcie),
        fact("Driver", known(&device.driver_version)),
        fact("VBIOS", known(&device.firmware_version)),
        fact(
            "Memory",
            device
                .memory
                .total_mib
                .map_or_else(|| UNKNOWN.into(), |mib| format!("{mib} MiB")),
        ),
        fact(
            "Compute",
            joined([
                api,
                compute
                    .capability
                    .as_ref()
                    .map(|capability| format!("capability {capability}")),
                compute.core_count.map(|cores| format!("{cores} cores")),
            ]),
        ),
        fact(
            "Maximum clocks",
            joined([
                device
                    .clocks
                    .graphics_max_mhz
                    .map(|mhz| format!("graphics {mhz} MHz")),
                device
                    .clocks
                    .memory_max_mhz
                    .map(|mhz| format!("memory {mhz} MHz")),
            ]),
        ),
    ]
}

const SUPPORTED: &str = "Supported";
const NOT_SUPPORTED: &str = "Not supported";
const RANGE_UNKNOWN: &str = "Supported · range unknown";

/// A supported control's allowed range, such as "400–600 W".
fn range(min: Option<u32>, max: Option<u32>, unit: &str) -> String {
    match (min, max) {
        (Some(min), Some(max)) => format!("{min}–{max} {unit}"),
        _ => RANGE_UNKNOWN.into(),
    }
}

/// A supported offset's allowed range, such as "-1000 to +1000 MHz".
fn offset_range(min: Option<i32>, max: Option<i32>) -> String {
    match (min, max) {
        (Some(min), Some(max)) => format!("{} to {} MHz", signed(min), signed(max)),
        _ => RANGE_UNKNOWN.into(),
    }
}

/// `allowed` when the GPU supports the control, or that it doesn't.
fn supported(supported: bool, allowed: impl FnOnce() -> String) -> String {
    if supported {
        allowed()
    } else {
        NOT_SUPPORTED.into()
    }
}

/// The GPU's control capabilities: what it lets Prism tune, and within which range.
fn capability_facts(gpu: &Gpu) -> Vec<Fact> {
    let control = &gpu.device.control;
    let features = &control.supported_features;
    let power = &control.power_limit;
    let offsets = &control.offsets;
    let lock = |lock: &prism_gpu::ClockLockControl| {
        supported(lock.supported, || {
            range(lock.min_allowed_mhz, lock.max_allowed_mhz, "MHz")
        })
    };
    // Cooling is known only from a reading.
    let cooling = match &gpu.latest {
        Latest::Waiting => Err(WAITING.to_owned()),
        Latest::Missing { reason } => Err(reason.clone()),
        Latest::Read(device) => Ok(&device.cooling),
    };
    let fan_control = match &cooling {
        Err(reason) => reason.clone(),
        Ok(cooling) if cooling.is_supported => {
            let count = match cooling.fan_count {
                1 => "1 fan".to_owned(),
                count => format!("{count} fans"),
            };
            let speeds = cooling.fans.first().and_then(|fan| {
                Some(format!(
                    "{}–{}%",
                    fan.min_speed_percent?, fan.max_speed_percent?
                ))
            });
            joined([Some(count), speeds])
        }
        Ok(cooling) => joined([
            Some(NOT_SUPPORTED.into()),
            cooling.unsupported_reason.clone(),
        ]),
    };
    let max_temperature = match &cooling {
        Err(reason) => reason.clone(),
        Ok(cooling) => match cooling.max_operating_temperature_c {
            Some(celsius) => Readout::celsius(Some(celsius)).text(),
            None => cooling
                .max_operating_temperature_reason
                .clone()
                .unwrap_or_else(|| UNKNOWN.into()),
        },
    };
    vec![
        fact(
            "Power limit",
            supported(features.power_limit, || {
                joined([
                    Some(range(power.min_watts, power.max_watts, "W")),
                    power
                        .default_watts
                        .map(|watts| format!("default {watts} W")),
                ])
            }),
        ),
        fact(
            "Graphics clock offset",
            supported(features.graphics_offset, || {
                offset_range(offsets.min_graphics_mhz, offsets.max_graphics_mhz)
            }),
        ),
        fact(
            "Memory clock offset",
            supported(features.memory_offset, || {
                offset_range(offsets.min_memory_mhz, offsets.max_memory_mhz)
            }),
        ),
        fact("Graphics clock lock", lock(&control.gpu_clock_lock)),
        fact("Memory clock lock", lock(&control.memory_clock_lock)),
        fact("Fan control", fan_control),
        fact("Maximum operating temperature", max_temperature),
        fact(
            "Reset to defaults",
            supported(features.reset, || SUPPORTED.into()),
        ),
    ]
}

/// Index, name and identity together, such as "GPU 0 · RTX 5090 · 01:00.0".
pub(crate) fn gpu_label(index: usize, gpu: &Gpu) -> String {
    let index = index_label(index);
    let identity = gpu
        .device
        .bus_id
        .as_deref()
        .map(short_bus_id)
        .unwrap_or_default();
    [index.as_str(), short_gpu_name(&gpu.device.name), identity]
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
        Latest::Waiting => return Some(WAITING.into()),
        Latest::Missing { reason } => return Some(reason.clone()),
        Latest::Read(device) => Readings::from(&device.telemetry),
    };
    let unknown: Vec<_> = metrics
        .iter()
        .filter(|metric| readings.get(**metric).is_none())
        .map(|metric| metric.name())
        .collect();
    match unknown.as_slice() {
        [] => None,
        [_] if metrics.len() == 1 => Some(NOT_REPORTED.into()),
        names => Some(format!(
            "{}: not reported by this GPU",
            capitalized(&names.join(", "))
        )),
    }
}

/// `text` with its first letter in upper case, such as "Fan speed".
fn capitalized(text: &str) -> String {
    let mut letters = text.chars();
    letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default()
}

fn latest_readings(gpu: &Gpu) -> Readings {
    latest_device(gpu)
        .map(|device| Readings::from(&device.telemetry))
        .unwrap_or_default()
}

/// The GPU as the latest sample read it, if it did.
fn latest_device(gpu: &Gpu) -> Option<&GpuPerformanceDevice> {
    match &gpu.latest {
        Latest::Read(device) => Some(device),
        Latest::Waiting | Latest::Missing { .. } => None,
    }
}
