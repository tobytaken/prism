//! Session connections: the handshake, the snapshot that follows it, and telemetry pushed to the
//! sessions that subscribed.

use serde_json::json;

use super::harness::{FakeGpuBackend, Handshake, Harness, tunable_gpu, tunable_gpu_with_id};
use prism_protocol::{
    DaemonResponse, HelloRequest, Incompatible, ProtocolVersion, TuningStoreHealth,
};

const PRISMD_VERSION: &str = env!("CARGO_PKG_VERSION");

fn hello(major: u32, minor: u32) -> HelloRequest {
    HelloRequest {
        protocol: ProtocolVersion { major, minor },
        prism_version: "0.1.0".into(),
    }
}

#[test]
fn a_hello_in_this_protocol_is_welcomed_with_both_versions() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));

    let Handshake::Welcomed(session) = daemon.hello(1000, "alice", HelloRequest::current("0.1.0"))
    else {
        panic!("the session was refused")
    };

    let welcome = session.welcome();
    assert_eq!(welcome.protocol, ProtocolVersion::CURRENT);
    assert_eq!(welcome.prismd_version, PRISMD_VERSION);
    assert!(welcome.can_tune);
    assert_eq!(welcome.reason, None);
}

#[test]
fn a_hello_in_another_protocol_is_refused_as_incompatible() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));

    // Before the first release, the minor version must match too.
    for (major, minor) in [(0, 0), (0, 2), (1, 1)] {
        let Handshake::Refused(response) = daemon.hello(1000, "alice", hello(major, minor)) else {
            panic!("{major}.{minor} was welcomed")
        };
        assert_eq!(
            response,
            DaemonResponse::Incompatible(Incompatible {
                protocol: ProtocolVersion::CURRENT,
                prismd_version: PRISMD_VERSION.into(),
            }),
            "{major}.{minor}"
        );
    }
}

#[test]
fn the_snapshot_after_the_welcome_holds_the_state_events_later_change() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));

    let alice = daemon.user(1000, "alice");

    let snapshot = alice.snapshot();
    let inventory: Vec<_> = snapshot.inventory.gpus.iter().map(|gpu| &gpu.id).collect();
    assert_eq!(inventory, ["GPU-123"]);
    assert_eq!(snapshot.store_health, TuningStoreHealth::Healthy {});
    assert_eq!(snapshot.tuning_states.len(), 1);
    assert_eq!(snapshot.tuning_states[0].gpu_id, "GPU-123");
    // The same as the caller reads scope by scope.
    assert_eq!(
        serde_json::to_value(&snapshot.running_games).unwrap(),
        alice.send_json(json!({ "type": "get_running_games" }))["payload"]
    );
    assert_eq!(
        serde_json::to_value(&snapshot.automatic_switching).unwrap(),
        alice.send_json(json!({ "type": "get_automatic_switching_status" }))["payload"]
    );
    // Only later changes arrive as events.
    assert_eq!(alice.pushed_events(), Vec::<serde_json::Value>::new());
}

#[test]
fn telemetry_is_read_once_per_tick_and_fanned_out_to_every_subscriber() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");
    for session in [&alice, &bob] {
        assert_eq!(
            session.send_json(json!({ "type": "subscribe_telemetry" }))["payload"]["success"],
            true
        );
    }
    daemon
        .gpus
        .update("GPU-123", |gpu| gpu.telemetry.temperature_c = Some(64));
    let reads = daemon.gpus.performance_reads();

    daemon.tick_telemetry();

    assert_eq!(daemon.gpus.performance_reads(), reads + 1);
    let sampled = alice.pushed_events();
    assert_eq!(sampled.len(), 1, "{sampled:?}");
    assert_eq!(sampled[0]["type"], "telemetry_sampled");
    assert_eq!(
        sampled[0]["payload"]["gpus"][0]["telemetry"]["temperature_c"],
        64
    );
    // The daemon's control of the fans travels with the readings.
    assert!(sampled[0]["payload"]["gpus"][0]["cooling"]["control"].is_object());
    assert_eq!(bob.pushed_events(), sampled);
}

#[test]
fn telemetry_is_not_read_while_no_session_is_subscribed() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    let reads = daemon.gpus.performance_reads();

    daemon.tick_telemetry();

    assert_eq!(daemon.gpus.performance_reads(), reads);
    assert_eq!(alice.pushed_events(), Vec::<serde_json::Value>::new());

    // A subscriber that leaves stops the reads again.
    let bob = daemon.user(1001, "bob");
    bob.send_json(json!({ "type": "subscribe_telemetry" }));
    drop(bob);
    let reads = daemon.gpus.performance_reads();
    daemon.tick_telemetry();
    assert_eq!(daemon.gpus.performance_reads(), reads);
}

#[test]
fn a_gpu_arriving_or_departing_changes_the_inventory_before_its_telemetry() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    alice.send_json(json!({ "type": "subscribe_telemetry" }));
    daemon.tick_telemetry();
    alice.pushed_events();

    daemon.gpus.arrive(tunable_gpu_with_id("GPU-456"));
    daemon.tick_telemetry();

    let events = alice.pushed_events();
    let kinds: Vec<_> = events.iter().map(|event| &event["type"]).collect();
    assert_eq!(kinds, ["inventory_changed", "telemetry_sampled"]);
    let ids: Vec<_> = events[0]["payload"]["gpus"]
        .as_array()
        .unwrap()
        .iter()
        .map(|gpu| gpu["id"].clone())
        .collect();
    assert_eq!(ids, ["GPU-123", "GPU-456"]);

    daemon.gpus.depart("GPU-123");
    daemon.tick_telemetry();
    let events = alice.pushed_events();
    assert_eq!(events[0]["type"], "inventory_changed");
    assert_eq!(events[0]["payload"]["gpus"][0]["id"], "GPU-456");
    assert_eq!(events[0]["payload"]["gpus"].as_array().unwrap().len(), 1);
}
