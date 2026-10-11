//! The prismd connection: where GPUs and telemetry come from with and without a session.

use std::time::Duration;

use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{GpuInventoryProvider, GpuPerformanceResponse, GpuTelemetryProvider};
use prism_protocol::{
    DaemonEvent, DaemonRequest, DaemonSnapshot, Incompatible, ProtocolVersion, SessionMessage,
    Welcome,
};
use prism_ui::{
    AppearanceMode, Content, Core, Effect, GpuWorkspace, Input, NotConnected, PrismdConnection,
    TelemetrySource,
};

fn tunable() -> SimulatedGpus {
    SimulatedGpus::new(SimulatedFixture::TunableGpu, || Duration::ZERO)
}

/// The snapshot from the shared contract, which describes the simulated tunable GPU.
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

fn welcome() -> Welcome {
    Welcome {
        protocol: ProtocolVersion::CURRENT,
        prismd_version: "0.1.0".into(),
        can_tune: true,
        reason: None,
    }
}

fn launched() -> Core {
    let mut core = Core::default();
    core.handle(Input::Launched {
        appearance: AppearanceMode::System,
    });
    core
}

fn connected(core: &mut Core) -> Vec<Effect> {
    core.handle(Input::PrismdConnected {
        welcome: welcome(),
        snapshot: snapshot(),
    })
}

fn not_running() -> Input {
    Input::PrismdNotConnected(NotConnected::Unavailable {
        message: "prismd isn't running".into(),
    })
}

fn with_temperature(celsius: u32) -> GpuPerformanceResponse {
    let mut sample = tunable().performance();
    sample.gpus[0].telemetry.temperature_c = Some(celsius);
    sample
}

/// What prismd pushes on a tick, received at `second`.
fn prismd_sample(core: &mut Core, celsius: u32, second: u64) -> Vec<Effect> {
    core.handle(Input::PrismdEvent {
        event: DaemonEvent::TelemetrySampled(with_temperature(celsius)),
        received_at: Duration::from_secs(second),
    })
}

/// What the in-process sampler read, received at `second`.
fn in_process_sample(core: &mut Core, celsius: u32, second: u64) -> Vec<Effect> {
    core.handle(Input::TelemetrySampled {
        gpus: with_temperature(celsius),
        source: TelemetrySource::InProcess,
        received_at: Duration::from_secs(second),
    })
}

fn workspace(core: &Core) -> GpuWorkspace {
    match core.content() {
        Content::Gpu(workspace) => *workspace,
        other => panic!("expected the GPU workspace, got {other:?}"),
    }
}

#[test]
fn prism_connects_to_prismd_before_reading_any_gpu_itself() {
    let mut core = Core::default();

    let effects = core.handle(Input::Launched {
        appearance: AppearanceMode::Dark,
    });

    assert_eq!(effects, [Effect::ConnectPrismd]);
    assert_eq!(core.appearance(), AppearanceMode::Dark);
}

#[test]
fn without_a_session_prism_samples_in_process() {
    let mut core = launched();

    assert_eq!(core.handle(not_running()), [Effect::StartSampler]);
    // Another failed attempt changes nothing.
    assert_eq!(core.handle(not_running()), []);

    assert!(matches!(core.prismd(), PrismdConnection::NotConnected(_)));
    assert_eq!(core.status_bar().connection, "PRISMD NOT CONNECTED");
    assert!(!core.status_bar().connected);
}

#[test]
fn a_session_from_startup_brings_the_gpus_and_their_telemetry_without_sampling_in_process() {
    let mut core = launched();

    let effects = connected(&mut core);

    assert_eq!(
        effects,
        [Effect::SendToPrismd(DaemonRequest::SubscribeTelemetry)]
    );
    assert_eq!(workspace(&core).title, "NVIDIA GeForce RTX 5090");
    assert_eq!(prismd_sample(&mut core, 64, 1), []);
    assert_eq!(core.sidebar().gpus[0].temperature.text(), "64 °C");
    assert!(matches!(core.prismd(), PrismdConnection::Connected(_)));
    assert_eq!(core.status_bar().connection, "PRISMD CONNECTED");
    assert!(core.status_bar().connected);
    assert_eq!(core.status_bar().telemetry, "MONITORING THROUGH PRISMD");
}

#[test]
fn connecting_stops_the_in_process_sampler_and_its_late_readings_are_ignored() {
    let mut core = launched();
    core.handle(not_running());
    core.handle(Input::InventoryRead(tunable().inventory()));
    in_process_sample(&mut core, 40, 1);

    let effects = connected(&mut core);

    assert_eq!(
        effects,
        [
            Effect::SendToPrismd(DaemonRequest::SubscribeTelemetry),
            Effect::StopSampler
        ]
    );
    // Read before the sampler stopped, and arriving after the session's own sample.
    prismd_sample(&mut core, 64, 2);
    in_process_sample(&mut core, 99, 2);
    let track = workspace(&core).temperature;
    assert_eq!(track.readout.text(), "64 °C");
    assert_eq!(track.max.text(), "64 °C");
    assert_eq!(track.chart.segments[0].len(), 2);
}

#[test]
fn losing_the_session_samples_in_process_again_and_history_continues_as_one_series() {
    let mut core = launched();
    connected(&mut core);
    prismd_sample(&mut core, 50, 1);
    prismd_sample(&mut core, 60, 2);

    assert_eq!(core.handle(not_running()), [Effect::StartSampler]);
    // The sampler reads the inventory first, which keeps each GPU's history.
    core.handle(Input::InventoryRead(tunable().inventory()));
    in_process_sample(&mut core, 70, 3);

    let track = workspace(&core).temperature;
    assert_eq!(track.readout.text(), "70 °C");
    assert_eq!(track.min.text(), "50 °C");
    assert_eq!(track.avg.text(), "60 °C");
    assert_eq!(track.chart.segments.len(), 1);
    assert_eq!(track.chart.segments[0].len(), 3);
    assert_eq!(core.status_bar().telemetry, "MONITORING IN-PROCESS");

    // And back to prismd, still one series.
    assert_eq!(
        connected(&mut core),
        [
            Effect::SendToPrismd(DaemonRequest::SubscribeTelemetry),
            Effect::StopSampler
        ]
    );
    prismd_sample(&mut core, 80, 4);
    assert_eq!(workspace(&core).temperature.chart.segments[0].len(), 4);
}

#[test]
fn an_incompatible_prismd_counts_as_no_connection() {
    let mut core = launched();

    let effects = core.handle(Input::PrismdNotConnected(NotConnected::Incompatible(
        Incompatible {
            protocol: ProtocolVersion { major: 0, minor: 9 },
            prismd_version: "0.9.0".into(),
        },
    )));

    assert_eq!(effects, [Effect::StartSampler]);
    assert_eq!(core.status_bar().connection, "PRISMD NOT CONNECTED");
}

#[test]
fn a_gpu_arriving_at_prismd_joins_the_sidebar_and_the_others_keep_their_history() {
    let mut core = launched();
    connected(&mut core);
    prismd_sample(&mut core, 50, 1);
    let mut inventory = snapshot().inventory;
    let mut arrived = inventory.gpus[0].clone();
    arrived.id = "GPU-5a1b0000-0000-4000-8000-0000000000ff".into();
    arrived.bus_id = Some("00000000:02:00.0".into());
    inventory.gpus.push(arrived);

    core.handle(Input::PrismdEvent {
        event: DaemonEvent::InventoryChanged(inventory),
        received_at: Duration::from_secs(2),
    });

    let labels: Vec<_> = core
        .sidebar()
        .gpus
        .iter()
        .map(|gpu| gpu.label.clone())
        .collect();
    assert_eq!(
        labels,
        ["GPU 0 · RTX 5090 · 01:00.0", "GPU 1 · RTX 5090 · 02:00.0"]
    );
    assert_eq!(core.sidebar().gpus[0].temperature.text(), "50 °C");
}
