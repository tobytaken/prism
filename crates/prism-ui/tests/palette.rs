//! The command palette, shortcuts and later launches, through the core's input/effect interface.

use std::time::Duration;

use prism_gpu::GpuInventoryProvider;
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_ui::telemetry::TelemetryRange;
use prism_ui::{
    AppearanceMode, Content, Core, Effect, Input, Intent, Key, PaletteEntry, Section, Shortcut,
};

const FIRST_GPU: &str = "Go to GPU 0 · RTX 4090 · 01:00.0";
const SECOND_GPU: &str = "Go to GPU 1 · RTX 4090 · 02:00.0";
const FIRST_GPU_ID: &str = "GPU-5a1b0000-0000-4000-8000-000000000002";
const SECOND_GPU_ID: &str = "GPU-5a1b0000-0000-4000-8000-000000000003";

/// A core that has launched and found the fixture's GPUs.
fn launched(fixture: SimulatedFixture) -> Core {
    let mut core = Core::default();
    core.handle(Input::Launched {
        appearance: AppearanceMode::System,
    });
    let gpus = SimulatedGpus::new(fixture, || Duration::ZERO);
    core.handle(Input::InventoryRead(gpus.inventory()));
    core
}

fn intent(core: &mut Core, intent: Intent) -> Vec<Effect> {
    core.handle(Input::Intent(intent))
}

/// The open palette's entries.
fn entries(core: &Core) -> Vec<PaletteEntry> {
    core.palette().expect("the palette is open").entries
}

fn titles(core: &Core) -> Vec<String> {
    entries(core).into_iter().map(|entry| entry.title).collect()
}

fn highlighted(core: &Core) -> String {
    let entries = entries(core);
    let mut highlighted = entries.iter().filter(|entry| entry.highlighted);
    let entry = highlighted.next().expect("an entry is highlighted");
    assert!(highlighted.next().is_none(), "one entry is highlighted");
    entry.title.clone()
}

fn entry(core: &Core, title: &str) -> PaletteEntry {
    entries(core)
        .into_iter()
        .find(|entry| entry.title == title)
        .unwrap_or_else(|| panic!("no entry is titled {title:?}"))
}

fn selected_section(core: &Core) -> Option<Section> {
    match core.content() {
        Content::Section(section) => Some(section),
        _ => None,
    }
}

fn selected_gpu(core: &Core) -> Option<String> {
    match core.content() {
        Content::Gpu(workspace) => Some(workspace.id),
        _ => None,
    }
}

#[test]
fn the_palette_lists_a_command_per_gpu_then_the_sections_their_settings_tuning_ranges_and_quit() {
    let mut core = launched(SimulatedFixture::TwoIdenticalGpus);
    assert!(core.palette().is_none());

    intent(&mut core, Intent::OpenPalette);

    let palette = core.palette().expect("the palette is open");
    assert_eq!(palette.query, "");
    assert_eq!(
        titles(&core),
        [
            FIRST_GPU,
            SECOND_GPU,
            "Go to Games",
            "Go to Drivers",
            "Go to Settings",
            "Go to Settings: Appearance",
            "Go to Settings: Startup",
            "Go to Settings: Notifications",
            "Go to Settings: Daemon",
            "Go to Settings: About",
            "Apply unapplied changes",
            "Discard unapplied changes",
            "Recall remembered fan curve",
            "Save to current profile",
            "Save as new profile…",
            "Rename profile…",
            "Delete profile…",
            "Use current profile as startup profile",
            "Manage profiles…",
            "Turn automatic switching on",
            "Add Steam game…",
            "Add program…",
            "Show last 1 minute",
            "Show last 5 minutes",
            "Show last 15 minutes",
            "Show last 1 hour",
            "Quit Prism",
        ]
    );
}

#[test]
fn each_command_shows_its_shortcut() {
    let mut core = launched(SimulatedFixture::TwoIdenticalGpus);
    intent(&mut core, Intent::OpenPalette);

    let keys: Vec<_> = entries(&core)
        .into_iter()
        .map(|entry| entry.keys.join("+"))
        .collect();

    assert_eq!(
        keys,
        [
            "Ctrl+1",
            "Ctrl+2",
            "Ctrl+Shift+G",
            "Ctrl+Shift+D",
            "Ctrl+,",
            "",
            "",
            "",
            "",
            "",
            "Ctrl+Enter",
            "Ctrl+Shift+Backspace",
            // Recall remembered fan curve and the profile actions.
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            // Turn automatic switching on, Add Steam game and Add program.
            "",
            "",
            "",
            // The ranges.
            "",
            "",
            "",
            "",
            "Ctrl+Q"
        ]
    );
}

#[test]
fn typing_fuzzy_matches_command_titles_regardless_of_case() {
    let mut core = launched(SimulatedFixture::TwoIdenticalGpus);
    intent(&mut core, Intent::OpenPalette);

    for (query, expected) in [
        ("drvrs", &["Go to Drivers"][..]),
        ("GTDRIV", &["Go to Drivers"]),
        (
            "go set",
            &[
                "Go to Settings",
                "Go to Settings: Appearance",
                "Go to Settings: Startup",
                "Go to Settings: Notifications",
                "Go to Settings: Daemon",
                "Go to Settings: About",
            ],
        ),
        ("set dae", &["Go to Settings: Daemon"]),
        ("02:00", &[SECOND_GPU]),
        ("4090", &[FIRST_GPU, SECOND_GPU]),
        ("quit", &["Quit Prism"]),
        // A subsequence keeps the query's order.
        ("vdr", &[]),
        ("xyz", &[]),
    ] {
        intent(&mut core, Intent::FilterPalette(query.into()));

        assert_eq!(core.palette().unwrap().query, query);
        assert_eq!(titles(&core), expected, "{query:?}");
    }
}

#[test]
fn the_first_command_that_can_run_is_highlighted() {
    let mut core = launched(SimulatedFixture::TwoIdenticalGpus);
    intent(&mut core, Intent::OpenPalette);

    // The first GPU is open already.
    assert_eq!(highlighted(&core), SECOND_GPU);

    intent(&mut core, Intent::FilterPalette("set".into()));
    assert_eq!(highlighted(&core), "Go to Settings");
}

#[test]
fn up_and_down_move_the_highlight_and_wrap_around() {
    let mut core = launched(SimulatedFixture::TwoIdenticalGpus);
    intent(&mut core, Intent::OpenPalette);
    intent(&mut core, Intent::FilterPalette("go to".into()));
    assert_eq!(highlighted(&core), SECOND_GPU);

    intent(&mut core, Intent::HighlightNext);
    assert_eq!(highlighted(&core), "Go to Games");

    intent(&mut core, Intent::HighlightPrevious);
    intent(&mut core, Intent::HighlightPrevious);
    assert_eq!(highlighted(&core), FIRST_GPU);

    intent(&mut core, Intent::HighlightPrevious);
    assert_eq!(highlighted(&core), "Go to Settings: About");

    intent(&mut core, Intent::HighlightNext);
    assert_eq!(highlighted(&core), FIRST_GPU);
}

#[test]
fn enter_runs_the_highlighted_command_and_closes_the_palette() {
    let mut core = launched(SimulatedFixture::TwoIdenticalGpus);
    intent(&mut core, Intent::OpenPalette);
    intent(&mut core, Intent::FilterPalette("drivers".into()));

    let effects = intent(&mut core, Intent::RunHighlighted);

    assert_eq!(effects, []);
    assert!(core.palette().is_none());
    assert_eq!(selected_section(&core), Some(Section::Drivers));
}

#[test]
fn choosing_an_entry_runs_it() {
    let mut core = launched(SimulatedFixture::TwoIdenticalGpus);
    intent(&mut core, Intent::OpenPalette);

    intent(&mut core, Intent::RunPaletteEntry(1));

    assert!(core.palette().is_none());
    assert_eq!(selected_gpu(&core).as_deref(), Some(SECOND_GPU_ID));
}

#[test]
fn quit_prism_quits() {
    let mut core = launched(SimulatedFixture::TwoIdenticalGpus);
    intent(&mut core, Intent::OpenPalette);
    intent(&mut core, Intent::FilterPalette("quit".into()));

    assert_eq!(intent(&mut core, Intent::RunHighlighted), [Effect::Quit]);
}

#[test]
fn an_unavailable_command_is_listed_with_its_reason_and_cannot_be_run() {
    let mut core = launched(SimulatedFixture::TwoIdenticalGpus);
    intent(&mut core, Intent::GoTo(Section::Drivers));
    intent(&mut core, Intent::OpenPalette);

    let drivers = entry(&core, "Go to Drivers");
    assert_eq!(drivers.unavailable.as_deref(), Some("Already open"));
    assert_eq!(entry(&core, FIRST_GPU).unavailable, None);

    intent(&mut core, Intent::FilterPalette("drivers".into()));
    assert_eq!(titles(&core), ["Go to Drivers"]);
    assert_eq!(highlighted(&core), "Go to Drivers");

    assert_eq!(intent(&mut core, Intent::RunHighlighted), []);
    assert_eq!(intent(&mut core, Intent::RunPaletteEntry(0)), []);
    assert!(core.palette().is_some(), "the palette stays open");
    assert_eq!(selected_section(&core), Some(Section::Drivers));

    // Nor does it count as run.
    intent(&mut core, Intent::FilterPalette(String::new()));
    assert_eq!(titles(&core)[0], FIRST_GPU);
}

#[test]
fn recently_run_commands_come_first() {
    let mut core = launched(SimulatedFixture::TwoIdenticalGpus);
    for query in ["settings", "games"] {
        intent(&mut core, Intent::OpenPalette);
        intent(&mut core, Intent::FilterPalette(query.into()));
        intent(&mut core, Intent::RunHighlighted);
    }

    intent(&mut core, Intent::OpenPalette);

    assert_eq!(
        titles(&core),
        [
            "Go to Games",
            "Go to Settings",
            FIRST_GPU,
            SECOND_GPU,
            "Go to Drivers",
            "Go to Settings: Appearance",
            "Go to Settings: Startup",
            "Go to Settings: Notifications",
            "Go to Settings: Daemon",
            "Go to Settings: About",
            "Turn automatic switching on",
            "Add Steam game…",
            "Add program…",
            "Show last 1 minute",
            "Show last 5 minutes",
            "Show last 15 minutes",
            "Show last 1 hour",
            "Quit Prism",
        ]
    );
    // Recent commands come first among the matches too.
    intent(&mut core, Intent::FilterPalette("go to s".into()));
    assert_eq!(
        titles(&core),
        [
            "Go to Games",
            "Go to Settings",
            "Go to Drivers",
            "Go to Settings: Appearance",
            "Go to Settings: Startup",
            "Go to Settings: Notifications",
            "Go to Settings: Daemon",
            "Go to Settings: About",
        ]
    );
}

#[test]
fn gpu_and_range_commands_are_left_out_when_the_host_has_no_gpus() {
    let mut core = launched(SimulatedFixture::NvmlMissing);

    intent(&mut core, Intent::OpenPalette);

    assert_eq!(
        titles(&core),
        [
            "Go to Games",
            "Go to Drivers",
            "Go to Settings",
            "Go to Settings: Appearance",
            "Go to Settings: Startup",
            "Go to Settings: Notifications",
            "Go to Settings: Daemon",
            "Go to Settings: About",
            "Turn automatic switching on",
            "Add Steam game…",
            "Add program…",
            "Quit Prism"
        ]
    );
}

#[test]
fn reopening_the_palette_starts_a_new_search() {
    let mut core = launched(SimulatedFixture::TwoIdenticalGpus);
    intent(&mut core, Intent::OpenPalette);
    intent(&mut core, Intent::FilterPalette("quit".into()));

    intent(&mut core, Intent::Dismiss);
    assert!(core.palette().is_none());
    intent(&mut core, Intent::OpenPalette);

    assert_eq!(core.palette().unwrap().query, "");
    assert_eq!(titles(&core).len(), 27);
}

#[test]
fn a_range_command_shows_its_range_unless_it_is_already_shown() {
    let mut core = launched(SimulatedFixture::TwoIdenticalGpus);
    intent(&mut core, Intent::OpenPalette);
    assert_eq!(
        entry(&core, "Show last 5 minutes").unavailable.as_deref(),
        Some("Already shown")
    );

    intent(&mut core, Intent::FilterPalette("last hour".into()));
    intent(&mut core, Intent::RunHighlighted);

    let Content::Gpu(workspace) = core.content() else {
        panic!("expected the GPU workspace");
    };
    assert_eq!(workspace.range, TelemetryRange::OneHour);
    assert!(core.palette().is_none());
    intent(&mut core, Intent::OpenPalette);
    assert_eq!(entry(&core, "Show last 5 minutes").unavailable, None);
    assert_eq!(
        entry(&core, "Show last 1 hour").unavailable.as_deref(),
        Some("Already shown")
    );
}

fn key(text: &str) -> Key {
    Key {
        text: text.into(),
        ..Key::default()
    }
}

fn ctrl(text: &str) -> Key {
    Key {
        control: true,
        ..key(text)
    }
}

fn ctrl_shift(text: &str) -> Key {
    Key {
        shift: true,
        ..ctrl(text)
    }
}

#[test]
fn keys_map_to_their_shortcuts() {
    for (key, shortcut) in [
        (ctrl("k"), Shortcut::OpenPalette),
        (ctrl("1"), Shortcut::GoToGpu(0)),
        (ctrl("9"), Shortcut::GoToGpu(8)),
        (ctrl_shift("G"), Shortcut::GoTo(Section::Games)),
        (ctrl_shift("g"), Shortcut::GoTo(Section::Games)),
        (ctrl_shift("D"), Shortcut::GoTo(Section::Drivers)),
        (ctrl(","), Shortcut::GoTo(Section::Settings)),
        (ctrl("q"), Shortcut::Quit),
        (key("\u{1b}"), Shortcut::Dismiss),
    ] {
        assert_eq!(Shortcut::for_key(&key), Some(shortcut), "{key:?}");
    }
}

#[test]
fn other_keys_are_no_shortcut() {
    let alt = Key {
        alt: true,
        ..ctrl("k")
    };
    for key in [
        key("k"),
        key("1"),
        ctrl("0"),
        ctrl_shift("k"),
        ctrl("g"),
        alt,
        ctrl_shift("\u{1b}"),
    ] {
        assert_eq!(Shortcut::for_key(&key), None, "{key:?}");
    }
}

#[test]
fn only_the_gpu_shortcuts_clash_with_text_editing() {
    let clashing: Vec<_> = [
        Shortcut::OpenPalette,
        Shortcut::GoToGpu(0),
        Shortcut::GoTo(Section::Games),
        Shortcut::Quit,
        Shortcut::Dismiss,
    ]
    .into_iter()
    .filter(|shortcut| shortcut.clashes_with_text_editing())
    .collect();

    assert_eq!(clashing, [Shortcut::GoToGpu(0)]);
}

#[test]
fn ctrl_and_a_number_opens_the_nth_gpu_in_sidebar_order() {
    let mut core = launched(SimulatedFixture::TwoIdenticalGpus);
    intent(&mut core, Intent::GoTo(Section::Games));

    intent(&mut core, Intent::Press(Shortcut::GoToGpu(1)));
    assert_eq!(selected_gpu(&core).as_deref(), Some(SECOND_GPU_ID));

    intent(&mut core, Intent::Press(Shortcut::GoToGpu(0)));
    assert_eq!(selected_gpu(&core).as_deref(), Some(FIRST_GPU_ID));

    // There is no third GPU.
    intent(&mut core, Intent::Press(Shortcut::GoToGpu(2)));
    assert_eq!(selected_gpu(&core).as_deref(), Some(FIRST_GPU_ID));
}

#[test]
fn section_shortcuts_open_their_sections() {
    let mut core = launched(SimulatedFixture::TwoIdenticalGpus);

    for section in [Section::Games, Section::Drivers, Section::Settings] {
        intent(&mut core, Intent::Press(Shortcut::GoTo(section)));
        assert_eq!(selected_section(&core), Some(section));
    }
}

#[test]
fn a_shortcut_closes_the_palette_when_it_runs_a_command() {
    let mut core = launched(SimulatedFixture::TwoIdenticalGpus);
    intent(&mut core, Intent::Press(Shortcut::OpenPalette));
    assert!(core.palette().is_some());

    intent(&mut core, Intent::Press(Shortcut::GoTo(Section::Games)));

    assert!(core.palette().is_none());
    assert_eq!(selected_section(&core), Some(Section::Games));
}

#[test]
fn ctrl_q_quits() {
    let mut core = launched(SimulatedFixture::TwoIdenticalGpus);

    assert_eq!(
        intent(&mut core, Intent::Press(Shortcut::Quit)),
        [Effect::Quit]
    );
}

#[test]
fn escape_closes_the_palette() {
    let mut core = launched(SimulatedFixture::TwoIdenticalGpus);
    intent(&mut core, Intent::Press(Shortcut::OpenPalette));

    intent(&mut core, Intent::Press(Shortcut::Dismiss));
    assert!(core.palette().is_none());

    // With nothing open, Escape does nothing.
    assert_eq!(intent(&mut core, Intent::Press(Shortcut::Dismiss)), []);
}

#[test]
fn a_later_launch_raises_the_window_with_its_activation_token() {
    let mut core = launched(SimulatedFixture::TwoIdenticalGpus);

    let effects = core.handle(Input::LaunchedAgain {
        activation_token: Some("token-from-the-launcher".into()),
    });

    assert_eq!(
        effects,
        [Effect::RaiseWindow {
            activation_token: Some("token-from-the-launcher".into())
        }]
    );
}
