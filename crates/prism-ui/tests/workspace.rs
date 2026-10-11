//! The GPU workspace's monitoring tracks, range picker and hardware details.

use std::time::Duration;

use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{
    GpuInventoryProvider, GpuPerformanceDevice, GpuPerformanceResponse, GpuTelemetryProvider,
};
use prism_ui::telemetry::{LimitKind, Metric, TelemetryRange};
use prism_ui::{
    AppearanceMode, Content, Core, GpuWorkspace, Input, Intent, TelemetrySource, Track, TrackKind,
};

fn simulated(fixture: SimulatedFixture) -> SimulatedGpus {
    SimulatedGpus::new(fixture, || Duration::ZERO)
}

fn launched(fixture: SimulatedFixture) -> Core {
    let mut core = Core::default();
    core.handle(Input::Launched {
        appearance: AppearanceMode::System,
    });
    core.handle(Input::InventoryRead(simulated(fixture).inventory()));
    core
}

/// The tunable GPU's first sample, changed by `change`.
fn tunable_sample(change: impl FnOnce(&mut GpuPerformanceDevice)) -> GpuPerformanceResponse {
    let mut sample = simulated(SimulatedFixture::TunableGpu).performance();
    change(&mut sample.gpus[0]);
    sample
}

fn sampled(core: &mut Core, gpus: GpuPerformanceResponse, second: u64) {
    core.handle(Input::TelemetrySampled {
        gpus,
        source: TelemetrySource::InProcess,
        received_at: Duration::from_secs(second),
    });
}

fn workspace(core: &Core) -> GpuWorkspace {
    match core.content() {
        Content::Gpu(workspace) => *workspace,
        other => panic!("expected the GPU workspace, got {other:?}"),
    }
}

fn track(core: &Core, kind: TrackKind) -> Track {
    workspace(core).track(kind).clone()
}

/// The track's max, avg and min as shown.
fn statistics(track: &Track) -> [String; 3] {
    [track.max.text(), track.avg.text(), track.min.text()]
}

/// The track's details as shown: each label with its reading.
fn details(track: &Track) -> Vec<(String, String)> {
    track
        .details
        .iter()
        .map(|detail| (detail.label.clone(), detail.readout.text()))
        .collect()
}

#[test]
fn the_workspace_has_the_five_tracks_top_to_bottom() {
    let core = launched(SimulatedFixture::TunableGpu);

    let names: Vec<_> = workspace(&core)
        .tracks
        .iter()
        .map(|track| track.kind.name())
        .collect();

    assert_eq!(
        names,
        ["Temperature", "Utilization", "Power", "Clocks", "Fans"]
    );
}

#[test]
fn each_track_s_readout_and_statistics_are_labeled_by_its_metric() {
    let core = launched(SimulatedFixture::TunableGpu);

    let power = track(&core, TrackKind::Power);

    assert_eq!(power.readout_label(), "Power draw reading");
    assert_eq!(
        power.statistic_labels(),
        [
            "Maximum power draw",
            "Average power draw",
            "Minimum power draw"
        ]
    );
}

#[test]
fn each_track_shows_its_reading_and_its_range_statistics() {
    let mut core = launched(SimulatedFixture::TunableGpu);
    for (second, load) in [(1, 20), (2, 90), (3, 40)] {
        let sample = tunable_sample(|gpu| {
            let telemetry = &mut gpu.telemetry;
            telemetry.temperature_c = Some(40 + load / 2);
            telemetry.gpu_utilization_percent = Some(load);
            telemetry.power_draw_watts = Some(5 * load);
            telemetry.core_clock_mhz = Some(30 * load);
            telemetry.memory_clock_mhz = Some(100 * load);
            telemetry.fan_speed_percent = Some(30 + load / 2);
        });
        sampled(&mut core, sample, second);
    }

    let expected = [
        (TrackKind::Temperature, "60 °C", ["85 °C", "65 °C", "50 °C"]),
        (TrackKind::Utilization, "40%", ["90%", "50%", "20%"]),
        (TrackKind::Power, "200 W", ["450 W", "250 W", "100 W"]),
        (
            TrackKind::Clocks,
            "1200 MHz",
            ["2700 MHz", "1500 MHz", "600 MHz"],
        ),
        (TrackKind::Fans, "50%", ["75%", "55%", "40%"]),
    ];
    for (kind, readout, stats) in expected {
        let track = track(&core, kind);
        assert_eq!(track.readout.text(), readout, "{kind:?}");
        assert_eq!(track.unknown_reason, None, "{kind:?}");
        assert_eq!(statistics(&track), stats, "{kind:?}");
        assert_eq!(track.range, "5 m", "{kind:?}");
        let chart = &track.charts[0];
        assert_eq!(chart.metric, track.kind.metric(), "{kind:?}");
        assert_eq!(chart.chart.segments.len(), 1, "{kind:?}");
        assert_eq!(chart.chart.segments[0].len(), 3, "{kind:?}");
    }
}

#[test]
fn the_power_track_draws_the_configured_and_the_enforced_power_limit() {
    let mut core = launched(SimulatedFixture::TunableGpu);
    let sample = tunable_sample(|gpu| {
        gpu.power.current_limit_watts = Some(450);
        gpu.power.enforced_limit_watts = Some(300);
        gpu.telemetry.power_draw_watts = Some(150);
    });

    sampled(&mut core, sample, 1);

    let power = track(&core, TrackKind::Power);
    assert_eq!(
        details(&power),
        [
            ("limit".to_owned(), "450 W".to_owned()),
            ("enforced".to_owned(), "300 W".to_owned()),
        ]
    );
    assert_eq!(power.details[0].name, "Power limit");
    assert_eq!(power.details[1].name, "Enforced power limit");
    let kinds: Vec<_> = power.details.iter().map(|detail| detail.limit).collect();
    assert_eq!(
        kinds,
        [Some(LimitKind::Configured), Some(LimitKind::Enforced)]
    );
    let chart = &power.charts[0];
    // The scale reaches the GPU's highest allowed limit, 600 W, so a limit never leaves it.
    assert_eq!((chart.chart.min, chart.chart.max), (0, 600));
    let limits: Vec<_> = chart
        .limits
        .iter()
        .map(|limit| (limit.kind, limit.y))
        .collect();
    assert_eq!(
        limits,
        [(LimitKind::Configured, 0.25), (LimitKind::Enforced, 0.5)]
    );
}

#[test]
fn the_clocks_track_charts_graphics_and_memory_clocks_and_shows_the_offsets() {
    let mut core = launched(SimulatedFixture::TunableGpu);
    let sample = tunable_sample(|gpu| {
        gpu.telemetry.core_clock_mhz = Some(2_400);
        gpu.telemetry.memory_clock_mhz = Some(13_000);
        gpu.clocks.graphics.current_offset_mhz = Some(150);
        gpu.clocks.memory.current_offset_mhz = Some(-500);
    });

    sampled(&mut core, sample, 1);

    let clocks = track(&core, TrackKind::Clocks);
    assert_eq!(clocks.readout.text(), "2400 MHz");
    assert_eq!(
        details(&clocks),
        [
            ("memory".to_owned(), "13000 MHz".to_owned()),
            ("graphics offset".to_owned(), "+150 MHz".to_owned()),
            ("memory offset".to_owned(), "-500 MHz".to_owned()),
        ]
    );
    let charts: Vec<_> = clocks
        .charts
        .iter()
        .map(|chart| (chart.metric, chart.label, chart.chart.max))
        .collect();
    // Each scale reaches the clock's maximum from the inventory: 3090 and 14001 MHz.
    assert_eq!(
        charts,
        [
            (Metric::GraphicsClock, Some("GRAPHICS"), 3_500),
            (Metric::MemoryClock, Some("MEMORY"), 14_500),
        ]
    );
}

#[test]
fn the_fans_track_shows_each_fan_s_current_speed() {
    let mut core = launched(SimulatedFixture::TunableGpu);
    let sample = tunable_sample(|gpu| {
        gpu.telemetry.fan_speed_percent = Some(45);
        gpu.cooling.fans[0].current_speed_percent = Some(44);
        gpu.cooling.fans[1].current_speed_percent = Some(46);
    });

    sampled(&mut core, sample, 1);

    let fans = track(&core, TrackKind::Fans);
    assert_eq!(fans.readout.text(), "45%");
    assert_eq!(
        details(&fans),
        [
            ("fan 0".to_owned(), "44%".to_owned()),
            ("fan 1".to_owned(), "46%".to_owned()),
        ]
    );
    assert_eq!(fans.details[1].name, "Fan 1 speed");
}

#[test]
fn only_the_fans_track_holds_the_fan_curve() {
    let mut core = launched(SimulatedFixture::TunableGpu);
    sampled(&mut core, tunable_sample(|_| {}), 1);

    for kind in TrackKind::ALL {
        assert_eq!(
            track(&core, kind).fan_curve,
            kind == TrackKind::Fans,
            "{kind:?}"
        );
    }
}

#[test]
fn every_track_shows_unknown_readings_as_unknown_with_a_reason() {
    let mut core = launched(SimulatedFixture::TunableGpu);
    let sample = tunable_sample(|gpu| {
        let telemetry = &mut gpu.telemetry;
        telemetry.temperature_c = None;
        telemetry.gpu_utilization_percent = None;
        telemetry.power_draw_watts = None;
        telemetry.core_clock_mhz = None;
        telemetry.memory_clock_mhz = None;
        telemetry.fan_speed_percent = None;
        gpu.power.current_limit_watts = None;
        gpu.power.enforced_limit_watts = None;
        gpu.power.current_value_error = Some("NVML: insufficient permissions".into());
        gpu.clocks.graphics.current_offset_mhz = None;
        gpu.clocks.memory.current_offset_mhz = None;
        for fan in &mut gpu.cooling.fans {
            fan.current_speed_percent = None;
        }
    });

    sampled(&mut core, sample, 1);

    for track in workspace(&core).tracks {
        let kind = track.kind;
        assert_eq!(track.readout.spoken(), "unknown", "{kind:?}");
        assert_eq!(
            track.unknown_reason.as_deref(),
            Some("Not reported by this GPU"),
            "{kind:?}"
        );
        assert_eq!(statistics(&track), ["—", "—", "—"], "{kind:?}");
        assert!(track.charts.iter().all(|chart| chart.limits.is_empty()));
        for detail in &track.details {
            assert_eq!(detail.readout.text(), "—", "{kind:?} {}", detail.label);
            assert!(detail.unknown_reason.is_some(), "{kind:?} {}", detail.label);
        }
    }
    let power = track(&core, TrackKind::Power);
    assert_eq!(
        power.details[0].unknown_reason.as_deref(),
        Some("NVML: insufficient permissions")
    );
}

#[test]
fn before_the_first_reading_every_track_waits_for_it() {
    let core = launched(SimulatedFixture::TunableGpu);

    for track in workspace(&core).tracks {
        assert_eq!(track.readout.text(), "—");
        assert_eq!(
            track.unknown_reason.as_deref(),
            Some("Waiting for the first reading")
        );
    }
}

#[test]
fn a_gpu_without_fans_shows_its_fan_speed_as_unknown_and_lists_no_fans() {
    let mut core = launched(SimulatedFixture::NoFanControl);

    sampled(
        &mut core,
        simulated(SimulatedFixture::NoFanControl).performance(),
        1,
    );

    let fans = track(&core, TrackKind::Fans);
    assert_eq!(fans.readout.text(), "—");
    assert_eq!(
        fans.unknown_reason.as_deref(),
        Some("Not reported by this GPU")
    );
    assert!(fans.details.is_empty());
}

/// The temperatures Prism took in at 10 s (90 °C), 250 s (40 °C) and 300 s (60 °C).
fn three_temperatures() -> Core {
    let mut core = launched(SimulatedFixture::TunableGpu);
    for (second, celsius) in [(10, 90), (250, 40), (300, 60)] {
        let sample = tunable_sample(|gpu| gpu.telemetry.temperature_c = Some(celsius));
        sampled(&mut core, sample, second);
    }
    core
}

#[test]
fn the_range_picker_starts_at_five_minutes() {
    let core = three_temperatures();

    let temperature = track(&core, TrackKind::Temperature);
    assert_eq!(workspace(&core).range, TelemetryRange::FiveMinutes);
    assert_eq!(statistics(&temperature), ["90 °C", "63 °C", "40 °C"]);
    assert_eq!(temperature.charts[0].chart.segments[0].len(), 3);
}

#[test]
fn picking_a_range_changes_every_track_s_chart_window_and_statistics() {
    let mut core = three_temperatures();

    core.handle(Input::Intent(Intent::ShowRange(TelemetryRange::OneMinute)));

    let workspace = workspace(&core);
    assert_eq!(workspace.range, TelemetryRange::OneMinute);
    let temperature = workspace.track(TrackKind::Temperature);
    assert_eq!(temperature.range, "1 m");
    assert_eq!(statistics(temperature), ["60 °C", "50 °C", "40 °C"]);
    let points: Vec<_> = temperature.charts[0].chart.segments[0]
        .iter()
        .map(|point| point.x)
        .collect();
    // 50 s before the newest sample is 10 s into the minute.
    assert_eq!(points.len(), 2);
    assert!((points[0] - 10.0 / 60.0).abs() < 1e-6, "{points:?}");
    assert_eq!(points[1], 1.0);
    assert!(
        workspace
            .tracks
            .iter()
            .all(|track| track.range == "1 m" && track.charts[0].chart.segments[0].len() == 2)
    );
}

#[test]
fn the_range_stays_when_another_gpu_opens() {
    let mut core = launched(SimulatedFixture::TwoIdenticalGpus);
    let second = core.sidebar().gpus[1].id.clone();

    core.handle(Input::Intent(Intent::ShowRange(TelemetryRange::OneHour)));
    core.handle(Input::Intent(Intent::SelectGpu(second)));

    assert_eq!(workspace(&core).range, TelemetryRange::OneHour);
}

#[test]
fn hardware_details_are_collapsed_until_opened() {
    let mut core = launched(SimulatedFixture::TunableGpu);
    assert!(!workspace(&core).hardware.open);

    core.handle(Input::Intent(Intent::ToggleHardwareDetails));
    assert!(workspace(&core).hardware.open);

    core.handle(Input::Intent(Intent::ToggleHardwareDetails));
    assert!(!workspace(&core).hardware.open);
}

fn facts(facts: &[prism_ui::Fact]) -> Vec<(&str, &str)> {
    facts
        .iter()
        .map(|fact| (fact.label.as_str(), fact.value.as_str()))
        .collect()
}

#[test]
fn hardware_details_list_the_gpu_s_inventory_and_control_capabilities() {
    let mut core = launched(SimulatedFixture::TunableGpu);
    sampled(
        &mut core,
        simulated(SimulatedFixture::TunableGpu).performance(),
        1,
    );

    let hardware = workspace(&core).hardware;

    assert_eq!(
        facts(&hardware.inventory),
        [
            ("Model", "NVIDIA GeForce RTX 5090"),
            ("Architecture", "Blackwell"),
            ("UUID", "GPU-5a1b0000-0000-4000-8000-000000000001"),
            ("Bus", "00000000:01:00.0"),
            ("PCIe", "Gen 4 ×16"),
            ("Driver", "595.58.03"),
            ("VBIOS", "98.02.2E.00.01"),
            ("Memory", "32607 MiB"),
            ("Compute", "CUDA 13.2 · capability 12.0 · 21760 cores"),
            ("Maximum clocks", "graphics 3090 MHz · memory 14001 MHz"),
        ]
    );
    assert_eq!(
        facts(&hardware.capabilities),
        [
            ("Power limit", "400–600 W · default 575 W"),
            ("Graphics clock offset", "-1000 to +1000 MHz"),
            ("Memory clock offset", "-2000 to +6000 MHz"),
            ("Graphics clock lock", "210–3090 MHz"),
            ("Memory clock lock", "405–14001 MHz"),
            ("Fan control", "2 fans · 30–100%"),
            ("Maximum operating temperature", "90 °C"),
            ("Reset to defaults", "Supported"),
        ]
    );
}

#[test]
fn hardware_details_explain_controls_the_gpu_lacks() {
    let mut core = launched(SimulatedFixture::NoFanControl);
    let fan_control = |core: &Core| {
        workspace(core)
            .hardware
            .capabilities
            .into_iter()
            .find(|fact| fact.label == "Fan control")
            .map(|fact| fact.value)
    };
    assert_eq!(
        fan_control(&core).as_deref(),
        Some("Waiting for the first reading")
    );

    sampled(
        &mut core,
        simulated(SimulatedFixture::NoFanControl).performance(),
        1,
    );

    assert_eq!(
        fan_control(&core).as_deref(),
        Some("Not supported · Fan control is not exposed by this GPU or driver.")
    );
}
