//! Seam 1: staged Apply. One `ApplyProfile` per GPU, based on the tuning-state revision its draft
//! started from, and current tuning values pushed whenever they change, even from outside Prism.

use serde_json::{Value, json};

use super::harness::{FakeGpuBackend, FakeSession, Harness, tunable_gpu};
use prism_protocol::DaemonRequest;

fn power_limit(watts: u32) -> Value {
    json!({ "power_limit_watts": { "kind": "value", "value": watts } })
}

fn apply(session: &FakeSession, settings: Value, revision: &str) -> Value {
    session.send_json(json!({ "type": "apply_profile", "payload": {
        "gpu_id": "GPU-123", "settings": settings, "expected_revision": revision
    } }))
}

fn tuning_state(session: &FakeSession) -> Value {
    let states = session.send_json(json!({ "type": "get_tuning_states" }));
    states["payload"][0].clone()
}

fn revision(session: &FakeSession) -> String {
    tuning_state(session)["revision"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// The tuning states pushed to the session since it last looked.
fn pushed_tuning_states(session: &FakeSession) -> Vec<Value> {
    session
        .pushed_events()
        .into_iter()
        .filter(|event| event["type"] == "tuning_state_changed")
        .map(|event| event["payload"].clone())
        .collect()
}

#[test]
fn an_apply_puts_its_settings_into_effect_and_records_them_as_last_applied_values() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");

    let response = apply(&alice, power_limit(450), &revision(&alice));

    assert_eq!(
        response["payload"]["power"]["status"], "succeeded",
        "{response}"
    );
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(450)
    );
    // A manual write, not a profile: no profile describes the GPU.
    assert_eq!(tuning_state(&alice)["current_profile"], Value::Null);
    let restore = alice.send_json(json!({
        "type": "set_startup_behavior", "payload": { "behavior": "restore_last_applied" }
    }));
    assert_eq!(restore["payload"]["success"], true, "{restore}");
    drop(alice);
    daemon
        .gpus
        .update("GPU-123", |gpu| gpu.power.current_limit_watts = Some(575));
    daemon.reboot();
    daemon.core().wait_for_startup();

    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(450)
    );
}

#[test]
fn an_apply_based_on_a_stale_revision_is_rejected_for_review_and_writes_nothing() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    let seen = revision(&alice);
    // Changed outside Prism after the draft started, as `nvidia-smi -pl 500` would.
    daemon
        .gpus
        .update("GPU-123", |gpu| gpu.power.current_limit_watts = Some(500));

    let response = apply(&alice, power_limit(450), &seen);

    assert_eq!(response["type"], "error", "{response}");
    assert_eq!(response["payload"]["code"], "stale_revision", "{response}");
    assert!(daemon.gpus.writes().is_empty());
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(500)
    );

    let reviewed = apply(&alice, power_limit(450), &revision(&alice));

    assert_eq!(
        reviewed["payload"]["power"]["status"], "succeeded",
        "{reviewed}"
    );
}

#[test]
fn a_power_limit_changed_outside_prism_is_pushed_within_a_tick() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    alice.send(DaemonRequest::SubscribeTelemetry);
    let before = revision(&alice);
    daemon.tick_telemetry();
    assert_eq!(pushed_tuning_states(&alice), Vec::<Value>::new());

    daemon
        .gpus
        .update("GPU-123", |gpu| gpu.power.current_limit_watts = Some(400));
    daemon.tick_telemetry();

    let pushed = pushed_tuning_states(&alice);
    assert_eq!(pushed.len(), 1, "{pushed:?}");
    assert_eq!(pushed[0]["reported"]["power_limit_watts"], 400);
    assert_ne!(pushed[0]["revision"], before.as_str());
    assert_eq!(pushed[0]["revision"], revision(&alice).as_str());
    daemon.tick_telemetry();
    assert_eq!(pushed_tuning_states(&alice), Vec::<Value>::new());
}

#[test]
fn an_apply_pushes_its_new_tuning_state_once() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    alice.send(DaemonRequest::SubscribeTelemetry);
    daemon.tick_telemetry();
    alice.pushed_events();

    apply(&alice, power_limit(450), &revision(&alice));
    daemon.tick_telemetry();

    let pushed = pushed_tuning_states(&alice);
    assert_eq!(pushed.len(), 1, "{pushed:?}");
    assert_eq!(pushed[0]["reported"]["power_limit_watts"], 450);
}

#[test]
fn an_apply_whose_reviewed_fans_changed_writes_no_cooling() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(
        vec![super::harness::cooled_gpu()],
    ));
    let alice = daemon.user(1000, "alice");

    let response = alice.send_json(json!({ "type": "apply_profile", "payload": {
        "gpu_id": "GPU-123",
        "settings": { "cooling": { "arrangement": "shared", "setting": { "mode": "fixed", "speed_percent": 50 } } },
        "expected_revision": revision(&alice),
        "fan_indexes": [0]
    } }));

    assert_eq!(
        response["payload"]["cooling"]["outcome"]["status"], "unattempted",
        "{response}"
    );
    assert!(daemon.gpus.writes().is_empty());
}
