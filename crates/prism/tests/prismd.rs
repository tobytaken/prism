//! View tests for the prismd connection: where monitoring comes from with and without a session.

mod support;

use std::time::Duration;

use i_slint_backend_testing::AccessibleRole;
use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{GpuPerformanceResponse, GpuTelemetryProvider};
use prism_protocol::DaemonRequest;
use prism_ui::{Content, Section};

use support::prismd::DaemonState;
use support::{Harness, texts};

use AccessibleRole::{Button, ListItem, Text};

const TUNABLE_GPU: &str = "GPU 0 · RTX 5090 · 01:00.0";

/// The tunable GPU's readings, as prismd would push them, at `celsius`.
fn reading(celsius: u32) -> GpuPerformanceResponse {
    let mut gpus =
        SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO).performance();
    gpus.gpus[0].telemetry.temperature_c = Some(celsius);
    gpus
}

/// How many samples the selected GPU's temperature chart holds, in one unbroken series.
fn temperature_samples(harness: &Harness) -> Vec<usize> {
    match harness.app.core().content() {
        Content::Gpu(workspace) => workspace
            .temperature
            .chart
            .segments
            .iter()
            .map(Vec::len)
            .collect(),
        other => panic!("expected the GPU workspace, got {other:?}"),
    }
}

fn statistic(harness: &Harness, label: &str) -> String {
    harness
        .find(Text, label)
        .accessible_value()
        .map(String::from)
        .unwrap_or_default()
}

#[test]
fn with_a_session_from_startup_telemetry_comes_from_prismd_and_nvml_is_never_loaded() {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::Running);

    harness.prismd_tick(reading(64));
    harness.prismd_tick(reading(66));

    assert_eq!(
        harness.prismd.requests(),
        [DaemonRequest::SubscribeTelemetry]
    );
    assert_eq!(harness.in_process_reads(), 0);
    let block = harness.find(ListItem, TUNABLE_GPU);
    assert_eq!(texts(&block)[3], "66");
    assert_eq!(statistic(&harness, "Minimum temperature"), "64 °C");
    harness.find(Text, "PRISMD CONNECTED");
    harness.find(Text, "MONITORING THROUGH PRISMD");
}

#[test]
fn stopping_the_daemon_switches_to_in_process_sampling_and_history_continues() {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::Running);
    // Hotter than the simulated GPU ever runs, so they stand out in the statistics.
    harness.prismd_tick(reading(88));
    harness.prismd_tick(reading(90));

    harness.prismd.stop_daemon();
    harness.ticks(2);

    assert!(harness.in_process_reads() > 0);
    assert_eq!(temperature_samples(&harness), [4]);
    assert_eq!(statistic(&harness, "Maximum temperature"), "90 °C");
    // The simulated GPU's own reading, four seconds in.
    let simulated =
        SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::from_secs(4)).performance();
    let celsius = simulated.gpus[0].telemetry.temperature_c.unwrap();
    assert_eq!(
        statistic(&harness, "Temperature reading"),
        format!("{celsius} °C")
    );
    harness.find(Text, "PRISMD NOT CONNECTED");
    harness.find(Text, "MONITORING IN-PROCESS");

    // When the daemon is back, it takes over again, still as one series.
    harness.prismd.start_daemon();
    let reads = harness.in_process_reads();
    harness.prismd_tick(reading(70));
    assert_eq!(harness.in_process_reads(), reads);
    assert_eq!(temperature_samples(&harness), [5]);
    harness.find(Text, "PRISMD CONNECTED");
}

#[test]
fn a_daemon_starting_later_takes_over_from_the_in_process_sampler() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);
    harness.tick();
    harness.find(Text, "MONITORING IN-PROCESS");

    harness.prismd.start_daemon();
    let reads = harness.in_process_reads();
    harness.prismd_tick(reading(60));

    assert_eq!(harness.in_process_reads(), reads);
    assert_eq!(
        harness.prismd.requests(),
        [DaemonRequest::SubscribeTelemetry]
    );
    assert_eq!(statistic(&harness, "Temperature reading"), "60 °C");
    assert_eq!(temperature_samples(&harness), [2]);
    harness.find(Text, "MONITORING THROUGH PRISMD");
}

#[test]
fn an_incompatible_daemon_counts_as_no_connection_and_monitoring_continues_in_process() {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::Incompatible);

    harness.ticks(2);

    assert!(harness.in_process_reads() > 0);
    assert_eq!(harness.prismd.requests(), []);
    assert_eq!(temperature_samples(&harness), [2]);
    harness.find(Text, "PRISMD NOT CONNECTED");
    harness.find(Text, "MONITORING IN-PROCESS");
}

#[test]
fn without_prismd_the_workspace_says_tuning_needs_it_and_set_up_opens_settings() {
    let harness = Harness::new(SimulatedFixture::TunableGpu);
    harness.tick();

    let line = harness.find(Text, "Tuning needs prismd");
    assert_eq!(line.accessible_value().as_deref(), Some("not connected"));

    // Settings → Daemon lands with the connection reasons (#132).
    harness
        .find(Button, "Set up")
        .invoke_accessible_default_action();
    assert_eq!(
        harness.app.core().content(),
        Content::Section(Section::Settings)
    );
}

#[test]
fn a_user_prismd_wont_let_tune_is_told_which_group_they_lack_and_monitors_through_prismd() {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::ReadOnly);

    harness.prismd_tick(reading(64));

    assert_eq!(
        harness
            .find(Text, "Tuning needs prismd")
            .accessible_value()
            .as_deref(),
        Some("bob isn't in group wheel")
    );
    assert_eq!(harness.in_process_reads(), 0);
    harness.find(Text, "PRISMD CONNECTED");
    harness.find(Text, "MONITORING THROUGH PRISMD");
}

#[test]
fn with_tuning_access_the_workspace_has_no_tuning_needs_prismd_line() {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::Running);

    harness.prismd_tick(reading(64));

    assert_eq!(harness.count(Text, "Tuning needs prismd"), 0);
    assert_eq!(harness.count(Button, "Set up"), 0);
}
