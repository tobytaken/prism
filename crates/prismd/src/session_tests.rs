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
    assert_eq!(
        kinds,
        [
            "inventory_changed",
            "tuning_state_changed",
            "telemetry_sampled"
        ]
    );
    assert_eq!(events[1]["payload"]["gpu_id"], "GPU-456");
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

#[test]
fn a_user_outside_the_admin_group_is_welcomed_read_only_with_the_group_they_lack() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));

    let bob = daemon.read_only_user(1001, "bob");

    let welcome = bob.welcome();
    assert_eq!(welcome.protocol, ProtocolVersion::CURRENT);
    assert!(!welcome.can_tune);
    assert_eq!(welcome.reason.as_deref(), Some("bob isn't in group admins"));
}

#[test]
fn a_read_only_session_gets_the_snapshot_events_telemetry_and_every_read() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let bob = daemon.read_only_user(1001, "bob");

    assert_eq!(bob.snapshot().inventory.gpus[0].id, "GPU-123");
    assert_eq!(bob.snapshot().tuning_states[0].gpu_id, "GPU-123");
    assert_eq!(
        bob.send_json(json!({ "type": "subscribe_telemetry" }))["payload"]["success"],
        true
    );
    daemon.gpus.arrive(tunable_gpu_with_id("GPU-456"));
    daemon.tick_telemetry();
    let kinds: Vec<_> = bob
        .pushed_events()
        .into_iter()
        .map(|event| event["type"].clone())
        .collect();
    assert_eq!(
        kinds,
        [
            "inventory_changed",
            "tuning_state_changed",
            "telemetry_sampled"
        ]
    );

    for request in [
        json!({ "type": "get_inventory" }),
        json!({ "type": "get_daemon_status" }),
        json!({ "type": "get_tuning_states" }),
        json!({ "type": "get_startup_state" }),
        json!({ "type": "list_profiles" }),
        json!({ "type": "get_games" }),
        json!({ "type": "get_running_games" }),
        json!({ "type": "list_steam_apps" }),
        json!({ "type": "list_running_programs" }),
        json!({ "type": "get_automatic_switching_status" }),
        json!({ "type": "get_driver_status" }),
        json!({ "type": "preview_fan_curve", "payload": {
            "gpu_id": "GPU-123", "points": [
                { "temperature_c": 40, "speed_percent": 40 },
                { "temperature_c": 70, "speed_percent": 100 }
            ]
        } }),
    ] {
        // Some reads fail on the fake host, such as listing programs without a /proc, but none
        // is refused.
        let response = bob.send_json(request.clone());
        assert_ne!(
            response["payload"]["code"], "no_permission",
            "{request} → {response}"
        );
    }
}

#[test]
fn every_write_from_a_read_only_session_is_refused_naming_the_user_and_group() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    alice.seed_profiles(json!([{
        "id": "quiet", "name": "Quiet", "gpu_id": "GPU-123",
        "settings": { "power_limit_watts": { "kind": "value", "value": 300 } }
    }]));
    let bob = daemon.read_only_user(1001, "bob");
    let revision = bob.snapshot().tuning_states[0].revision.clone();
    let store = daemon.persistent_file(super::tuning_store::TUNING_STORE_FILE_NAME);
    let steam = json!({ "kind": "steam", "app_id": 570 });

    for request in [
        json!({ "type": "set_edit_holds", "payload": { "gpu_ids": ["GPU-123"] } }),
        json!({ "type": "apply_profile", "payload": {
            "gpu_id": "GPU-123", "settings": { "power_limit_watts": { "kind": "default" } },
            "expected_revision": revision
        } }),
        json!({ "type": "set_startup_behavior", "payload": { "behavior": "startup_profiles" } }),
        json!({ "type": "set_startup_profile", "payload": {
            "gpu_id": "GPU-123", "profile_id": "quiet"
        } }),
        json!({ "type": "dismiss_startup_outcome", "payload": { "gpu_id": "GPU-123" } }),
        json!({ "type": "save_profile_as_new", "payload": {
            "gpu_id": "GPU-123", "name": "Mine", "revision": revision
        } }),
        json!({ "type": "save_to_current_profile", "payload": {
            "gpu_id": "GPU-123", "revision": revision
        } }),
        json!({ "type": "rename_profile", "payload": { "profile_id": "quiet", "name": "Silent" } }),
        json!({ "type": "delete_profile", "payload": { "profile_id": "quiet" } }),
        json!({ "type": "apply_profile_by_id", "payload": { "profile_id": "quiet" } }),
        json!({ "type": "correct_and_apply_profile", "payload": {
            "profile_id": "quiet", "corrections": []
        } }),
        json!({ "type": "add_program", "payload": { "path": "/usr/bin/game" } }),
        json!({ "type": "add_steam_game", "payload": { "app_id": 570 } }),
        json!({ "type": "set_game_profile", "payload": {
            "game": steam, "gpu_id": "GPU-123", "profile_id": "quiet"
        } }),
        json!({ "type": "remove_game", "payload": { "game": steam } }),
        json!({ "type": "set_automatic_switching", "payload": { "enabled": true } }),
        json!({ "type": "set_gpu_switching_paused", "payload": {
            "gpu_id": "GPU-123", "paused": true
        } }),
        json!({ "type": "retry_automatic_switching", "payload": { "gpu_id": "GPU-123" } }),
        json!({ "type": "start_driver_action", "payload": { "action": "install_recommended" } }),
        json!({ "type": "cancel_driver_job", "payload": { "job_id": "job-1" } }),
        json!({ "type": "request_host_reboot" }),
    ] {
        assert_eq!(
            bob.send_json(request.clone()),
            json!({ "type": "error", "payload": {
                "code": "no_permission", "message": "bob isn't in group admins"
            } }),
            "{request}"
        );
    }
    daemon.settle();
    assert_eq!(daemon.gpus.writes(), []);
    assert_eq!(
        daemon.persistent_file(super::tuning_store::TUNING_STORE_FILE_NAME),
        store
    );
    assert!(daemon.core().tuning_store.held_gpus().is_empty());
}
