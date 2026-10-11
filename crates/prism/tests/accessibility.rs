//! The accessibility audit: every widget in every fixture state declares the role, label, value
//! and state a screen reader needs.

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use prism_gpu::simulated::SimulatedFixture;
use slint::ComponentHandle;

use support::Harness;
use support::audit::{
    Element, TuningAccess, assert_accessible, elements, is_interactive, problems,
};
use support::prismd::DaemonState;

/// Audits the window in `state`, with the tuning access its prismd connection gives: without it,
/// every control must be read-only.
fn audit(state: &str, harness: &Harness) {
    let access = match harness.app.core().tuning_access() {
        prism_ui::TuningAccess::Granted => TuningAccess::Granted,
        prism_ui::TuningAccess::ReadOnly { .. } => TuningAccess::Denied,
    };
    assert_accessible(state, harness.app.window(), access);
}

/// The rows of the sidebar list labeled `list`, by label.
fn rows(harness: &Harness, list: &str) -> Vec<String> {
    harness
        .find(AccessibleRole::List, list)
        .query_descendants()
        .match_accessible_role(AccessibleRole::ListItem)
        .find_all()
        .iter()
        .filter_map(ElementHandle::accessible_label)
        .map(String::from)
        .collect()
}

/// Add each prismd connection reason, tuning access granted and denied, and every menu, popover
/// and dialog open as they land (#120's testing decisions).
#[test]
fn every_fixture_state_is_accessible() {
    for fixture in SimulatedFixture::ALL {
        let name = fixture.name();
        let harness = Harness::new(fixture);
        audit(&format!("{name}, looking for GPUs"), &harness);

        harness.tick();
        audit(name, &harness);

        // The edge-ranges fixture swings each reading to the ends of its range.
        harness.ticks(4);
        audit(&format!("{name}, after five readings"), &harness);

        for gpu in rows(&harness, "GPUs") {
            harness
                .find(AccessibleRole::ListItem, &gpu)
                .invoke_accessible_default_action();
            audit(&format!("{name}, {gpu} selected"), &harness);
        }
        for section in rows(&harness, "Sections") {
            harness
                .find(AccessibleRole::ListItem, &section)
                .invoke_accessible_default_action();
            audit(&format!("{name}, {section}"), &harness);
        }

        harness.ctrl("k");
        audit(&format!("{name}, command palette"), &harness);
        harness.type_text("zzz");
        audit(
            &format!("{name}, command palette without matches"),
            &harness,
        );
        harness.press(&[], slint::platform::Key::Escape);
    }

    let harness = Harness::without_temperature();
    harness.tick();
    audit("tunable, temperature unknown", &harness);

    for (daemon, state) in [
        (DaemonState::Running, "connected to prismd"),
        (
            DaemonState::ReadOnly,
            "connected to prismd without tuning access",
        ),
        (DaemonState::Incompatible, "with an incompatible prismd"),
    ] {
        let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, daemon);
        harness.tick();
        audit(&format!("tunable, {state}"), &harness);
    }
}

/// Every `accessible-action-*` a widget or view declares, and the test below that invokes it.
const ACTION_TESTS: &[(&str, &str, &str)] = &[
    (
        "LinkButton",
        "default",
        "a_link_button_s_default_action_activates_it",
    ),
    (
        "ListRow",
        "default",
        "a_list_row_s_default_action_activates_it",
    ),
    (
        "PaletteRow",
        "default",
        "a_palette_row_s_default_action_runs_its_command",
    ),
    (
        "WindowButton",
        "default",
        "the_window_buttons_default_actions_minimize_maximize_and_close",
    ),
];

#[test]
fn every_declared_accessible_action_has_a_test() {
    let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    let mut declared = Vec::new();
    for file in slint_files(&ui) {
        let source = fs::read_to_string(&file).unwrap();
        let mut component = None;
        for line in source.lines().map(str::trim) {
            let line = line.strip_prefix("export ").unwrap_or(line);
            if let Some(rest) = line.strip_prefix("component ") {
                component = rest.split_whitespace().next();
            }
            if let Some(rest) = line.strip_prefix("accessible-action-") {
                let action = rest.split([' ', '=']).next().unwrap();
                let component = component.expect("an action inside a component");
                declared.push((component.to_owned(), action.to_owned()));
            }
        }
    }
    declared.sort();

    let tests = include_str!("accessibility.rs");
    for (_, _, test) in ACTION_TESTS {
        assert!(
            tests.contains(&format!("#[test]\nfn {test}()")),
            "ACTION_TESTS names {test}, which isn't a test here"
        );
    }
    let mut tested: Vec<_> = ACTION_TESTS
        .iter()
        .map(|(component, action, _)| (component.to_string(), action.to_string()))
        .collect();
    tested.sort();
    assert_eq!(
        declared, tested,
        "each declared accessible action needs a test that invokes it, listed in ACTION_TESTS"
    );
}

fn slint_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(slint_files(&path));
        } else if path
            .extension()
            .is_some_and(|extension| extension == "slint")
        {
            files.push(path);
        }
    }
    files
}

#[test]
fn a_list_row_s_default_action_activates_it() {
    let harness = Harness::new(SimulatedFixture::TwoIdenticalGpus);
    harness.tick();
    let second_gpu = harness.find(AccessibleRole::ListItem, "GPU 1 · RTX 4090 · 02:00.0");
    let drivers = harness.find(AccessibleRole::ListItem, "Drivers");

    second_gpu.invoke_accessible_default_action();
    assert_eq!(second_gpu.accessible_item_selected(), Some(true));

    drivers.invoke_accessible_default_action();
    assert_eq!(drivers.accessible_item_selected(), Some(true));
    assert_eq!(second_gpu.accessible_item_selected(), Some(false));
}

#[test]
fn a_palette_row_s_default_action_runs_its_command() {
    let harness = Harness::new(SimulatedFixture::TwoIdenticalGpus);
    harness.tick();
    harness.ctrl("k");

    harness
        .find(AccessibleRole::ListItem, "Go to Games")
        .invoke_accessible_default_action();

    assert_eq!(
        harness.count(AccessibleRole::Groupbox, "Command palette"),
        0
    );
    assert_eq!(
        harness
            .find(AccessibleRole::ListItem, "Games")
            .accessible_item_selected(),
        Some(true)
    );
}

#[test]
fn a_link_button_s_default_action_activates_it() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);
    harness.tick();

    harness
        .find(AccessibleRole::Button, "Set up")
        .invoke_accessible_default_action();

    assert_eq!(
        harness
            .find(AccessibleRole::ListItem, "Settings")
            .accessible_item_selected(),
        Some(true)
    );
}

#[test]
fn the_window_buttons_default_actions_minimize_maximize_and_close() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);
    let window = harness.app.window().window();
    let press = |label| {
        harness
            .find(AccessibleRole::Button, label)
            .invoke_accessible_default_action()
    };

    press("Maximize");
    assert!(window.is_maximized());
    press("Maximize");
    assert!(!window.is_maximized());

    press("Minimize");
    assert!(window.is_minimized());

    press("Close");
    assert!(!window.is_visible());
}

#[test]
fn the_audit_sees_the_window_s_interactive_elements_and_chart_parts() {
    let harness = Harness::new(SimulatedFixture::TwoIdenticalGpus);
    // A series needs two readings.
    harness.ticks(2);

    let interactive: Vec<_> = elements(harness.app.window())
        .into_iter()
        .filter(|element| is_interactive(element.role))
        .filter_map(|element| element.label)
        .collect();
    let chart_parts: Vec<_> = elements(harness.app.window())
        .into_iter()
        .filter(|element| ["Grid", "Series"].contains(&element.type_name.as_str()))
        .map(|element| (element.type_name, element.role))
        .collect();

    assert_eq!(
        interactive,
        [
            "Minimize",
            "Maximize",
            "Close",
            "GPU 0 · RTX 4090 · 01:00.0",
            "GPU 1 · RTX 4090 · 02:00.0",
            "Games",
            "Drivers",
            "Settings",
            "Set up",
        ]
    );
    assert_eq!(
        chart_parts,
        [
            ("Grid".to_owned(), AccessibleRole::None),
            ("Series".to_owned(), AccessibleRole::None),
        ]
    );
}

/// An element as the audit sees it, with the role and label set and nothing else.
fn element(type_name: &str, id: &str, role: AccessibleRole, label: &str) -> Element {
    Element {
        type_name: type_name.into(),
        id: id.into(),
        role,
        label: Some(label.into()),
        ..Element::default()
    }
}

fn spinbox() -> Element {
    Element {
        value: Some("300 W".into()),
        minimum: Some(100.0),
        maximum: Some(450.0),
        step: Some(5.0),
        ..element(
            "Stepper",
            "PowerTrack::limit",
            AccessibleRole::Spinbox,
            "Power limit",
        )
    }
}

fn switch() -> Element {
    Element {
        checkable: Some(true),
        checked: Some(false),
        ..element(
            "Toggle",
            "FansTrack::curve",
            AccessibleRole::Switch,
            "Fan curve",
        )
    }
}

#[test]
fn an_interactive_element_without_a_label_fails() {
    let unlabeled = Element {
        label: None,
        ..element("WindowButton", "", AccessibleRole::Button, "")
    };
    let blank = element("SectionRow", "Sidebar::games", AccessibleRole::ListItem, "");

    assert_eq!(
        problems(&[unlabeled, blank], TuningAccess::Granted),
        [
            "WindowButton <no id> (Button): no label",
            "SectionRow Sidebar::games (ListItem): no label",
        ]
    );
}

#[test]
fn text_and_containers_need_no_label() {
    let elements = [
        element("StyledText", "", AccessibleRole::Text, ""),
        element("Rectangle", "", AccessibleRole::None, ""),
        element("Sidebar", "", AccessibleRole::Groupbox, ""),
    ];

    assert_eq!(
        problems(&elements, TuningAccess::Granted),
        Vec::<String>::new()
    );
}

#[test]
fn a_spinbox_or_slider_without_its_value_and_range_fails() {
    let without_range = Element {
        minimum: None,
        step: None,
        ..spinbox()
    };
    let without_value = Element {
        role: AccessibleRole::Slider,
        value: None,
        maximum: None,
        ..spinbox()
    };

    assert_eq!(
        problems(&[without_range, without_value], TuningAccess::Granted),
        [
            "Stepper PowerTrack::limit (Spinbox \"Power limit\"): no minimum",
            "Stepper PowerTrack::limit (Spinbox \"Power limit\"): no step",
            "Stepper PowerTrack::limit (Slider \"Power limit\"): no value",
            "Stepper PowerTrack::limit (Slider \"Power limit\"): no maximum",
        ]
    );
}

#[test]
fn a_switch_or_checkbox_without_its_checked_state_fails() {
    let not_checkable = Element {
        checkable: None,
        ..switch()
    };
    let without_checked = Element {
        role: AccessibleRole::Checkbox,
        checked: None,
        ..switch()
    };

    assert_eq!(
        problems(&[not_checkable, without_checked], TuningAccess::Granted),
        [
            "Toggle FansTrack::curve (Switch \"Fan curve\"): not checkable",
            "Toggle FansTrack::curve (Checkbox \"Fan curve\"): no checked state",
        ]
    );
}

#[test]
fn a_control_that_stays_editable_without_tuning_access_fails() {
    let read_only = |element: Element| Element {
        read_only: Some(true),
        ..element
    };
    let editable = [spinbox(), switch()];
    let locked = editable.clone().map(read_only);

    assert_eq!(
        problems(&editable, TuningAccess::Granted),
        Vec::<String>::new()
    );
    assert_eq!(
        problems(&locked, TuningAccess::Denied),
        Vec::<String>::new()
    );
    assert_eq!(
        problems(&editable, TuningAccess::Denied),
        [
            "Stepper PowerTrack::limit (Spinbox \"Power limit\"): not read-only",
            "Toggle FansTrack::curve (Switch \"Fan curve\"): not read-only",
        ]
    );
}

#[test]
fn a_text_field_may_stay_editable_without_tuning_access() {
    let field = element(
        "TextInput",
        "TextField::input",
        AccessibleRole::TextInput,
        "Command",
    );

    assert_eq!(
        problems(&[field], TuningAccess::Denied),
        Vec::<String>::new()
    );
}

#[test]
fn a_chart_part_with_an_accessible_role_fails() {
    let series = Element {
        bases: vec!["Path".into()],
        ..element("Series", "", AccessibleRole::Image, "")
    };
    let unrolled = Element {
        role: AccessibleRole::None,
        ..series.clone()
    };

    assert_eq!(
        problems(&[series, unrolled], TuningAccess::Granted),
        ["Series <no id> (Image): a chart part with a role; the track's readout carries the value"]
    );
}
