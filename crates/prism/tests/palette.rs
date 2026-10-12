//! View tests for the command palette, the global shortcuts and later launches: the real window,
//! driven headless with keys and accessible actions.

mod support;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use prism_gpu::simulated::SimulatedFixture;
use slint::ComponentHandle;
use slint::platform::Key;

use support::Harness;

use AccessibleRole::{Groupbox, List, ListItem, TextInput};

const FIRST_GPU: &str = "GPU 0 · RTX 4090 · 01:00.0";
const SECOND_GPU: &str = "GPU 1 · RTX 4090 · 02:00.0";

/// Prism on two GPUs, after their first reading.
fn harness() -> Harness {
    let harness = Harness::new(SimulatedFixture::TwoIdenticalGpus);
    harness.tick();
    harness
}

fn palette_is_open(harness: &Harness) -> bool {
    harness.count(Groupbox, "Command palette") == 1
}

/// The palette's commands, by title.
fn commands(harness: &Harness) -> Vec<String> {
    rows(harness)
        .iter()
        .filter_map(ElementHandle::accessible_label)
        .map(String::from)
        .collect()
}

fn rows(harness: &Harness) -> Vec<ElementHandle> {
    harness
        .find(List, "Commands")
        .query_descendants()
        .match_accessible_role(ListItem)
        .find_all()
}

fn highlighted(harness: &Harness) -> Vec<String> {
    rows(harness)
        .into_iter()
        .filter(|row| row.accessible_item_selected() == Some(true))
        .filter_map(|row| row.accessible_label().map(String::from))
        .collect()
}

fn selected(harness: &Harness, label: &str) -> bool {
    harness.find(ListItem, label).accessible_item_selected() == Some(true)
}

#[test]
fn ctrl_k_opens_the_palette_with_its_field_focused() {
    let harness = harness();
    assert!(!palette_is_open(&harness));

    harness.ctrl("k");

    assert!(palette_is_open(&harness));
    let field = harness.find(TextInput, "Command");
    assert_eq!(field.accessible_value().as_deref(), Some(""));
    assert_eq!(
        commands(&harness),
        [
            format!("Go to {FIRST_GPU}"),
            format!("Go to {SECOND_GPU}"),
            "Go to Games".into(),
            "Go to Drivers".into(),
            "Go to Settings".into(),
            "Go to Settings: Appearance".into(),
            "Go to Settings: Startup".into(),
            "Go to Settings: Notifications".into(),
            "Go to Settings: Daemon".into(),
            "Go to Settings: About".into(),
            "Apply unapplied changes".into(),
            "Discard unapplied changes".into(),
            "Recall remembered fan curve".into(),
            "Save to current profile".into(),
            "Save as new profile…".into(),
            "Rename profile…".into(),
            "Delete profile…".into(),
            "Use current profile as startup profile".into(),
            "Manage profiles…".into(),
            "Turn automatic switching on".into(),
            "Add Steam game…".into(),
            "Add program…".into(),
            "Show last 1 minute".into(),
            "Show last 5 minutes".into(),
            "Show last 15 minutes".into(),
            "Show last 1 hour".into(),
            // The harness's prismd is installed but not running.
            "Enable prismd".into(),
            "Copy prismd setup command".into(),
            "Quit Prism".into(),
        ]
    );
}

#[test]
fn typing_filters_the_commands_and_enter_runs_the_highlighted_one() {
    let harness = harness();
    harness.ctrl("k");

    harness.type_text("drvrs");

    assert_eq!(
        harness
            .find(TextInput, "Command")
            .accessible_value()
            .as_deref(),
        Some("drvrs")
    );
    assert_eq!(commands(&harness), ["Go to Drivers"]);
    assert_eq!(highlighted(&harness), ["Go to Drivers"]);

    harness.press(&[], Key::Return);

    assert!(!palette_is_open(&harness));
    assert!(selected(&harness, "Drivers"));
    assert!(!selected(&harness, FIRST_GPU));
}

#[test]
fn up_and_down_move_the_highlight() {
    let harness = harness();
    harness.ctrl("k");
    // The first GPU is open already, so the second is highlighted.
    assert_eq!(highlighted(&harness), [format!("Go to {SECOND_GPU}")]);

    harness.press(&[], Key::DownArrow);
    assert_eq!(highlighted(&harness), ["Go to Games"]);

    harness.press(&[], Key::UpArrow);
    harness.press(&[], Key::UpArrow);
    assert_eq!(highlighted(&harness), [format!("Go to {FIRST_GPU}")]);

    harness.press(&[], Key::DownArrow);
    harness.press(&[], Key::Return);
    assert!(selected(&harness, SECOND_GPU));
}

#[test]
fn an_unavailable_command_is_dimmed_with_its_reason_and_cannot_be_run() {
    let harness = harness();
    harness.ctrl("k");

    let open = harness.find(ListItem, &format!("Go to {FIRST_GPU}"));
    assert_eq!(open.accessible_enabled(), Some(false));
    assert_eq!(
        open.accessible_description().as_deref(),
        Some("Already open · Ctrl+1")
    );
    let games = harness.find(ListItem, "Go to Games");
    assert_eq!(games.accessible_enabled(), Some(true));
    assert_eq!(
        games.accessible_description().as_deref(),
        Some("Ctrl+Shift+G")
    );

    open.invoke_accessible_default_action();

    assert!(palette_is_open(&harness));
    assert!(selected(&harness, FIRST_GPU));
}

#[test]
fn choosing_a_command_runs_it() {
    let harness = harness();
    harness.ctrl("k");

    harness
        .find(ListItem, "Go to Settings")
        .invoke_accessible_default_action();

    assert!(!palette_is_open(&harness));
    assert!(selected(&harness, "Settings"));
}

#[test]
fn escape_closes_the_palette_and_ctrl_k_opens_it_again() {
    let harness = harness();
    harness.ctrl("k");
    harness.type_text("quit");

    harness.press(&[], Key::Escape);
    assert!(!palette_is_open(&harness));

    harness.ctrl("k");
    assert!(palette_is_open(&harness));
    assert_eq!(commands(&harness).len(), 29);
}

#[test]
fn navigation_shortcuts_open_gpus_and_sections() {
    let harness = harness();

    harness.ctrl("2");
    assert!(selected(&harness, SECOND_GPU));

    harness.ctrl_shift("G");
    assert!(selected(&harness, "Games"));

    harness.ctrl_shift("D");
    assert!(selected(&harness, "Drivers"));

    harness.ctrl(",");
    assert!(selected(&harness, "Settings"));

    harness.ctrl("1");
    assert!(selected(&harness, FIRST_GPU));
}

#[test]
fn shortcuts_work_after_tabbing_to_a_control() {
    let harness = harness();
    harness.press(&[], Key::Tab);

    harness.ctrl("2");

    assert!(selected(&harness, SECOND_GPU));
}

#[test]
fn shortcuts_that_clash_with_text_editing_do_not_fire_in_a_text_field() {
    let harness = harness();
    harness.ctrl("k");

    harness.ctrl("2");

    assert!(palette_is_open(&harness));
    assert!(selected(&harness, FIRST_GPU));

    // Ctrl+Shift+G doesn't clash, so it runs, and closes the palette.
    harness.ctrl_shift("G");
    assert!(!palette_is_open(&harness));
    assert!(selected(&harness, "Games"));
}

#[test]
fn ctrl_q_quits_prism() {
    let harness = harness();
    assert!(harness.app.window().window().is_visible());

    harness.ctrl("q");

    assert!(!harness.app.window().window().is_visible());
}

#[test]
fn quit_prism_in_the_palette_quits() {
    let harness = harness();
    harness.ctrl("k");
    harness.type_text("quit");

    harness.press(&[], Key::Return);

    assert!(!harness.app.window().window().is_visible());
}

#[test]
fn a_later_launch_raises_the_window_with_its_activation_token() {
    let harness = harness();
    let window = harness.app.window().window();
    window.set_minimized(true);

    harness.launches.launch(Some("token-from-the-launcher"));
    harness.app.receive_posted();

    assert!(!window.is_minimized());
    assert!(window.is_visible());
    assert_eq!(
        harness.activation.tokens(),
        [Some("token-from-the-launcher".to_owned())]
    );
}

#[test]
fn copy_prismd_setup_command_puts_it_on_the_clipboard() {
    let harness = harness();
    harness.ctrl("k");
    harness.type_text("copy prismd");

    harness
        .find(ListItem, "Copy prismd setup command")
        .invoke_accessible_default_action();

    assert_eq!(harness.clipboard(), "sudo systemctl enable --now prismd");
}
