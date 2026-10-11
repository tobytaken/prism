//! Copies the core's view models into the Slint globals after every input.

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use prism_ui::format::Readout;
use prism_ui::telemetry::{LimitKind, Metric, TelemetryRange};
use prism_ui::{
    AppearanceMode as CoreAppearance, Content, Core, Fact, Section as CoreSection,
    ServiceAction as CoreServiceAction, SettingsSection as CoreSettingsSection, Track,
};

use crate::{
    AppState, AppearanceMode, ChartId, ChartView, ContentKind, DaemonView, DetailView,
    ExplainedStateView, FactView, LimitMark, MainWindow, PaletteEntryView, ReadoutView,
    SegmentView, ServiceAction, SettingsSection, SidebarGpuView, Theme, TrackView, WorkspaceView,
};

pub(crate) fn sync(window: &MainWindow, core: &Core) {
    window.global::<Theme>().set_mode(match core.appearance() {
        CoreAppearance::System => AppearanceMode::System,
        CoreAppearance::Light => AppearanceMode::Light,
        CoreAppearance::Dark => AppearanceMode::Dark,
    });

    let state = window.global::<AppState>();
    let sidebar = core.sidebar();
    let gpus: Vec<_> = sidebar
        .gpus
        .iter()
        .map(|gpu| SidebarGpuView {
            id: gpu.id.as_str().into(),
            index: gpu.index.as_str().into(),
            name: gpu.name.as_str().into(),
            identity: gpu.identity.as_str().into(),
            label: gpu.label.as_str().into(),
            temperature: readout(&gpu.temperature),
            utilization: readout(&gpu.utilization),
            power_draw: readout(&gpu.power_draw),
            fan_speed: readout(&gpu.fan_speed),
            vitals: gpu.vitals().into(),
            selected: gpu.selected,
        })
        .collect();
    if let Some(model) = update_in_place(&state.get_gpus(), gpus, PartialEq::eq) {
        state.set_gpus(model);
    }
    if let Some(no_gpus) = &sidebar.no_gpus {
        state.set_no_gpus(ExplainedStateView {
            title: no_gpus.title.as_str().into(),
            reason: no_gpus.reason.as_str().into(),
            hint: no_gpus.hint.as_str().into(),
        });
    }

    state.set_content(match core.content() {
        Content::Searching => ContentKind::Searching,
        Content::NoGpus(_) => ContentKind::NoGpus,
        Content::Gpu(workspace) => {
            state.set_workspace(WorkspaceView {
                id: workspace.id.as_str().into(),
                title: workspace.title.as_str().into(),
                subtitle: workspace.subtitle.as_str().into(),
                no_tuning_access: workspace
                    .no_tuning_access
                    .as_deref()
                    .unwrap_or_default()
                    .into(),
                hardware_open: workspace.hardware.open,
            });
            let current = state.get_tracks();
            let tracks: Vec<_> = workspace
                .tracks
                .iter()
                .enumerate()
                .map(|(index, track)| track_view(current.row_data(index), track))
                .collect();
            if let Some(model) = update_in_place(&current, tracks, PartialEq::eq) {
                state.set_tracks(model);
            }
            let ranges: Vec<_> = TelemetryRange::ALL
                .into_iter()
                .map(|range| SegmentView {
                    text: range.label().into(),
                    label: range.command().into(),
                    selected: range == workspace.range,
                })
                .collect();
            if let Some(model) = update_in_place(&state.get_ranges(), ranges, PartialEq::eq) {
                state.set_ranges(model);
            }
            if let Some(model) = update_in_place(
                &state.get_hardware_inventory(),
                facts(&workspace.hardware.inventory),
                PartialEq::eq,
            ) {
                state.set_hardware_inventory(model);
            }
            if let Some(model) = update_in_place(
                &state.get_hardware_capabilities(),
                facts(&workspace.hardware.capabilities),
                PartialEq::eq,
            ) {
                state.set_hardware_capabilities(model);
            }
            ContentKind::Gpu
        }
        Content::Section(CoreSection::Games) => ContentKind::Games,
        Content::Section(CoreSection::Drivers) => ContentKind::Drivers,
        Content::Section(CoreSection::Settings) => ContentKind::Settings,
    });

    state.set_settings_section(match core.settings_section() {
        CoreSettingsSection::Appearance => SettingsSection::Appearance,
        CoreSettingsSection::Startup => SettingsSection::Startup,
        CoreSettingsSection::Notifications => SettingsSection::Notifications,
        CoreSettingsSection::Daemon => SettingsSection::Daemon,
        CoreSettingsSection::About => SettingsSection::About,
    });
    state.set_settings_section_name(core.settings_section().name().into());
    let daemon = core.daemon_settings();
    let action = daemon.action.as_ref();
    state.set_daemon(DaemonView {
        title: daemon.title.as_str().into(),
        has_action: action.is_some(),
        action: match action.map(|action| action.action) {
            Some(CoreServiceAction::Restart) => ServiceAction::Restart,
            Some(CoreServiceAction::Enable) | None => ServiceAction::Enable,
        },
        action_title: action
            .map(|action| action.title.as_str())
            .unwrap_or_default()
            .into(),
        action_busy: action.is_some_and(|action| action.busy),
        error: daemon.error.as_deref().unwrap_or_default().into(),
    });
    let strings = |lines: &[String]| lines.iter().map(SharedString::from).collect::<Vec<_>>();
    if let Some(model) = update_in_place(
        &state.get_daemon_paragraphs(),
        strings(&daemon.paragraphs),
        PartialEq::eq,
    ) {
        state.set_daemon_paragraphs(model);
    }
    if let Some(model) = update_in_place(
        &state.get_daemon_commands(),
        strings(&daemon.commands),
        PartialEq::eq,
    ) {
        state.set_daemon_commands(model);
    }
    if let Some(model) = update_in_place(
        &state.get_daemon_facts(),
        facts(&daemon.facts),
        PartialEq::eq,
    ) {
        state.set_daemon_facts(model);
    }

    let palette = core.palette();
    state.set_palette_open(palette.is_some());
    let entries: Vec<_> = palette
        .map(|palette| palette.entries)
        .unwrap_or_default()
        .iter()
        .map(|entry| PaletteEntryView {
            title: entry.title.as_str().into(),
            keys: ModelRc::new(VecModel::from(
                entry
                    .keys
                    .iter()
                    .map(SharedString::from)
                    .collect::<Vec<_>>(),
            )),
            available: entry.unavailable.is_none(),
            reason: entry.unavailable.as_deref().unwrap_or_default().into(),
            description: entry.description().into(),
            highlighted: entry.highlighted,
        })
        .collect();
    if let Some(model) = update_in_place(&state.get_palette_entries(), entries, same_entry) {
        state.set_palette_entries(model);
    }

    let status = core.status_bar();
    state.set_connection(status.connection.into());
    state.set_connected(status.connected);
    state.set_telemetry(status.telemetry.into());
    // Slint's int is 32 bits; wrapping still changes the value on every input.
    state.set_revision(core.revision() as i32);
}

/// A track as the view shows it. Its details and charts go into the models `current` has, if any,
/// so Slint keeps their elements.
fn track_view(current: Option<TrackView>, track: &Track) -> TrackView {
    let details = track
        .details
        .iter()
        .map(|detail| DetailView {
            label: detail.label.as_str().into(),
            name: detail.name.as_str().into(),
            readout: readout(&detail.readout),
            unknown_reason: detail.unknown_reason.as_deref().unwrap_or_default().into(),
            mark: match detail.limit {
                None => LimitMark::None,
                Some(LimitKind::Configured) => LimitMark::Solid,
                Some(LimitKind::Enforced) => LimitMark::Dashed,
            },
        })
        .collect();
    let charts = track
        .charts
        .iter()
        .map(|chart| {
            let draws = |kind| chart.limits.iter().any(|limit| limit.kind == kind);
            ChartView {
                id: chart_id(chart.metric),
                label: chart.label.unwrap_or_default().into(),
                area: chart.metric == Metric::Utilization,
                limit: draws(LimitKind::Configured),
                enforced_limit: draws(LimitKind::Enforced),
            }
        })
        .collect();
    let [max_label, avg_label, min_label] = track.statistic_labels().map(SharedString::from);
    TrackView {
        name: track.kind.name().into(),
        label: track.kind.name().to_uppercase().into(),
        readout: readout(&track.readout),
        readout_label: track.readout_label().into(),
        unknown_reason: track.unknown_reason.as_deref().unwrap_or_default().into(),
        details: reuse(
            current.as_ref().map(|current| current.details.clone()),
            details,
        ),
        charts: reuse(current.map(|current| current.charts), charts),
        range: track.range.into(),
        max: readout(&track.max),
        avg: readout(&track.avg),
        min: readout(&track.min),
        max_label,
        avg_label,
        min_label,
    }
}

/// The chart the view asks to draw `metric` in.
fn chart_id(metric: Metric) -> ChartId {
    match metric {
        Metric::Temperature => ChartId::Temperature,
        Metric::Utilization => ChartId::Utilization,
        Metric::PowerDraw => ChartId::PowerDraw,
        Metric::GraphicsClock => ChartId::GraphicsClock,
        Metric::MemoryClock => ChartId::MemoryClock,
        Metric::FanSpeed => ChartId::FanSpeed,
    }
}

fn facts(facts: &[Fact]) -> Vec<FactView> {
    facts
        .iter()
        .map(|fact| FactView {
            label: fact.label.as_str().into(),
            value: fact.value.as_str().into(),
        })
        .collect()
}

/// `rows` in `model`, updated in place, when there is one this module made; otherwise in a new
/// model.
fn reuse<T: Clone + PartialEq + 'static>(model: Option<ModelRc<T>>, rows: Vec<T>) -> ModelRc<T> {
    match model {
        Some(model) => update_in_place(&model, rows, PartialEq::eq).unwrap_or(model),
        None => ModelRc::new(VecModel::from(rows)),
    }
}

/// Updates the rows of a model this module made, so Slint keeps the elements built for unchanged
/// rows (`same`) and their focus; returns a new model to set when `current` is another kind.
fn update_in_place<T: Clone + 'static>(
    current: &ModelRc<T>,
    rows: Vec<T>,
    same: impl Fn(&T, &T) -> bool,
) -> Option<ModelRc<T>> {
    let Some(model) = current.as_any().downcast_ref::<VecModel<T>>() else {
        return Some(ModelRc::new(VecModel::from(rows)));
    };
    while model.row_count() > rows.len() {
        model.remove(model.row_count() - 1);
    }
    for (index, row) in rows.into_iter().enumerate() {
        match model.row_data(index) {
            Some(existing) if same(&existing, &row) => {}
            Some(_) => model.set_row_data(index, row),
            None => model.push(row),
        }
    }
    None
}

/// Whether two palette entries show the same, comparing their keys by content: models compare by
/// identity.
fn same_entry(a: &PaletteEntryView, b: &PaletteEntryView) -> bool {
    a.title == b.title
        && a.keys.iter().eq(b.keys.iter())
        && a.available == b.available
        && a.reason == b.reason
        && a.description == b.description
        && a.highlighted == b.highlighted
}

fn readout(readout: &Readout) -> ReadoutView {
    ReadoutView {
        value: readout.value.as_str().into(),
        unit: readout.unit.into(),
        known: readout.is_known(),
        text: readout.text().into(),
        spoken: readout.spoken().into(),
    }
}
