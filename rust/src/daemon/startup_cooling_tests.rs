//! Seam 1: cooling through the existing host-wide startup behavior, before any user logs in.

use serde_json::{json, Value};

use super::boot_state::STARTUP_IN_PROGRESS;
use super::cooling::CoolingApplicationSource;
use super::harness::{cooled_gpu, FakeGpuBackend, FakeSession, Harness, Setter};
use crate::gpu::{FanControlMode, GpuPerformanceDevice};

fn send(session: &FakeSession, request: Value) -> Value {
    let response = session.send_json(request);
    assert_ne!(response["type"], "error", "{response}");
    response["payload"].clone()
}

fn curve() -> Value {
    json!({"mode": "curve", "points": [
        {"temperatureC": 40, "speedPercent": 30},
        {"temperatureC": 80, "speedPercent": 100}
    ]})
}

fn mixed() -> Value {
    json!({"arrangement": "perFan", "fans": [
        {"fanIndex": 0, "setting": curve()},
        {"fanIndex": 1, "setting": {"mode": "fixed", "speedPercent": 55}}
    ]})
}

fn seed(session: &FakeSession, cooling: Value) {
    session.seed_profiles(json!([{
        "id": "cool", "name": "Cool", "gpuId": "GPU-123", "settings": {
            "powerLimitWatts": { "kind": "value", "value": 300 },
            "cooling": cooling
        }
    }]));
}

fn use_startup_profile(session: &FakeSession) {
    send(
        session,
        json!({ "type": "set_startup_profile", "payload": { "gpuId": "GPU-123", "profileId": "cool" } }),
    );
    set_behavior(session, "startupProfiles");
}

fn set_behavior(session: &FakeSession, behavior: &str) {
    let response = send(
        session,
        json!({ "type": "set_startup_behavior", "payload": { "behavior": behavior } }),
    );
    assert_eq!(response["success"], true, "{response}");
}

fn apply_cooling(session: &FakeSession, configuration: Value) {
    let response = send(
        session,
        json!({"type": "apply_cooling", "payload": {
            "gpuId": "GPU-123", "configuration": configuration
        }}),
    );
    assert_eq!(response["outcome"]["status"], "succeeded", "{response}");
}

/// The startup outcome for GPU-123 once startup has finished with it.
fn completed_outcome(daemon: &Harness) -> Value {
    let session = daemon.user(1000, "alice");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let state = send(&session, json!({ "type": "get_startup_state" }));
        if let Some(outcome) = state["outcomes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|outcome| outcome["gpuId"] == "GPU-123")
            .filter(|outcome| outcome["reason"] != STARTUP_IN_PROGRESS)
        {
            return outcome.clone();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "startup did not finish: {state}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn control(daemon: &Harness) -> Value {
    daemon.send_json(json!({"type": "get_performance"}))["payload"]["gpus"][0]["cooling"]["control"]
        .clone()
}

fn fan(daemon: &Harness, index: usize) -> (Option<FanControlMode>, Option<u32>) {
    let fan = &daemon.gpus.gpu("GPU-123").cooling.fans[index];
    (fan.mode, fan.current_speed_percent)
}

fn tuning_state(daemon: &Harness) -> Value {
    send(
        &daemon.user(1000, "alice"),
        json!({ "type": "get_tuning_states" }),
    )[0]
    .clone()
}

#[test]
fn a_startup_profile_runs_its_complete_cooling_from_the_fresh_temperature_once_per_boot() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let alice = daemon.user(1000, "alice");
    seed(&alice, mixed());
    use_startup_profile(&alice);
    drop(alice);
    daemon.set_temperature("GPU-123", Some(60));
    daemon.reboot();
    let outcome = completed_outcome(&daemon);
    assert_eq!(outcome["status"], "applied", "{outcome}");
    assert_eq!(outcome["profile"]["name"], "Cool");
    assert_eq!(
        outcome["application"]["cooling"]["outcome"]["status"],
        "succeeded"
    );
    // The curve requests its speed at the temperature read at boot.
    assert_eq!(fan(&daemon, 0), (Some(FanControlMode::Manual), Some(65)));
    assert_eq!(fan(&daemon, 1), (Some(FanControlMode::Manual), Some(55)));
    assert_eq!(control(&daemon)["applied"], mixed());
    let state = tuning_state(&daemon);
    assert_eq!(state["currentProfile"]["id"], "cool");
    assert_eq!(state["reported"]["cooling"], mixed());
    // The daemon keeps following the curve with no app running.
    daemon.set_temperature("GPU-123", Some(80));
    daemon.run_curves_for(500);
    assert_eq!(fan(&daemon, 0), (Some(FanControlMode::Manual), Some(100)));

    // A daemon restart resumes the curve but is not a new boot, so startup doesn't run again.
    daemon
        .gpus
        .update("GPU-123", |gpu| gpu.power.current_limit_watts = Some(400));
    daemon.restart();
    assert_eq!(completed_outcome(&daemon), outcome);
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(400)
    );
    assert_eq!(control(&daemon)["applied"], mixed());
}

#[test]
fn restoring_last_applied_values_restores_the_latest_manual_cooling_configuration() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let alice = daemon.user(1000, "alice");
    apply_cooling(
        &alice,
        json!({"arrangement": "shared", "setting": {"mode": "fixed", "speedPercent": 70}}),
    );
    let shared_curve = json!({"arrangement": "shared", "setting": curve()});
    apply_cooling(&alice, shared_curve.clone());
    set_behavior(&alice, "restoreLastApplied");
    drop(alice);
    // Ordinary curve output is not a manual value.
    daemon.set_temperature("GPU-123", Some(85));
    daemon.run_curves_for(500);
    assert_eq!(fan(&daemon, 0).1, Some(100));
    // Temporary automatic applications, such as a game's, never become last applied values.
    let temporary = daemon.core().apply_cooling(
        serde_json::from_value(json!({
            "gpuId": "GPU-123",
            "configuration": {"arrangement": "shared", "setting": {"mode": "fixed", "speedPercent": 90}}
        }))
        .unwrap(),
        CoolingApplicationSource::NonManual,
    );
    assert_eq!(
        serde_json::to_value(temporary).unwrap()["outcome"]["status"],
        "succeeded"
    );
    daemon.set_temperature("GPU-123", Some(60));
    // This host boot resets fan policy; the rollback baseline must be freshly readable.
    daemon.gpus.update("GPU-123", |gpu| {
        for fan in &mut gpu.cooling.fans {
            fan.mode = Some(FanControlMode::Auto);
        }
    });
    daemon.reboot();
    let outcome = completed_outcome(&daemon);
    assert_eq!(outcome["status"], "applied", "{outcome}");
    assert!(outcome["profile"].is_null());
    assert_eq!(control(&daemon)["applied"], shared_curve);
    for index in 0..2 {
        assert_eq!(
            fan(&daemon, index),
            (Some(FanControlMode::Manual), Some(65))
        );
    }
}

#[test]
fn startup_off_and_gpus_without_manual_cooling_leave_cooling_unchanged() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let alice = daemon.user(1000, "alice");
    apply_cooling(
        &alice,
        json!({"arrangement": "shared", "setting": {"mode": "fixed", "speedPercent": 70}}),
    );
    drop(alice);
    let writes = daemon.gpus.writes();
    daemon.reboot();
    assert_eq!(
        completed_outcome(&daemon)["reason"],
        "Startup behavior is Off."
    );
    assert_eq!(daemon.gpus.writes(), writes);
    assert_eq!(fan(&daemon, 0), (Some(FanControlMode::Manual), Some(70)));

    // Restoring a GPU whose last applied values include no cooling leaves its fans alone.
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let alice = daemon.user(1000, "alice");
    send(
        &alice,
        json!({ "type": "set_power_limit", "payload": { "gpuId": "GPU-123", "limitWatts": 300 } }),
    );
    set_behavior(&alice, "restoreLastApplied");
    drop(alice);
    daemon.reboot();
    assert_eq!(completed_outcome(&daemon)["status"], "applied");
    assert!(!daemon
        .gpus
        .writes()
        .iter()
        .any(|(_, setter)| matches!(setter, Setter::FanSpeed | Setter::FanModeAuto)));
}

/// What the GPU reports differently at the next boot.
type Change = fn(&mut GpuPerformanceDevice);

#[test]
fn unusable_saved_cooling_is_reported_at_startup_without_writing_or_clamping() {
    let cases: [(&str, Change); 4] = [
        ("operating temperature", |gpu| {
            gpu.cooling.max_operating_temperature_c = None
        }),
        ("20%", |gpu| {
            gpu.cooling.fans[0].max_speed_percent = Some(90)
        }),
        ("Fan 2 isn't reported", |gpu| {
            gpu.cooling.fans.truncate(1);
            gpu.cooling.fan_count = 1;
        }),
        ("temperature is unavailable", |gpu| {
            gpu.telemetry.temperature_c = None
        }),
    ];
    for (reason, change) in cases {
        let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
        let alice = daemon.user(1000, "alice");
        seed(&alice, mixed());
        use_startup_profile(&alice);
        drop(alice);
        daemon.gpus.update("GPU-123", change);
        let before = daemon.gpus.gpu("GPU-123");
        daemon.reboot();
        let outcome = completed_outcome(&daemon);
        assert_eq!(outcome["status"], "notAttempted", "{reason}: {outcome}");
        assert!(
            outcome["reason"].as_str().unwrap().contains(reason),
            "{reason}: {outcome}"
        );
        let application = &outcome["application"];
        assert_eq!(application["cooling"]["outcome"]["status"], "unattempted");
        assert_eq!(application["power"]["status"], "unattempted");
        assert!(application.get("corrections").is_none());
        assert!(daemon.gpus.writes().is_empty(), "{reason}");
        assert_eq!(daemon.gpus.gpu("GPU-123"), before, "{reason}");
    }
}

#[test]
fn a_rejected_fan_command_at_startup_is_reported_per_fan_and_suspends_the_curve() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let alice = daemon.user(1000, "alice");
    seed(&alice, json!({"arrangement": "shared", "setting": curve()}));
    use_startup_profile(&alice);
    drop(alice);
    daemon
        .gpus
        .fail_fan("GPU-123", Setter::FanSpeed, 1, "Fan rejected the speed.");
    daemon.reboot();
    let outcome = completed_outcome(&daemon);
    assert_eq!(outcome["status"], "failed", "{outcome}");
    assert!(outcome["reason"]
        .as_str()
        .unwrap()
        .contains("Fan rejected the speed."));
    let application = &outcome["application"];
    assert_eq!(application["power"]["status"], "unattempted");
    assert_eq!(application["cooling"]["outcome"]["status"], "failed");
    assert_eq!(
        application["cooling"]["fans"][1]["outcome"]["status"],
        "failed"
    );
    assert!(!control(&daemon)["suspension"].is_null());
    assert_eq!(fan(&daemon, 0).0, Some(FanControlMode::Auto));
    assert!(tuning_state(&daemon)["currentProfile"].is_null());
}

#[test]
fn a_crash_after_restoring_cooling_skips_the_next_restore_once() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let alice = daemon.user(1000, "alice");
    apply_cooling(&alice, json!({"arrangement": "shared", "setting": curve()}));
    set_behavior(&alice, "restoreLastApplied");
    drop(alice);
    daemon.gpus.update("GPU-123", |gpu| {
        for fan in &mut gpu.cooling.fans {
            fan.mode = Some(FanControlMode::Auto);
        }
    });
    daemon.reboot();
    assert_eq!(completed_outcome(&daemon)["status"], "applied");
    daemon.crash_and_reboot();
    let skipped = completed_outcome(&daemon);
    assert_eq!(skipped["status"], "notAttempted");
    assert!(skipped["reason"].as_str().unwrap().contains("skipped once"));
    assert!(control(&daemon)["applied"].is_null());
    daemon.gpus.update("GPU-123", |gpu| {
        for fan in &mut gpu.cooling.fans {
            fan.mode = Some(FanControlMode::Auto);
        }
    });
    daemon.reboot();
    assert_eq!(completed_outcome(&daemon)["status"], "applied");
    assert_eq!(
        control(&daemon)["applied"],
        json!({"arrangement": "shared", "setting": curve()})
    );
}
