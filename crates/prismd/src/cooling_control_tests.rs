//! Seam 1: Prism's control of each GPU's fans reaches sessions in their snapshot and as
//! `CoolingControlChanged` whenever it changes, each with the session's own remembered curve.

use serde_json::{Value, json};

use super::harness::{FakeGpuBackend, FakeSession, Harness, Setter, cooled_gpu};

fn curve() -> Value {
    json!({"arrangement": "shared", "setting": {"mode": "curve", "points": [
        {"temperature_c": 40, "speed_percent": 30},
        {"temperature_c": 80, "speed_percent": 100}
    ]}})
}

fn fixed(speed_percent: i64) -> Value {
    json!({"arrangement": "shared", "setting": {"mode": "fixed", "speed_percent": speed_percent}})
}

fn apply(session: &FakeSession, configuration: Value) {
    let response = session.apply("GPU-123", json!({ "cooling": configuration }));
    assert_eq!(
        response["payload"]["cooling"]["outcome"]["status"], "succeeded",
        "{response}"
    );
}

/// The cooling controls pushed to the session since it last looked.
fn pushed_controls(session: &FakeSession) -> Vec<Value> {
    session
        .pushed_events()
        .into_iter()
        .filter(|event| event["type"] == "cooling_control_changed")
        .map(|event| event["payload"].clone())
        .collect()
}

fn snapshot_controls(session: &FakeSession) -> Value {
    serde_json::to_value(&session.snapshot().cooling_controls).unwrap()
}

#[test]
fn the_snapshot_carries_each_gpus_cooling_control_with_the_callers_remembered_curve() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let untouched = daemon.user(1000, "alice");
    assert_eq!(
        snapshot_controls(&untouched),
        json!([{ "gpu_id": "GPU-123", "revision": 0, "applied": null }])
    );
    apply(&untouched, curve());
    apply(&untouched, fixed(55));

    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");

    let control = &snapshot_controls(&alice)[0];
    assert_eq!(control["applied"], fixed(55));
    assert_eq!(control["remembered_curve"], curve());
    assert_eq!(control.get("suspension"), None);
    assert_eq!(snapshot_controls(&bob)[0].get("remembered_curve"), None);
    assert_eq!(snapshot_controls(&bob)[0]["applied"], fixed(55));
}

#[test]
fn applying_cooling_pushes_the_new_control_to_every_session_with_its_own_remembered_curve() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");

    apply(&alice, curve());

    let to_alice = pushed_controls(&alice);
    assert_eq!(to_alice.len(), 1, "{to_alice:?}");
    assert_eq!(to_alice[0]["gpu_id"], "GPU-123");
    assert_eq!(to_alice[0]["applied"], curve());
    assert_eq!(to_alice[0]["remembered_curve"], curve());
    let to_bob = pushed_controls(&bob);
    assert_eq!(to_bob.len(), 1, "{to_bob:?}");
    assert_eq!(to_bob[0]["applied"], curve());
    assert_eq!(to_bob[0].get("remembered_curve"), None);
    // The curve's changing output comes with telemetry, not with its control.
    assert_eq!(to_alice[0].get("requested"), None);
}

#[test]
fn a_cooling_fault_in_the_curve_loop_pushes_the_suspension_and_its_fallback() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let alice = daemon.user(1000, "alice");
    apply(&alice, curve());
    pushed_controls(&alice);

    daemon.set_temperature("GPU-123", None);
    daemon.run_curves_for(500);

    let pushed = pushed_controls(&alice);
    assert_eq!(pushed.len(), 1, "{pushed:?}");
    let suspension = &pushed[0]["suspension"];
    assert_eq!(suspension["kind"], "fault");
    assert!(
        suspension["reason"]
            .as_str()
            .unwrap()
            .contains("temperature is unavailable"),
        "{suspension}"
    );
    assert_eq!(suspension["restoration"][0]["status"], "restored");
    // The curve stays applied, to resume on an explicit retry.
    assert_eq!(pushed[0]["applied"], curve());
    assert_eq!(pushed[0]["remembered_curve"], curve());
}

#[test]
fn a_curve_following_the_temperature_pushes_no_cooling_control_change() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let alice = daemon.user(1000, "alice");
    apply(&alice, curve());
    pushed_controls(&alice);

    daemon.set_temperature("GPU-123", Some(80));
    daemon.run_curves_for(1_000);
    daemon.tick_telemetry();

    assert_eq!(pushed_controls(&alice), Vec::<Value>::new());
}

#[test]
fn cooling_restored_at_startup_pushes_its_control_to_a_session_that_connected_meanwhile() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let alice = daemon.user(1000, "alice");
    alice.seed_profiles(json!([{
        "id": "cool", "name": "Cool", "gpu_id": "GPU-123",
        "settings": { "cooling": fixed(55) }
    }]));
    for request in [
        json!({ "type": "set_startup_profile", "payload": { "gpu_id": "GPU-123", "profile_id": "cool" } }),
        json!({ "type": "set_startup_behavior", "payload": { "behavior": "startup_profiles" } }),
    ] {
        let response = alice.send_json(request);
        assert_ne!(response["type"], "error", "{response}");
    }
    drop(alice);
    let gate = daemon.gpus.block("GPU-123", Setter::FanSpeed);

    daemon.reboot();
    gate.wait_until_entered();
    let alice = daemon.user(1000, "alice");
    assert_eq!(snapshot_controls(&alice)[0]["applied"], Value::Null);
    gate.release();
    daemon.core().wait_for_startup();

    let pushed = pushed_controls(&alice);
    assert_eq!(pushed.len(), 1, "{pushed:?}");
    assert_eq!(pushed[0]["applied"], fixed(55));
}
