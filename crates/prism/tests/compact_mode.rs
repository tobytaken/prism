//! View tests for compact mode: the same window showing each GPU's block with its profile buttons
//! and automatic switching status, sized to fit them. Ctrl+Shift+M, the palette and the title
//! bar's Full layout button switch it, and the choice is remembered. A GPU's block opens the GPU
//! in the full layout.

mod support;

use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use prism_gpu::GpuTelemetryProvider;
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_protocol::{
    AutomaticSwitchingStatus, CurrentProfile, DaemonEvent, DaemonRequest, GameId,
    GpuSwitchingState, GpuSwitchingStatus, ProfileIdRequest, ProfilesState, SwitchingCondition,
};
use prism_ui::GuiSettings;
use slint::ComponentHandle;
use slint::platform::Key;

use support::prismd::DaemonState;
use support::{DEFAULT_SIZE, FakeSettings, Harness};

use AccessibleRole::{Button, Groupbox, List, ListItem, Spinbox, Text};

const GPU: &str = "GPU-5a1b0000-0000-4000-8000-000000000001";
const TUNABLE: &str = "GPU 0 · RTX 5090 · 01:00.0";
const FIRST: &str = "GPU 0 · RTX 4090 · 01:00.0";
const SECOND: &str = "GPU 1 · RTX 4090 · 02:00.0";

/// The caller's Quiet and Boost profiles of the tunable GPU, with Quiet as its startup profile and
/// Boost current.
fn profiles() -> ProfilesState {
    serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/profiles-state.json"
    ))
    .unwrap()
}

/// Prism connected to prismd with one reading of the tunable GPU and its profiles pushed.
fn connected() -> Harness {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::Running);
    harness.prismd_tick(
        SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
    );
    harness
        .prismd
        .push(DaemonEvent::ProfilesChanged(profiles()));
    harness.app.receive_posted();
    harness
}

/// Two identical GPUs monitored in-process, at the default window size.
fn monitoring() -> Harness {
    let harness = Harness::new(SimulatedFixture::TwoIdenticalGpus);
    harness.resize(DEFAULT_SIZE);
    harness.ticks(2);
    harness
}

fn compact_mode(harness: &Harness) {
    harness.ctrl_shift("M");
    harness.tick();
}

fn in_compact_mode(harness: &Harness) -> bool {
    harness
        .app
        .window()
        .global::<prism::AppState>()
        .get_compact()
}

/// The window's size in logical pixels.
fn window_size(harness: &Harness) -> (f32, f32) {
    let window = harness.app.window().window();
    let size = window.size().to_logical(window.scale_factor());
    (size.width, size.height)
}

/// Compact mode's width, and the height of the title bar and `gpus` blocks.
fn compact_size(harness: &Harness, gpus: usize) -> (f32, f32) {
    let size = harness.app.window().global::<prism::Theme>().get_size();
    (
        size.compact_width,
        size.title_bar + gpus as f32 * size.compact_gpu_block,
    )
}

fn profile_requests(harness: &Harness) -> Vec<DaemonRequest> {
    harness
        .prismd
        .requests()
        .into_iter()
        .filter(|request| matches!(request, DaemonRequest::ApplyProfileById(_)))
        .collect()
}

fn apply_by_id(profile_id: &str) -> DaemonRequest {
    DaemonRequest::ApplyProfileById(ProfileIdRequest {
        profile_id: profile_id.into(),
    })
}

fn press(element: ElementHandle, harness: &Harness) {
    element.invoke_accessible_default_action();
    harness.tick();
}

#[test]
fn ctrl_shift_m_stacks_every_gpus_block_in_a_window_that_fits_them() {
    let harness = monitoring();
    assert_eq!(harness.count(ListItem, "Games"), 1);

    compact_mode(&harness);

    assert!(in_compact_mode(&harness));
    let blocks: Vec<_> = harness
        .find(List, "GPUs")
        .query_descendants()
        .match_accessible_role(ListItem)
        .find_all()
        .iter()
        .filter_map(|block| block.accessible_label().map(String::from))
        .collect();
    assert_eq!(blocks, [FIRST, SECOND]);
    let vitals = harness
        .find(ListItem, FIRST)
        .accessible_description()
        .unwrap_or_default();
    assert!(
        vitals.starts_with("temperature "),
        "the block carries the vitals: {vitals}"
    );
    // No sidebar sections, workspace or status bar.
    assert_eq!(harness.count(ListItem, "Games"), 0);
    assert_eq!(harness.count(Groupbox, "Range"), 0);
    assert_eq!(harness.count(Button, "PRISMD · NOT RUNNING"), 0);
    // Without prismd the GPUs have no profiles.
    assert_eq!(harness.count(Text, "Profiles need prismd · not running"), 2);
    assert_eq!(window_size(&harness), compact_size(&harness, 2));
    let minimum = support::minimum_size(harness.app.window().window());
    assert!(
        minimum.width <= compact_size(&harness, 2).0,
        "the desktop can keep it this narrow: {minimum:?}"
    );

    compact_mode(&harness);

    assert!(!in_compact_mode(&harness));
    assert_eq!(harness.count(ListItem, "Games"), 1);
    assert_eq!(harness.count(Groupbox, "Range"), 1);
    assert_eq!(
        window_size(&harness),
        (DEFAULT_SIZE.0 as f32, DEFAULT_SIZE.1 as f32),
        "the full layout gets its size back"
    );
}

#[test]
fn the_palette_and_the_full_layout_button_toggle_compact_mode() {
    let harness = monitoring();
    harness.ctrl("k");
    harness.type_text("toggle compact");
    let entry = harness.find(ListItem, "Toggle compact mode");
    assert_eq!(
        entry.accessible_description().as_deref(),
        Some("Ctrl+Shift+M")
    );
    harness.press(&[], Key::Return);
    harness.tick();
    assert!(in_compact_mode(&harness));

    let full_layout = harness.find(Button, "Full layout");
    assert_eq!(
        full_layout.accessible_description().as_deref(),
        Some("Ctrl+Shift+M")
    );
    press(full_layout, &harness);

    assert!(!in_compact_mode(&harness));
    assert_eq!(harness.count(Button, "Full layout"), 0);
}

#[test]
fn the_choice_is_remembered_after_a_restart() {
    let harness = monitoring();
    compact_mode(&harness);
    let saved = harness.settings.saved();
    assert!(saved.compact_mode);
    drop(harness);

    let restarted = Harness::with_settings(
        SimulatedFixture::TwoIdenticalGpus,
        DaemonState::NotRunning,
        FakeSettings::new(GuiSettings {
            compact_mode: true,
            ..GuiSettings::default()
        }),
    );
    restarted.ticks(2);

    assert!(in_compact_mode(&restarted));
    assert_eq!(restarted.count(ListItem, FIRST), 1);
    assert_eq!(window_size(&restarted), compact_size(&restarted, 2));

    // Started in compact mode, the full layout opens at its default size.
    compact_mode(&restarted);
    assert_eq!(
        window_size(&restarted),
        (DEFAULT_SIZE.0 as f32, DEFAULT_SIZE.1 as f32)
    );
    assert!(!restarted.settings.saved().compact_mode);
}

#[test]
fn a_gpus_block_opens_it_in_the_full_layout() {
    let harness = monitoring();
    compact_mode(&harness);

    press(harness.find(ListItem, SECOND), &harness);

    assert!(!in_compact_mode(&harness));
    assert_eq!(
        harness.find(ListItem, SECOND).accessible_item_selected(),
        Some(true)
    );
    assert!(!harness.settings.saved().compact_mode);
}

#[test]
fn choosing_a_profile_in_compact_mode_applies_it() {
    let harness = connected();
    compact_mode(&harness);
    assert_eq!(
        harness
            .find(Button, "Apply profile Boost")
            .accessible_checked(),
        Some(true)
    );
    assert_eq!(
        harness
            .find(Button, "Apply profile Quiet")
            .accessible_description()
            .as_deref(),
        Some("Startup profile")
    );

    press(harness.find(Button, "Apply profile Quiet"), &harness);

    assert_eq!(profile_requests(&harness), [apply_by_id("quiet")]);
    assert!(in_compact_mode(&harness));
}

#[test]
fn with_unapplied_changes_choosing_a_profile_asks_first() {
    let harness = connected();
    harness
        .find(Spinbox, "Power limit")
        .set_accessible_value("450");
    compact_mode(&harness);
    assert!(harness.shows("1 unapplied change"));
    let question = "Discard 1 unapplied change and apply Quiet?";

    press(harness.find(Button, "Apply profile Quiet"), &harness);

    assert_eq!(profile_requests(&harness), []);
    assert_eq!(harness.count(Groupbox, question), 1);
    press(harness.find(Button, "Discard and apply"), &harness);
    assert_eq!(profile_requests(&harness), [apply_by_id("quiet")]);
    assert_eq!(harness.count(Groupbox, question), 0);
    assert!(in_compact_mode(&harness));
    assert!(!harness.shows("1 unapplied change"));
}

#[test]
fn compact_mode_shows_each_gpus_automatic_switching_status() {
    let harness = connected();
    harness.prismd.push(DaemonEvent::AutomaticSwitchingChanged(
        AutomaticSwitchingStatus {
            condition: SwitchingCondition::Ready {},
            gpus: vec![GpuSwitchingStatus {
                gpu_id: GPU.into(),
                state: GpuSwitchingState::Active {
                    game: GameId::Steam { app_id: 1_091_500 },
                    game_name: "Cyberpunk 2077".into(),
                    profile: CurrentProfile {
                        id: "quiet".into(),
                        name: "Quiet".into(),
                    },
                },
                application: None,
            }],
        },
    ));
    harness.app.receive_posted();

    compact_mode(&harness);

    let status = harness.find(Text, &format!("Automatic switching status of {TUNABLE}"));
    assert_eq!(
        status.accessible_value().as_deref(),
        Some("Cyberpunk 2077 → Quiet")
    );
}

#[test]
fn compact_mode_says_why_there_are_no_gpus() {
    let harness = Harness::with_settings(
        SimulatedFixture::NvmlMissing,
        DaemonState::NotRunning,
        FakeSettings::new(GuiSettings {
            compact_mode: true,
            ..GuiSettings::default()
        }),
    );
    harness.ticks(2);

    assert!(in_compact_mode(&harness));
    assert!(harness.shows("No GPU telemetry"));
    assert_eq!(window_size(&harness), compact_size(&harness, 1));
}
