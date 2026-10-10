//! Copies the core's view models into the Slint globals after every input.

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use prism_ui::format::Readout;
use prism_ui::{AppearanceMode as CoreAppearance, Content, Core, Section as CoreSection};

use crate::{
    AppState, AppearanceMode, ContentKind, ExplainedStateView, MainWindow, PaletteEntryView,
    ReadoutView, SidebarGpuView, TemperatureTrackView, Theme, WorkspaceView,
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
            let track = &workspace.temperature;
            state.set_workspace(WorkspaceView {
                id: workspace.id.as_str().into(),
                title: workspace.title.as_str().into(),
                subtitle: workspace.subtitle.as_str().into(),
                temperature: TemperatureTrackView {
                    readout: readout(&track.readout),
                    unknown_reason: track.unknown_reason.as_deref().unwrap_or_default().into(),
                    range: track.range.into(),
                    max: readout(&track.max),
                    avg: readout(&track.avg),
                    min: readout(&track.min),
                },
            });
            ContentKind::Gpu
        }
        Content::Section(CoreSection::Games) => ContentKind::Games,
        Content::Section(CoreSection::Drivers) => ContentKind::Drivers,
        Content::Section(CoreSection::Settings) => ContentKind::Settings,
    });

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
    state.set_telemetry(status.telemetry.into());
    // Slint's int is 32 bits; wrapping still changes the value on every input.
    state.set_revision(core.revision() as i32);
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
