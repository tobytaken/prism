//! Remembered choices through caller-scoped daemon requests, across session and host lifetimes.
use serde_json::{json, Value};

use super::cooling::CoolingApplicationSource;
use super::harness::{cooled_gpu, FakeGpuBackend, FakeSession, Harness, Setter};

fn curve() -> Value {
    json!({"arrangement": "shared", "setting": {"mode": "curve", "points": [
        {"temperatureC": 40, "speedPercent": 30},
        {"temperatureC": 80, "speedPercent": 100}
    ]}})
}

fn apply(session: &FakeSession, configuration: Value) -> Value {
    session.send_json(json!({"type": "apply_cooling", "payload": {
        "gpuId": "GPU-123", "configuration": configuration
    }}))["payload"]
        .clone()
}

fn remembered(session: &FakeSession) -> Value {
    session.send_json(json!({"type": "get_performance"}))["payload"]["gpus"][0]["cooling"]
        ["control"]["rememberedCurve"]
        .clone()
}

#[test]
fn remembered_curve_survives_modes_sessions_daemon_restart_and_reboot() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let alice = daemon.user(1000, "alice");
    assert_eq!(apply(&alice, curve())["outcome"]["status"], "succeeded");
    assert_eq!(remembered(&alice), curve());
    for setting in [
        json!({"mode": "automatic"}),
        json!({"mode": "fixed", "speedPercent": 55}),
    ] {
        assert_eq!(
            apply(&alice, json!({"arrangement": "shared", "setting": setting}))["outcome"]
                ["status"],
            "succeeded"
        );
        assert_eq!(remembered(&alice), curve());
    }
    drop(alice);
    assert_eq!(remembered(&daemon.user(1000, "alice")), curve());
    daemon.restart();
    assert_eq!(remembered(&daemon.user(1000, "alice")), curve());
    daemon.reboot();
    assert_eq!(remembered(&daemon.user(1000, "alice")), curve());
}

#[test]
fn remembered_curves_belong_to_the_account_and_individual_gpu() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let alice = daemon.user(1000, "alice");
    apply(&alice, curve());
    assert!(remembered(&daemon.user(1001, "bob")).is_null());
    assert!(remembered(&daemon.user(1000, "replacement-account")).is_null());
    assert!(remembered(&daemon.user(1002, "alice")).is_null());
    assert!(
        daemon.send_json(json!({"type": "get_performance"}))["payload"]["gpus"][0]["cooling"]
            ["control"]["rememberedCurve"]
            .is_null()
    );
    // Same model, same display index, different hardware identity.
    daemon.gpus.update("GPU-123", |gpu| {
        gpu.stable_id = Some("GPU-replacement".into())
    });
    assert!(remembered(&alice).is_null());
    daemon
        .gpus
        .update("GPU-123", |gpu| gpu.stable_id = Some("GPU-123".into()));
    assert_eq!(remembered(&alice), curve());
}

#[test]
fn a_successful_mixed_arrangement_is_remembered_in_full_and_replaces_the_old_choice() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let alice = daemon.user(1000, "alice");
    apply(&alice, curve());
    let mixed = json!({"arrangement": "perFan", "fans": [
        {"fanIndex": 0, "setting": curve()["setting"]},
        {"fanIndex": 1, "setting": {"mode": "fixed", "speedPercent": 55}}
    ]});
    assert_eq!(
        apply(&alice, mixed.clone())["outcome"]["status"],
        "succeeded"
    );
    assert_eq!(remembered(&alice), mixed);
    drop(alice);
    daemon.reboot();
    assert_eq!(remembered(&daemon.user(1000, "alice")), mixed);
}

#[test]
fn failed_applications_and_current_capability_validation_preserve_the_manual_choice() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let alice = daemon.user(1000, "alice");
    apply(&alice, curve());
    let mut edited = curve();
    edited["setting"]["points"][0]["speedPercent"] = json!(40);
    daemon
        .gpus
        .fail_fan("GPU-123", Setter::FanSpeed, 1, "fan rejected change");
    assert_eq!(apply(&alice, edited)["outcome"]["status"], "failed");
    assert_eq!(remembered(&alice), curve());
    daemon.gpus.update("GPU-123", |gpu| {
        gpu.cooling.fans[0].min_speed_percent = Some(50)
    });
    assert_eq!(
        apply(&alice, remembered(&alice))["outcome"]["status"],
        "unattempted"
    );
    assert_eq!(remembered(&alice), curve());
}

#[test]
fn automatic_applications_restorations_and_curve_ticks_never_replace_manual_history() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let alice = daemon.user(1000, "alice");
    apply(&alice, curve());
    let saved = daemon.persistent_file("tuning-state.json");
    daemon.set_temperature("GPU-123", Some(85));
    daemon.run_curves_for(500);
    assert_eq!(remembered(&alice), curve());
    assert_eq!(daemon.persistent_file("tuning-state.json"), saved);
    let mut temporary = curve();
    temporary["setting"]["points"][0]["speedPercent"] = json!(50);
    // The common controller application boundary consumed by game applications and restorations.
    // Neither identifies a manual owner, even if its configuration includes a custom curve.
    for configuration in [temporary, curve()] {
        let application = daemon.core().apply_cooling(
            serde_json::from_value(json!({
                "gpuId": "GPU-123", "configuration": configuration
            }))
            .unwrap(),
            CoolingApplicationSource::NonManual,
        );
        assert_eq!(
            serde_json::to_value(application).unwrap()["outcome"]["status"],
            "succeeded"
        );
        assert_eq!(remembered(&alice), curve());
        assert_eq!(daemon.persistent_file("tuning-state.json"), saved);
    }
}

#[test]
fn a_version_seven_store_migrates_without_inventing_a_remembered_curve() {
    let saved = br#"{"schemaVersion":7,"users":[{"uid":1000,"username":"alice","automaticSwitching":true}]}"#;
    let mut daemon = Harness::start_with_persistent_files(
        FakeGpuBackend::with_gpus(vec![cooled_gpu()]),
        &[("tuning-state.json", saved)],
    );
    let alice = daemon.user(1000, "alice");
    assert!(remembered(&alice).is_null());
    assert_eq!(apply(&alice, curve())["outcome"]["status"], "succeeded");
    drop(alice);
    daemon.restart();
    assert_eq!(remembered(&daemon.user(1000, "alice")), curve());
}

#[test]
fn an_unhealthy_store_does_not_block_manual_cooling_or_invent_persisted_history() {
    let saved = br#"{"schemaVersion":99}"#;
    let daemon = Harness::start_with_persistent_files(
        FakeGpuBackend::with_gpus(vec![cooled_gpu()]),
        &[("tuning-state.json", saved)],
    );
    let alice = daemon.user(1000, "alice");
    assert_eq!(apply(&alice, curve())["outcome"]["status"], "succeeded");
    assert!(remembered(&alice).is_null());
    assert_eq!(
        daemon.persistent_file("tuning-state.json").as_deref(),
        Some(saved.as_slice())
    );
}
