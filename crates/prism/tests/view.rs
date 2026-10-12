//! View tests: the real window, driven headless through accessible labels, roles and actions.

mod support;

use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use prism_gpu::GpuTelemetryProvider;
use prism_gpu::cooling::{CoolingConfiguration, FanSetting};
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_protocol::DaemonEvent;
use prism_ui::AppearanceMode;
use slint::ComponentHandle;

use support::prismd::DaemonState;
use support::{Harness, texts};

use AccessibleRole::{Button, Groupbox, List, ListItem, Spinbox, Text};

const TUNABLE_GPU: &str = "GPU 0 · RTX 5090 · 01:00.0";

#[test]
fn prism_opens_with_its_title_bar_sidebar_workspace_and_status_bar() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);
    harness.tick();

    for button in ["Minimize", "Maximize", "Close"] {
        harness.find(Button, button);
    }
    harness.find(Text, "Prism");
    harness.find(List, "GPUs");
    harness.find(ListItem, TUNABLE_GPU);
    harness.find(Text, "NVIDIA GeForce RTX 5090");
    for track in ["Temperature", "Utilization", "Power", "Clocks", "Fans"] {
        harness.find(Groupbox, track);
    }
    for section in ["Games", "Drivers", "Settings"] {
        harness.find(ListItem, section);
    }
    harness.find(Button, "PRISMD · NOT RUNNING");
    harness.find(Text, "MONITORING IN-PROCESS");
}

#[test]
fn prism_looks_for_gpus_until_the_first_reading() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);

    assert!(harness.shows("Looking for GPUs…"));
    assert!(harness.shows("WAITING FOR TELEMETRY"));
    assert_eq!(harness.count(ListItem, TUNABLE_GPU), 0);
}

#[test]
fn sidebar_vitals_update_with_each_sample() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);

    // The tunable fixture's second second runs at 4% load, its third at 3%.
    harness.tick();
    let block = harness.find(ListItem, TUNABLE_GPU);
    assert_eq!(
        block.accessible_description().as_deref(),
        Some("temperature 35 °C, utilization 4%, power draw 43 W, fan speed 31%")
    );
    assert_eq!(
        texts(&block),
        [
            "GPU 0", "01:00.0", "RTX 5090", "35", "°C", "4%", "43 W", "fan ", "31%"
        ]
    );

    harness.tick();
    assert_eq!(
        texts(&block),
        [
            "GPU 0", "01:00.0", "RTX 5090", "35", "°C", "3%", "38 W", "fan ", "31%"
        ]
    );
}

#[test]
fn the_temperature_track_shows_the_reading_its_chart_and_range_statistics() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);

    // Loads of 4, 3, 38, 87 and 97% heat the GPU to 35, 35, 50, 72 and 76 °C.
    harness.ticks(5);

    let reading = harness.find(Text, "Temperature reading");
    assert_eq!(reading.accessible_value().as_deref(), Some("76 °C"));
    assert_eq!(reading.accessible_description().as_deref(), Some(""));
    for (statistic, value) in [
        ("Maximum temperature", "76 °C"),
        ("Average temperature", "54 °C"),
        ("Minimum temperature", "35 °C"),
    ] {
        assert_eq!(
            harness.find(Text, statistic).accessible_value().as_deref(),
            Some(value),
            "{statistic}"
        );
    }
    assert_eq!(
        harness
            .find(Button, "Show last 5 minutes")
            .accessible_checked(),
        Some(true)
    );
    assert!(
        ElementHandle::find_by_element_type_name(harness.app.window(), "Series")
            .next()
            .is_some(),
        "the chart draws a series"
    );
}

#[test]
fn an_unavailable_reading_shows_as_unknown_with_a_reason() {
    let harness = Harness::without_temperature();
    harness.tick();

    let reading = harness.find(Text, "Temperature reading");
    assert_eq!(reading.accessible_value().as_deref(), Some("unknown"));
    assert_eq!(
        reading.accessible_description().as_deref(),
        Some("Not reported by this GPU")
    );
    assert_eq!(
        harness
            .find(Text, "Maximum temperature")
            .accessible_value()
            .as_deref(),
        Some("unknown")
    );
    let block = harness.find(ListItem, TUNABLE_GPU);
    assert_eq!(texts(&block)[3], "—");
    assert!(
        block
            .accessible_description()
            .is_some_and(|vitals| vitals.starts_with("temperature unknown, utilization 4%"))
    );
}

#[test]
fn a_gpu_without_fan_control_shows_its_fan_speed_as_unknown() {
    let harness = Harness::new(SimulatedFixture::NoFanControl);
    harness.tick();

    let block = harness.find(ListItem, "GPU 0 · RTX 4060 Laptop GPU · 01:00.0");

    assert_eq!(texts(&block).last().map(String::as_str), Some("—"));
    assert!(block.accessible_description().is_some_and(|vitals| {
        vitals.ends_with("fan speed unknown. Fan speed: not reported by this GPU")
    }));
}

#[test]
fn identical_gpus_each_get_a_block_and_selecting_one_opens_it() {
    let harness = Harness::new(SimulatedFixture::TwoIdenticalGpus);
    harness.tick();
    let first = harness.find(ListItem, "GPU 0 · RTX 4090 · 01:00.0");
    let second = harness.find(ListItem, "GPU 1 · RTX 4090 · 02:00.0");
    assert_eq!(first.accessible_item_selected(), Some(true));
    assert!(harness.shows("GPU 0 · 01:00.0 · Ada Lovelace"));

    second.invoke_accessible_default_action();

    assert_eq!(first.accessible_item_selected(), Some(false));
    assert_eq!(second.accessible_item_selected(), Some(true));
    assert!(harness.shows("GPU 1 · 02:00.0 · Ada Lovelace"));
    assert!(!harness.shows("GPU 0 · 01:00.0 · Ada Lovelace"));
}

/// Asserts that Prism opened into Drivers, with the GPU list explaining why it has no GPUs.
fn opens_into_drivers_explaining(harness: &Harness, title: &str, reason: &str) {
    assert_eq!(
        harness.find(ListItem, "Drivers").accessible_item_selected(),
        Some(true)
    );
    // Drivers' heading.
    assert!(harness.shows("Drivers"));
    assert_eq!(
        harness
            .find(Text, "Driver status")
            .accessible_value()
            .as_deref(),
        Some("Driver management needs prismd · not running")
    );
    // Only the GPU list explains it.
    assert_eq!(harness.count(Text, title), 1);
    assert_eq!(harness.count(Text, reason), 1);
    assert_eq!(harness.count(Groupbox, "Temperature"), 0);
}

#[test]
fn a_host_without_nvml_opens_into_drivers_and_shows_why_there_is_no_telemetry() {
    let harness = Harness::new(SimulatedFixture::NvmlMissing);
    harness.tick();

    opens_into_drivers_explaining(
        &harness,
        "No GPU telemetry",
        "NVIDIA inventory is unavailable on this system: unable to initialize NVML: a libloading \
         error occurred: libnvidia-ml.so: cannot open shared object file: No such file or \
         directory",
    );
    assert!(harness.shows(
        "Prism reads NVIDIA GPUs through NVML, which comes with the NVIDIA driver. Check that \
         the driver is installed and loaded, then restart Prism."
    ));
}

#[test]
fn an_empty_inventory_opens_into_drivers_with_an_explained_gpu_list() {
    let harness = Harness::new(SimulatedFixture::EmptyInventory);
    harness.tick();

    opens_into_drivers_explaining(
        &harness,
        "No GPUs found",
        "NVML reports no NVIDIA GPUs on this host.",
    );
    assert!(harness.shows(
        "An empty list alone doesn't mean the host has no GPU. Check that the NVIDIA driver is \
         loaded, then restart Prism."
    ));
}

#[test]
fn a_host_without_the_driver_loaded_opens_into_drivers_with_an_explained_gpu_list() {
    let harness = Harness::new(SimulatedFixture::DriverNotLoaded);
    harness.tick();

    opens_into_drivers_explaining(
        &harness,
        "No GPU telemetry",
        "NVIDIA inventory is unavailable on this system: unable to initialize NVML: NVIDIA \
         driver is not loaded",
    );
}

#[test]
fn with_gpus_present_prism_still_opens_into_the_selected_gpu() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);
    harness.tick();

    assert_eq!(
        harness
            .find(ListItem, TUNABLE_GPU)
            .accessible_item_selected(),
        Some(true)
    );
    assert_eq!(
        harness.find(ListItem, "Drivers").accessible_item_selected(),
        Some(false)
    );
    harness.find(Groupbox, "Temperature");
}

#[test]
fn the_sidebar_rows_lead_to_their_sections() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);
    harness.tick();

    harness
        .find(ListItem, "Drivers")
        .invoke_accessible_default_action();

    assert_eq!(
        harness.find(ListItem, "Drivers").accessible_item_selected(),
        Some(true)
    );
    assert_eq!(
        harness
            .find(ListItem, TUNABLE_GPU)
            .accessible_item_selected(),
        Some(false)
    );
    assert_eq!(
        harness
            .find(Text, "Driver status")
            .accessible_value()
            .as_deref(),
        Some("Driver management needs prismd · not running")
    );
    assert_eq!(harness.count(Groupbox, "Temperature"), 0);
}

#[test]
fn the_close_button_quits_prism() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);
    assert!(harness.app.window().window().is_visible());

    harness
        .find(Button, "Close")
        .invoke_accessible_default_action();

    assert!(!harness.app.window().window().is_visible());
}

#[test]
fn the_appearance_mode_picks_the_light_or_dark_theme() {
    let light = Harness::with_appearance(SimulatedFixture::TunableGpu, AppearanceMode::Light);
    let dark = Harness::with_appearance(SimulatedFixture::TunableGpu, AppearanceMode::Dark);

    let light_theme = light.app.window().global::<prism::Theme>();
    let dark_theme = dark.app.window().global::<prism::Theme>();
    assert!(!light_theme.get_dark());
    assert!(dark_theme.get_dark());
    assert_ne!(
        light_theme.get_color().canvas,
        dark_theme.get_color().canvas
    );
}

mod harness {
    slint::slint! {
        export component Empty inherits Window { }
    }

    #[test]
    #[should_panic(expected = "the element tree is empty")]
    fn the_harness_fails_loudly_when_the_element_tree_is_empty() {
        super::support::init_testing_backend();
        super::support::ensure_element_tree(&Empty::new().unwrap());
    }
}

#[test]
fn values_prismd_pushes_show_even_when_readings_arrived_since_the_window_last_drew() {
    // Slint brings a list inside a track up to date only when it next draws or is read, and
    // nothing is drawn or read between these inputs, as when several arrive at once.
    for (readings, changing) in [(1, false), (2, false), (2, true), (3, true), (4, true)] {
        let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::Running);
        for second in 0..readings {
            let second = if changing { second } else { 1 };
            harness.prismd_tick(
                SimulatedGpus::new(SimulatedFixture::TunableGpu, move || {
                    Duration::from_secs(second)
                })
                .performance(),
            );
        }
        let mut state = support::prismd::snapshot().tuning_states[0].clone();
        state.reported.power_limit_watts = Some(500);
        state.reported.cooling = Some(CoolingConfiguration::Shared {
            setting: FanSetting::Fixed { speed_percent: 60 },
        });
        state.revision = "after".into();
        harness.prismd.push(DaemonEvent::TuningStateChanged(state));
        harness.app.receive_posted();

        let case = format!("after {readings} readings, changing: {changing}");
        assert_eq!(
            harness
                .find(Spinbox, "Power limit")
                .accessible_value()
                .as_deref(),
            Some("500 W"),
            "{case}"
        );
        assert_eq!(
            harness.find(Button, "Fixed speed").accessible_checked(),
            Some(true),
            "{case}"
        );
    }
}
