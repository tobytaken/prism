//! Compact mode: the same window showing each GPU's vitals, its profile buttons and its automatic
//! switching status, stacked. It doesn't edit: anything more opens the full layout. Ctrl+Shift+M
//! and the palette's Toggle compact mode switch it, and the choice is saved.

use std::time::Duration;

use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{GpuInventoryProvider, GpuTelemetryProvider};
use prism_protocol::{
    AutomaticSwitchingStatus, CurrentProfile, DaemonEvent, DaemonRequest, DaemonSnapshot, GameId,
    GpuSwitchingState, GpuSwitchingStatus, ProfileIdRequest, ProfileSettingKey, ProfilesState,
    ProtocolVersion, SessionMessage, SwitchingCondition, Welcome,
};
use prism_ui::{
    CompactMode, Content, Core, Dialog, Effect, GuiSettings, Input, Intent, Key, Section,
    SettingsSection, Shortcut,
};

const GPU: &str = "GPU-5a1b0000-0000-4000-8000-000000000001";
const FIRST_GPU_ID: &str = "GPU-5a1b0000-0000-4000-8000-000000000002";
const SECOND_GPU_ID: &str = "GPU-5a1b0000-0000-4000-8000-000000000003";

fn launched_with(settings: GuiSettings) -> Core {
    let mut core = Core::default();
    core.handle(Input::Launched {
        settings,
        started_unix_ms: 0,
    });
    core
}

/// Prism monitoring two identical GPUs in-process, with a reading of each.
fn monitoring() -> Core {
    let mut core = launched_with(GuiSettings::default());
    let gpus = SimulatedGpus::new(SimulatedFixture::TwoIdenticalGpus, || Duration::ZERO);
    core.handle(Input::InventoryRead(gpus.inventory()));
    core.handle(Input::TelemetrySampled {
        gpus: gpus.performance(),
        source: prism_ui::TelemetrySource::InProcess,
        received_at: Duration::from_secs(1),
    });
    core
}

fn snapshot() -> DaemonSnapshot {
    let message: SessionMessage = serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/daemon-snapshot.json"
    ))
    .unwrap();
    let SessionMessage::Snapshot { snapshot } = message else {
        panic!("the contract holds a snapshot")
    };
    snapshot
}

/// The caller's Quiet and Boost profiles of the tunable GPU, with Quiet as the startup profile
/// and Boost current.
fn profiles() -> ProfilesState {
    serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/profiles-state.json"
    ))
    .unwrap()
}

/// Prism connected to prismd with tuning access to the tunable GPU and the caller's profiles of
/// it.
fn connected() -> Core {
    let mut core = launched_with(GuiSettings::default());
    let mut snapshot = snapshot();
    snapshot.profiles = profiles();
    core.handle(Input::PrismdConnected {
        welcome: Welcome {
            protocol: ProtocolVersion::CURRENT,
            prismd_version: "0.1.0".into(),
            can_tune: true,
            reason: None,
            authorization: None,
        },
        snapshot,
        socket: "/run/prism/prismd.sock".into(),
    });
    core.handle(Input::PrismdEvent {
        event: DaemonEvent::TelemetrySampled(
            SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance(),
        ),
        received_at: Duration::from_secs(1),
    });
    core
}

fn intent(core: &mut Core, intent: Intent) -> Vec<Effect> {
    core.handle(Input::Intent(intent))
}

fn ctrl_shift_m() -> Key {
    Key {
        text: "M".into(),
        control: true,
        shift: true,
        ..Key::default()
    }
}

fn toggle(core: &mut Core) -> Vec<Effect> {
    intent(core, Intent::Press(Shortcut::ToggleCompactMode))
}

fn compact(core: &Core) -> CompactMode {
    core.compact_mode().expect("compact mode is on")
}

fn saved(effects: &[Effect]) -> Vec<GuiSettings> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::SaveSettings(settings) => Some(*settings),
            _ => None,
        })
        .collect()
}

fn profile_applications(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::CommandPrismd {
                request: DaemonRequest::ApplyProfileById(ProfileIdRequest { profile_id }),
                ..
            } => Some(profile_id.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn ctrl_shift_m_toggles_compact_mode() {
    assert_eq!(
        Shortcut::for_key(&ctrl_shift_m()),
        Some(Shortcut::ToggleCompactMode)
    );
    assert_eq!(Shortcut::ToggleCompactMode.keys(), ["Ctrl", "Shift", "M"]);
    assert!(!Shortcut::ToggleCompactMode.clashes_with_text_editing());

    let mut core = monitoring();
    assert!(core.compact_mode().is_none());

    toggle(&mut core);
    assert!(core.compact_mode().is_some());

    toggle(&mut core);
    assert!(core.compact_mode().is_none());
}

#[test]
fn the_choice_is_saved_and_the_next_launch_starts_with_it() {
    let mut core = monitoring();

    let effects = toggle(&mut core);
    assert_eq!(
        saved(&effects),
        [GuiSettings {
            compact_mode: true,
            ..GuiSettings::default()
        }]
    );

    let next = launched_with(saved(&effects)[0]);
    assert!(next.compact_mode().is_some());

    let effects = toggle(&mut core);
    assert_eq!(saved(&effects), [GuiSettings::default()]);
    assert!(!GuiSettings::default().compact_mode);
}

#[test]
fn the_palette_toggles_compact_mode_with_its_shortcut_shown() {
    let mut core = monitoring();
    intent(&mut core, Intent::OpenPalette);
    intent(&mut core, Intent::FilterPalette("toggle compact".into()));

    let palette = core.palette().expect("the palette is open");
    let entry = palette
        .entries
        .iter()
        .find(|entry| entry.title == "Toggle compact mode")
        .expect("the palette lists Toggle compact mode");
    assert_eq!(entry.shortcut(), "Ctrl+Shift+M");
    assert_eq!(entry.unavailable, None);

    intent(&mut core, Intent::RunHighlighted);
    assert!(core.compact_mode().is_some());
    assert!(!core.palette_open());

    // It stays listed in compact mode, to go back.
    intent(&mut core, Intent::OpenPalette);
    intent(&mut core, Intent::FilterPalette("toggle compact".into()));
    intent(&mut core, Intent::RunHighlighted);
    assert!(core.compact_mode().is_none());
}

#[test]
fn compact_mode_stacks_every_gpus_vitals() {
    let mut core = monitoring();
    let sidebar = core.sidebar();
    toggle(&mut core);

    let compact = compact(&core);
    let ids: Vec<_> = compact
        .gpus
        .iter()
        .map(|gpu| gpu.vitals.id.as_str())
        .collect();
    assert_eq!(ids, [FIRST_GPU_ID, SECOND_GPU_ID]);
    for (gpu, block) in compact.gpus.iter().zip(&sidebar.gpus) {
        assert_eq!(gpu.vitals, *block, "the same vitals as the sidebar's block");
        assert!(gpu.vitals.temperature.is_known());
    }
    // Without prismd there are no profiles and no switching status.
    assert!(
        compact
            .gpus
            .iter()
            .all(|gpu| gpu.profiles.choices.is_empty())
    );
    assert!(compact.gpus.iter().all(|gpu| gpu.switching.is_none()));
}

#[test]
fn compact_mode_shows_each_gpus_profile_buttons_and_switching_status() {
    let mut core = connected();
    core.handle(Input::PrismdEvent {
        event: DaemonEvent::AutomaticSwitchingChanged(AutomaticSwitchingStatus {
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
        }),
        received_at: Duration::from_secs(2),
    });
    let Content::Gpu(workspace) = core.content() else {
        panic!("expected the GPU workspace")
    };
    toggle(&mut core);

    let compact = compact(&core);
    let [gpu] = compact.gpus.as_slice() else {
        panic!("one GPU")
    };
    assert_eq!(
        gpu.profiles, workspace.profiles,
        "the header's profile buttons"
    );
    let names: Vec<_> = gpu
        .profiles
        .choices
        .iter()
        .map(|choice| &choice.name)
        .collect();
    assert_eq!(names, ["Quiet", "Boost"]);
    assert_eq!(gpu.switching, workspace.switching);
    assert_eq!(
        gpu.switching.as_ref().map(|status| status.summary.as_str()),
        Some("Cyberpunk 2077 → Quiet")
    );
    assert_eq!(gpu.unapplied, None);
}

#[test]
fn choosing_a_profile_in_compact_mode_applies_it_and_stays_compact() {
    let mut core = connected();
    toggle(&mut core);

    let effects = intent(&mut core, Intent::ApplyProfile("quiet".into()));

    assert_eq!(profile_applications(&effects), ["quiet"]);
    assert!(core.compact_mode().is_some());
    assert!(saved(&effects).is_empty());
}

#[test]
fn with_unapplied_changes_compact_mode_asks_before_discarding_them() {
    let mut core = connected();
    intent(
        &mut core,
        Intent::Stage(ProfileSettingKey::PowerLimitWatts, 450),
    );
    toggle(&mut core);
    assert_eq!(
        compact(&core).gpus[0].unapplied.as_deref(),
        Some("1 unapplied change")
    );

    let effects = intent(&mut core, Intent::ApplyProfile("quiet".into()));
    assert!(profile_applications(&effects).is_empty());
    let Some(Dialog::DiscardForProfile(dialog)) = core.dialog() else {
        panic!("expected the discard question, got {:?}", core.dialog())
    };
    assert_eq!(dialog.title, "Discard 1 unapplied change and apply Quiet?");

    let effects = intent(&mut core, Intent::ConfirmProfile);
    assert_eq!(profile_applications(&effects), ["quiet"]);
    assert!(core.compact_mode().is_some());
    assert_eq!(compact(&core).gpus[0].unapplied, None);
}

#[test]
fn anything_more_opens_the_full_layout() {
    for (name, then) in [
        ("selecting a GPU", Intent::SelectGpu(SECOND_GPU_ID.into())),
        ("going to Games", Intent::GoTo(Section::Games)),
        (
            "going to Settings",
            Intent::GoToSettings(SettingsSection::Daemon),
        ),
        ("opening Activity", Intent::OpenActivity),
        ("the GPU shortcut", Intent::Press(Shortcut::GoToGpu(0))),
        (
            "the range",
            Intent::ShowRange(prism_ui::telemetry::TelemetryRange::OneHour),
        ),
    ] {
        let mut core = monitoring();
        toggle(&mut core);

        let effects = intent(&mut core, then);

        assert!(core.compact_mode().is_none(), "{name} leaves compact mode");
        assert_eq!(saved(&effects), [GuiSettings::default()], "{name}");
    }
}

#[test]
fn compact_mode_keeps_what_it_can_do_itself() {
    let mut core = monitoring();
    toggle(&mut core);

    let effects = intent(
        &mut core,
        Intent::SetAppearance(prism_ui::AppearanceMode::Dark),
    );

    assert!(core.compact_mode().is_some());
    assert_eq!(
        saved(&effects),
        [GuiSettings {
            appearance: prism_ui::AppearanceMode::Dark,
            compact_mode: true,
            ..GuiSettings::default()
        }]
    );
}

#[test]
fn entering_compact_mode_closes_activity_and_the_profile_menu() {
    let mut core = connected();
    intent(&mut core, Intent::OpenActivity);
    assert!(core.activity_open());

    toggle(&mut core);
    assert!(core.compact_mode().is_some());
    assert!(!core.activity_open());

    toggle(&mut core);
    intent(&mut core, Intent::ToggleProfileMenu);
    assert!(core.profile_menu_open());
    toggle(&mut core);
    assert!(!core.profile_menu_open());
}

#[test]
fn leaving_compact_mode_shows_what_was_shown_before() {
    let mut core = monitoring();
    intent(&mut core, Intent::SelectGpu(SECOND_GPU_ID.into()));
    toggle(&mut core);
    toggle(&mut core);

    let Content::Gpu(workspace) = core.content() else {
        panic!("expected the GPU workspace")
    };
    assert_eq!(workspace.id, SECOND_GPU_ID);
}
