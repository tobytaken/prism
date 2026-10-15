//! Curve control depends on the boot-scoped cooling record (ADR 0006): whoever runs after the
//! daemon stops returns the fans it lists to automatic cooling. A curve never takes its fans
//! unless that record was saved first.

use serde_json::{Value, json};

use super::harness::{FakeGpuBackend, FakeSession, Harness, Setter, cooled_gpu};
use prism_gpu::FanControlMode;

const STARTER: [(i64, i64); 4] = [(45, 20), (65, 60), (75, 80), (85, 100)];
const UNRECORDED: &str = "Prism couldn't save the record it needs to return these fans to automatic cooling if the daemon stops.";
const STALLED: &str =
    "Prism's fan curve control stopped responding, so the daemon stopped following this curve.";

fn shared(setting: Value) -> Value {
    json!({ "arrangement": "shared", "setting": setting })
}

fn shared_curve() -> Value {
    let points: Vec<Value> = STARTER
        .iter()
        .map(|(temperature, speed)| json!({ "temperature_c": temperature, "speed_percent": speed }))
        .collect();
    shared(json!({ "mode": "curve", "points": points }))
}

fn apply(daemon: &Harness, configuration: Value) -> Value {
    let response = daemon.apply("GPU-123", json!({ "cooling": configuration }));
    assert_eq!(response["type"], "profile_application", "{response}");
    response["payload"]["cooling"].clone()
}

fn control(daemon: &Harness) -> Value {
    daemon.performance_json()["gpus"][0]["cooling"]["control"].clone()
}

fn modes(daemon: &Harness) -> Vec<Option<FanControlMode>> {
    let gpu = daemon.gpus.gpu("GPU-123");
    gpu.cooling.fans.iter().map(|fan| fan.mode).collect()
}

const AUTOMATIC: [Option<FanControlMode>; 2] = [Some(FanControlMode::Auto); 2];

fn fan_writes(daemon: &Harness) -> usize {
    daemon
        .gpus
        .writes()
        .iter()
        .filter(|(_, setter)| matches!(setter, Setter::FanSpeed | Setter::FanModeAuto))
        .count()
}

fn speed_writes(daemon: &Harness) -> usize {
    daemon
        .gpus
        .writes()
        .iter()
        .filter(|(_, setter)| *setter == Setter::FanSpeed)
        .count()
}

/// Makes saving the cooling record fail, as a full or read-only boot-state directory would.
fn block_record(daemon: &Harness) {
    std::fs::create_dir(daemon.boot_path("cooling-control.json.tmp")).unwrap();
}

fn unblock_record(daemon: &Harness) {
    std::fs::remove_dir(daemon.boot_path("cooling-control.json.tmp")).unwrap();
}

/// Asserts the application failed before any fan command, explaining the unsaved record.
fn assert_unrecorded(application: &Value) {
    assert_eq!(application["outcome"]["status"], "failed", "{application}");
    let error = application["outcome"]["error"].as_str().unwrap();
    assert!(error.starts_with(UNRECORDED), "{error}");
    for fan in application["fans"].as_array().unwrap() {
        assert_eq!(fan["outcome"]["status"], "unattempted", "{application}");
    }
    assert_eq!(application["fans"].as_array().unwrap().len(), 2);
}

#[test]
fn a_curve_without_a_recovery_record_must_not_leave_manual_fans_after_a_crash() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    block_record(&daemon);

    let application = apply(&daemon, shared_curve());

    assert_unrecorded(&application);
    let error = application["outcome"]["error"].as_str().unwrap();
    let record = daemon.boot_path("cooling-control.json");
    let boot_dir = record.parent().unwrap().display().to_string();
    assert!(
        error.contains(&boot_dir),
        "the error doesn't point to the boot-state directory: {error}"
    );
    daemon.crash_daemon();
    assert_eq!(modes(&daemon), AUTOMATIC);
}

#[test]
fn an_unrecorded_curve_sends_no_fan_command_and_keeps_the_applied_configuration() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let session: FakeSession = daemon.user(1000, "alice");
    let fixed = shared(json!({ "mode": "fixed", "speed_percent": 40 }));
    apply(&daemon, fixed.clone());
    let before = control(&daemon);
    let writes = fan_writes(&daemon);
    block_record(&daemon);

    let response = session.apply("GPU-123", json!({ "cooling": shared_curve() }));
    let application = &response["payload"]["cooling"];

    assert_unrecorded(application);
    assert_eq!(application["revision"], before["revision"]);
    assert_eq!(fan_writes(&daemon), writes);
    assert_eq!(control(&daemon), before);
    assert_eq!(control(&daemon)["applied"], fixed);
    assert_eq!(
        session.performance_json()["gpus"][0]["cooling"]["control"]["remembered_curve"],
        Value::Null
    );
}

#[test]
fn fixed_and_automatic_cooling_apply_without_a_recovery_record() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    block_record(&daemon);

    let fixed = apply(
        &daemon,
        shared(json!({ "mode": "fixed", "speed_percent": 55 })),
    );
    assert_eq!(fixed["outcome"]["status"], "succeeded", "{fixed}");
    let automatic = apply(&daemon, shared(json!({ "mode": "automatic" })));
    assert_eq!(automatic["outcome"]["status"], "succeeded", "{automatic}");
}

#[test]
fn without_a_boot_id_curves_fail_and_fixed_or_automatic_cooling_still_applies() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    daemon.restart_without_boot_id();
    let writes = fan_writes(&daemon);

    assert_unrecorded(&apply(&daemon, shared_curve()));
    assert_eq!(fan_writes(&daemon), writes);
    assert_eq!(control(&daemon)["applied"], Value::Null);

    let fixed = apply(
        &daemon,
        shared(json!({ "mode": "fixed", "speed_percent": 55 })),
    );
    assert_eq!(fixed["outcome"]["status"], "succeeded", "{fixed}");
    let automatic = apply(&daemon, shared(json!({ "mode": "automatic" })));
    assert_eq!(automatic["outcome"]["status"], "succeeded", "{automatic}");
}

#[test]
fn a_resumption_without_a_recovery_record_leaves_the_fans_automatic_until_a_retry() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve());
    daemon.stop_daemon();
    let writes = speed_writes(&daemon);
    block_record(&daemon);

    daemon.start_daemon();

    assert_eq!(modes(&daemon), AUTOMATIC);
    assert_eq!(speed_writes(&daemon), writes);
    let control = control(&daemon);
    assert_eq!(control["requested"], json!([]));
    assert_eq!(control["suspension"]["kind"], "fault");
    let reason = control["suspension"]["reason"].as_str().unwrap();
    assert!(reason.contains(UNRECORDED), "{reason}");
    assert_eq!(control["recovery"]["status"], "retry_required");
}

#[test]
fn a_failed_save_after_a_curve_owns_its_fans_still_leaves_them_to_recovery() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve());
    block_record(&daemon);

    // Taking the fans off the curve can't be saved, so the record still lists them.
    let fixed = apply(
        &daemon,
        shared(json!({ "mode": "fixed", "speed_percent": 55 })),
    );
    assert_eq!(fixed["outcome"]["status"], "succeeded", "{fixed}");
    daemon.crash_daemon();

    assert_eq!(modes(&daemon), AUTOMATIC);
}

#[test]
fn an_unrecorded_stall_returns_the_curve_fans_to_automatic_cooling_as_a_fault() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve());
    block_record(&daemon);

    daemon.core().record_cooling_stall(true);

    assert_eq!(modes(&daemon), AUTOMATIC);
    let suspended = control(&daemon);
    assert_eq!(suspended["requested"], json!([]));
    assert_eq!(suspended["suspension"]["kind"], "fault");
    assert_eq!(suspended["suspension"]["reason"], STALLED);
    // The curve loop doesn't bring it back.
    daemon.run_curves_for(1_000);
    assert_eq!(modes(&daemon), AUTOMATIC);

    // With no curve left running, the supervisor sees progress again. Once the record can be
    // saved, it still records the stall, so the next daemon of this boot doesn't resume the
    // curve whose fans it lists.
    daemon.core().record_cooling_stall(false);
    unblock_record(&daemon);
    daemon.core().record_cooling_stall(false);
    let record: Value =
        serde_json::from_slice(&std::fs::read(daemon.boot_path("cooling-control.json")).unwrap())
            .unwrap();
    assert_eq!(record["stalled"], true, "{record}");
    daemon.restart();

    assert_eq!(modes(&daemon), AUTOMATIC);
    assert_eq!(control(&daemon)["requested"], json!([]));
    assert_eq!(control(&daemon)["suspension"]["reason"], STALLED);
}

#[test]
fn a_stall_never_recorded_isnt_resumed_while_the_record_stays_unwritable() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve());
    block_record(&daemon);

    daemon.core().record_cooling_stall(true);
    daemon.crash_daemon();

    assert_eq!(modes(&daemon), AUTOMATIC);
    assert_eq!(control(&daemon)["requested"], json!([]));
}
