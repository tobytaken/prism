//! Tuning access: whether this Prism can change GPU tuning at all, decided by the prismd
//! connection alone, and which controls that leaves editable.

use std::time::Duration;

use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};
use prism_gpu::{
    ControlSupport, GpuInventoryProvider, GpuPerformanceResponse, GpuTelemetryProvider,
};
use prism_protocol::{
    DaemonEvent, DaemonSnapshot, Incompatible, ProtocolVersion, SessionMessage, Welcome,
};
use prism_ui::{
    AppearanceMode, Content, Core, Input, NotConnected, TelemetrySource, TuningAccess,
    TuningControl,
};

const GPU: &str = "GPU-5a1b0000-0000-4000-8000-000000000001";

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

fn welcome(can_tune: bool, reason: Option<&str>) -> Welcome {
    Welcome {
        protocol: ProtocolVersion::CURRENT,
        prismd_version: "0.1.0".into(),
        can_tune,
        reason: reason.map(Into::into),
    }
}

fn launched() -> Core {
    let mut core = Core::default();
    core.handle(Input::Launched {
        appearance: AppearanceMode::System,
    });
    core
}

fn connect(core: &mut Core, welcome: Welcome) {
    core.handle(Input::PrismdConnected {
        welcome,
        snapshot: snapshot(),
    });
}

fn not_running() -> Input {
    Input::PrismdNotConnected(NotConnected::Unavailable {
        message: "No such file or directory (os error 2)".into(),
    })
}

fn incompatible() -> Input {
    Input::PrismdNotConnected(NotConnected::Incompatible(Incompatible {
        protocol: ProtocolVersion { major: 0, minor: 9 },
        prismd_version: "0.9.0".into(),
    }))
}

fn read_only(reason: &str) -> TuningAccess {
    TuningAccess::ReadOnly {
        reason: reason.into(),
    }
}

/// The inline "Tuning needs prismd" line's reason in the GPU workspace, if it shows.
fn no_tuning_access(core: &Core) -> Option<String> {
    match core.content() {
        Content::Gpu(workspace) => workspace.no_tuning_access,
        other => panic!("expected the GPU workspace, got {other:?}"),
    }
}

/// prismd's tick of the tunable GPU, with each control's support as given.
fn sample(core: &mut Core, change: impl FnOnce(&mut GpuPerformanceResponse)) {
    let mut sample = tunable().performance();
    change(&mut sample);
    core.handle(Input::PrismdEvent {
        event: DaemonEvent::TelemetrySampled(sample),
        received_at: Duration::from_secs(1),
    });
}

#[test]
fn every_connection_state_decides_tuning_access() {
    let mut core = launched();
    assert_eq!(core.tuning_access(), read_only("connecting"));

    core.handle(not_running());
    assert_eq!(core.tuning_access(), read_only("not connected"));

    core.handle(incompatible());
    assert_eq!(core.tuning_access(), read_only("incompatible version"));

    connect(&mut core, welcome(false, Some("bob isn't in group wheel")));
    assert_eq!(core.tuning_access(), read_only("bob isn't in group wheel"));

    connect(&mut core, welcome(false, None));
    assert_eq!(core.tuning_access(), read_only("not allowed"));

    connect(&mut core, welcome(true, None));
    assert_eq!(core.tuning_access(), TuningAccess::Granted);

    core.handle(not_running());
    assert_eq!(core.tuning_access(), read_only("not connected"));
}

#[test]
fn running_prism_as_root_without_prismd_still_has_no_tuning_access() {
    let mut core = launched();
    core.handle(not_running());
    core.handle(Input::InventoryRead(tunable().inventory()));
    core.handle(Input::TelemetrySampled {
        gpus: tunable().performance(),
        source: TelemetrySource::InProcess,
        received_at: Duration::from_secs(1),
    });

    // The in-process reading says the GPU supports every control, but there is no write path
    // without prismd.
    assert_eq!(core.tuning_access(), read_only("not connected"));
    for control in TuningControl::ALL {
        assert!(!core.editable(GPU, control), "{control:?}");
    }
}

#[test]
fn a_control_is_editable_only_with_tuning_access_and_the_capability_for_it() {
    for can_tune in [true, false] {
        for support in [
            ControlSupport::Supported,
            ControlSupport::Unsupported,
            ControlSupport::Unknown,
        ] {
            let mut core = launched();
            connect(
                &mut core,
                welcome(can_tune, (!can_tune).then_some("not allowed")),
            );
            sample(&mut core, |sample| {
                let gpu = &mut sample.gpus[0];
                gpu.power.support = support;
                gpu.clocks.graphics.support = support;
                gpu.clocks.memory.support = support;
                gpu.cooling.is_supported = support == ControlSupport::Supported;
            });

            let expected = can_tune && support == ControlSupport::Supported;
            for control in TuningControl::ALL {
                assert_eq!(
                    core.editable(GPU, control),
                    expected,
                    "{control:?} with can_tune {can_tune} and {support:?}"
                );
            }
        }
    }
}

#[test]
fn each_control_follows_its_own_capability() {
    let mut core = launched();
    connect(&mut core, welcome(true, None));
    sample(&mut core, |sample| {
        sample.gpus[0].clocks.memory.support = ControlSupport::Unsupported;
        sample.gpus[0].cooling.is_supported = false;
    });

    assert!(core.editable(GPU, TuningControl::PowerLimit));
    assert!(core.editable(GPU, TuningControl::GraphicsClockOffset));
    assert!(!core.editable(GPU, TuningControl::MemoryClockOffset));
    assert!(!core.editable(GPU, TuningControl::Cooling));
}

#[test]
fn nothing_is_editable_before_the_first_reading_or_on_a_gpu_prism_doesnt_know() {
    let mut core = launched();
    connect(&mut core, welcome(true, None));

    for control in TuningControl::ALL {
        assert!(!core.editable(GPU, control), "{control:?} before a reading");
    }
    sample(&mut core, |_| {});
    for control in TuningControl::ALL {
        assert!(core.editable(GPU, control), "{control:?}");
        assert!(
            !core.editable("GPU-unknown", control),
            "{control:?} on another GPU"
        );
    }
}

#[test]
fn the_workspace_says_why_tuning_needs_prismd_only_without_tuning_access() {
    let mut core = launched();
    core.handle(not_running());
    core.handle(Input::InventoryRead(tunable().inventory()));
    assert_eq!(no_tuning_access(&core).as_deref(), Some("not connected"));

    connect(&mut core, welcome(false, Some("bob isn't in group wheel")));
    assert_eq!(
        no_tuning_access(&core).as_deref(),
        Some("bob isn't in group wheel")
    );

    connect(&mut core, welcome(true, None));
    assert_eq!(no_tuning_access(&core), None);
}
