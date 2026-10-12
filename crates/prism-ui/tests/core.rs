use std::time::Duration;

use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{GpuInventoryProvider, GpuPerformanceResponse, GpuTelemetryProvider};
use prism_ui::{
    AppearanceMode, Content, Core, Effect, ExplainedState, Input, Intent, Section, TelemetrySource,
    Track, TrackKind,
};

fn simulated(fixture: SimulatedFixture) -> SimulatedGpus {
    SimulatedGpus::new(fixture, || Duration::ZERO)
}

/// A core that has launched and found the fixture's GPUs, before any telemetry arrives.
fn launched(fixture: SimulatedFixture) -> Core {
    let mut core = Core::default();
    core.handle(Input::Launched {
        appearance: AppearanceMode::System,
        started_unix_ms: 0,
    });
    core.handle(Input::InventoryRead(simulated(fixture).inventory()));
    core
}

fn sampled(core: &mut Core, gpus: GpuPerformanceResponse, second: u64) {
    core.handle(Input::TelemetrySampled {
        gpus,
        source: TelemetrySource::InProcess,
        received_at: Duration::from_secs(second),
    });
}

#[test]
fn the_first_gpu_found_opens_in_the_workspace() {
    let core = launched(SimulatedFixture::TunableGpu);

    let Content::Gpu(workspace) = core.content() else {
        panic!("expected the GPU workspace, got {:?}", core.content());
    };
    assert_eq!(workspace.title, "NVIDIA GeForce RTX 5090");
    assert_eq!(workspace.subtitle, "GPU 0 · 01:00.0 · Blackwell");
    assert!(core.sidebar().gpus[0].selected);
}

#[test]
fn identical_gpus_are_told_apart_in_the_sidebar() {
    let core = launched(SimulatedFixture::TwoIdenticalGpus);

    let labels: Vec<_> = core
        .sidebar()
        .gpus
        .iter()
        .map(|gpu| gpu.label.clone())
        .collect();

    assert_eq!(
        labels,
        ["GPU 0 · RTX 4090 · 01:00.0", "GPU 1 · RTX 4090 · 02:00.0"]
    );
}

/// The fixture's first sample, with the first GPU's temperature replaced.
fn temperature_sample(fixture: SimulatedFixture, celsius: Option<u32>) -> GpuPerformanceResponse {
    let mut sample = simulated(fixture).performance();
    sample.gpus[0].telemetry.temperature_c = celsius;
    sample
}

fn workspace(core: &Core) -> prism_ui::GpuWorkspace {
    match core.content() {
        Content::Gpu(workspace) => *workspace,
        other => panic!("expected the GPU workspace, got {other:?}"),
    }
}

fn temperature(core: &Core) -> Track {
    workspace(core).track(TrackKind::Temperature).clone()
}

#[test]
fn sidebar_vitals_follow_the_latest_sample() {
    let mut core = launched(SimulatedFixture::TunableGpu);
    let mut sample = simulated(SimulatedFixture::TunableGpu).performance();
    let telemetry = &mut sample.gpus[0].telemetry;
    telemetry.temperature_c = Some(71);
    telemetry.gpu_utilization_percent = Some(96);
    telemetry.power_draw_watts = Some(312);
    telemetry.fan_speed_percent = Some(42);

    sampled(&mut core, sample, 1);

    let gpu = &core.sidebar().gpus[0];
    assert_eq!(gpu.temperature.text(), "71 °C");
    assert_eq!(gpu.utilization.text(), "96%");
    assert_eq!(gpu.power_draw.text(), "312 W");
    assert_eq!(gpu.fan_speed.text(), "42%");
    assert_eq!(
        gpu.vitals(),
        "temperature 71 °C, utilization 96%, power draw 312 W, fan speed 42%"
    );
}

#[test]
fn readings_are_unknown_until_the_first_sample_arrives() {
    let core = launched(SimulatedFixture::TunableGpu);

    let track = temperature(&core);

    assert_eq!(core.sidebar().gpus[0].temperature.text(), "—");
    assert_eq!(
        core.sidebar().gpus[0].vitals(),
        "temperature unknown, utilization unknown, power draw unknown, fan speed unknown. \
         Waiting for the first reading"
    );
    assert_eq!(track.readout.text(), "—");
    assert_eq!(
        track.unknown_reason.as_deref(),
        Some("Waiting for the first reading")
    );
}

#[test]
fn the_temperature_track_shows_the_reading_and_its_range_statistics() {
    let mut core = launched(SimulatedFixture::TunableGpu);
    for (second, celsius) in [(1, 40), (2, 71), (3, 45)] {
        sampled(
            &mut core,
            temperature_sample(SimulatedFixture::TunableGpu, Some(celsius)),
            second,
        );
    }

    let track = temperature(&core);

    assert_eq!(track.readout.text(), "45 °C");
    assert_eq!(track.unknown_reason, None);
    assert_eq!(track.range, "5 m");
    assert_eq!(track.max.text(), "71 °C");
    assert_eq!(track.avg.text(), "52 °C");
    assert_eq!(track.min.text(), "40 °C");
    assert_eq!(track.charts[0].chart.segments.len(), 1);
    assert_eq!(track.charts[0].chart.segments[0].len(), 3);
}

#[test]
fn a_reading_the_gpu_does_not_report_is_unknown_with_a_reason() {
    let mut core = launched(SimulatedFixture::TunableGpu);

    sampled(
        &mut core,
        temperature_sample(SimulatedFixture::TunableGpu, None),
        1,
    );

    let track = temperature(&core);
    assert_eq!(track.readout.spoken(), "unknown");
    assert_eq!(
        track.unknown_reason.as_deref(),
        Some("Not reported by this GPU")
    );
    assert_eq!(track.max.text(), "—");
}

#[test]
fn a_gpu_missing_from_a_sample_is_unknown_with_the_sample_s_reason() {
    let mut core = launched(SimulatedFixture::TunableGpu);
    sampled(
        &mut core,
        temperature_sample(SimulatedFixture::TunableGpu, Some(60)),
        1,
    );

    sampled(
        &mut core,
        simulated(SimulatedFixture::NvmlMissing).performance(),
        2,
    );

    let track = temperature(&core);
    assert_eq!(track.readout.text(), "—");
    assert_eq!(
        track.unknown_reason.as_deref(),
        Some(
            "NVIDIA performance controls are unavailable on this system: unable to initialize \
             NVML: a libloading error occurred: libnvidia-ml.so: cannot open shared object \
             file: No such file or directory"
        )
    );
    assert_eq!(core.sidebar().gpus[0].temperature.text(), "—");
    // The earlier reading stays in the range statistics.
    assert_eq!(track.max.text(), "60 °C");
}

#[test]
fn a_host_without_nvml_explains_why_there_is_no_telemetry() {
    let core = launched(SimulatedFixture::NvmlMissing);

    let expected = ExplainedState {
        title: "No GPU telemetry".into(),
        reason: "NVIDIA inventory is unavailable on this system: unable to initialize NVML: a \
                 libloading error occurred: libnvidia-ml.so: cannot open shared object file: No \
                 such file or directory"
            .into(),
        hint: "Prism reads NVIDIA GPUs through NVML, which comes with the NVIDIA driver. Check \
               that the driver is installed and loaded, then restart Prism."
            .into(),
    };
    assert_eq!(core.content(), Content::NoGpus(expected.clone()));
    assert_eq!(core.sidebar().no_gpus, Some(expected));
    assert!(core.sidebar().gpus.is_empty());
}

#[test]
fn an_empty_inventory_is_explained_without_claiming_the_host_has_no_gpu() {
    let core = launched(SimulatedFixture::EmptyInventory);

    let Content::NoGpus(state) = core.content() else {
        panic!("expected an explained state, got {:?}", core.content());
    };
    assert_eq!(state.title, "No GPUs found");
    assert_eq!(state.reason, "NVML reports no NVIDIA GPUs on this host.");
}

#[test]
fn prism_shows_that_it_is_looking_for_gpus_until_the_inventory_is_read() {
    let mut core = Core::default();
    core.handle(Input::Launched {
        appearance: AppearanceMode::System,
        started_unix_ms: 0,
    });

    assert_eq!(core.content(), Content::Searching);
    assert_eq!(core.sidebar().no_gpus, None);
}

#[test]
fn the_sidebar_leads_to_sections_and_back_to_a_gpu() {
    let mut core = launched(SimulatedFixture::TwoIdenticalGpus);
    let second = core.sidebar().gpus[1].id.clone();

    core.handle(Input::Intent(Intent::GoTo(Section::Drivers)));
    assert_eq!(core.content(), Content::Section(Section::Drivers));
    assert_eq!(core.sidebar().section, Some(Section::Drivers));
    assert!(core.sidebar().gpus.iter().all(|gpu| !gpu.selected));

    core.handle(Input::Intent(Intent::SelectGpu(second.clone())));
    assert_eq!(workspace(&core).id, second);
    assert_eq!(workspace(&core).subtitle, "GPU 1 · 02:00.0 · Ada Lovelace");
    assert_eq!(core.sidebar().section, None);
    assert!(core.sidebar().gpus[1].selected);
}

#[test]
fn closing_the_window_quits_prism() {
    let mut core = launched(SimulatedFixture::TunableGpu);

    assert_eq!(
        core.handle(Input::Intent(Intent::CloseWindow)),
        [Effect::Quit]
    );
}

#[test]
fn the_status_bar_shows_the_prismd_connection_and_the_telemetry_source() {
    let mut core = launched(SimulatedFixture::TunableGpu);
    assert_eq!(core.status_bar().connection, "PRISMD · CONNECTING");
    assert_eq!(core.status_bar().telemetry, "WAITING FOR TELEMETRY");

    sampled(
        &mut core,
        simulated(SimulatedFixture::TunableGpu).performance(),
        1,
    );

    assert_eq!(core.status_bar().telemetry, "MONITORING IN-PROCESS");
}

#[test]
fn the_status_bar_does_not_claim_monitoring_when_no_gpu_can_be_read() {
    let mut core = launched(SimulatedFixture::NvmlMissing);

    sampled(
        &mut core,
        simulated(SimulatedFixture::NvmlMissing).performance(),
        1,
    );

    assert_eq!(core.status_bar().telemetry, "NO GPU TELEMETRY");
}

#[test]
fn selecting_a_gpu_that_is_not_in_the_inventory_keeps_the_current_one() {
    let mut core = launched(SimulatedFixture::TunableGpu);
    let current = workspace(&core).id;

    core.handle(Input::Intent(Intent::SelectGpu("GPU-gone".into())));

    assert_eq!(workspace(&core).id, current);
}

#[test]
fn unknown_sidebar_vitals_say_why() {
    let mut core = launched(SimulatedFixture::NoFanControl);
    assert_eq!(
        core.sidebar().gpus[0].vitals(),
        "temperature unknown, utilization unknown, power draw unknown, fan speed unknown. \
         Waiting for the first reading"
    );

    sampled(
        &mut core,
        simulated(SimulatedFixture::NoFanControl).performance(),
        1,
    );

    assert_eq!(
        core.sidebar().gpus[0].vitals(),
        "temperature 41 °C, utilization 2%, power draw 5 W, fan speed unknown. Fan speed: not \
         reported by this GPU"
    );
}
