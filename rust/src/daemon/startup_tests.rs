//! Seam 1: host-wide startup choices and outcomes through identified daemon sessions.

use serde_json::{json, Value};

use super::harness::{tunable_gpu, FakeGpuBackend, FakeSession, Harness};

fn startup_gpu() -> crate::gpu::GpuPerformanceDevice {
    let mut gpu = tunable_gpu();
    gpu.power.min_limit_watts = Some(100);
    gpu
}

fn seed(session: &FakeSession, id: &str, name: &str, watts: u32) {
    session.seed_profiles(json!([{ "id": id, "name": name, "gpuId": "GPU-123",
        "settings": { "powerLimitWatts": { "kind": "value", "value": watts } } }]));
}

fn startup(session: &FakeSession) -> Value {
    let response = session.send_json(json!({ "type": "get_startup_state" }));
    assert_eq!(response["type"], "startup_state", "{response}");
    response["payload"].clone()
}

fn designate(session: &FakeSession, id: Option<&str>) -> Value {
    session.send_json(json!({ "type": "set_startup_profile", "payload": {
        "gpuId": "GPU-123", "profileId": id
    } }))
}

#[test]
fn startup_choices_are_host_wide_latest_wins_and_other_users_see_the_profile_by_name() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![startup_gpu()]));
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");
    assert_eq!(startup(&alice)["behavior"], "off");
    seed(&alice, "quiet", "Quiet", 300);
    seed(&bob, "silent", "Silent", 250);
    assert_eq!(designate(&alice, Some("quiet"))["payload"]["success"], true);
    assert_eq!(designate(&bob, Some("silent"))["payload"]["success"], true);
    assert_eq!(
        startup(&alice)["profiles"],
        json!([{
            "gpuId": "GPU-123", "profile": { "id": "silent", "name": "Silent" }
        }])
    );
    assert_eq!(
        alice.send_json(json!({ "type": "list_profiles" }))["payload"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(designate(&alice, Some("silent"))["type"], "error");
    assert_eq!(designate(&alice, None)["payload"]["success"], true);
    assert_eq!(startup(&bob)["profiles"], json!([]));
    assert!(daemon.gpus.writes().is_empty());
}

fn enable(session: &FakeSession) {
    let response = session.send_json(json!({ "type": "set_startup_behavior", "payload": {
        "behavior": "startupProfiles"
    } }));
    assert_eq!(response["payload"]["success"], true, "{response}");
}

fn completed_startup(daemon: &Harness) -> Value {
    let session = daemon.user(1000, "alice");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let state = startup(&session);
        if state["outcomes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|outcome| outcome["reason"] != "Startup is in progress.")
        {
            return state;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "startup did not finish: {state}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn startup_applies_without_an_app_once_per_boot_id_and_sets_the_current_profile() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![startup_gpu()]));
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");
    seed(&alice, "quiet", "Quiet", 300);
    seed(&bob, "silent", "Silent", 250);
    designate(&alice, Some("quiet"));
    designate(&bob, Some("silent"));
    enable(&alice);
    drop(alice);
    drop(bob);
    daemon.reboot();
    let outcome = completed_startup(&daemon);
    assert_eq!(outcome["outcomes"][0]["status"], "applied");
    assert_eq!(outcome["outcomes"][0]["profile"]["name"], "Silent");
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(250)
    );
    let alice = daemon.user(1000, "alice");
    assert_eq!(
        alice.send_json(json!({ "type": "get_tuning_states" }))["payload"][0]["currentProfile"],
        json!({ "id": "silent", "name": "Silent" })
    );
    drop(alice);
    let writes = daemon.gpus.writes();
    daemon
        .gpus
        .update("GPU-123", |gpu| gpu.power.current_limit_watts = Some(400));
    daemon.restart();
    assert_eq!(completed_startup(&daemon), outcome);
    assert_eq!(daemon.gpus.writes(), writes);
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(400)
    );
    daemon.reboot_with_retained_boot_state();
    completed_startup(&daemon);
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(250)
    );
    assert_eq!(daemon.gpus.writes().len(), writes.len() + 1);
}

#[test]
fn startup_validates_every_setting_without_writing_or_clamping_and_reports_why() {
    for unsupported in [false, true] {
        let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![startup_gpu()]));
        let alice = daemon.user(1000, "alice");
        alice.seed_profiles(json!([{
            "id": "quiet", "name": "Quiet", "gpuId": "GPU-123", "settings": {
                "powerLimitWatts": { "kind": "value", "value": 300 },
                "memoryOffsetMHz": { "kind": "value", "value": 100 }
            }
        }]));
        designate(&alice, Some("quiet"));
        enable(&alice);
        drop(alice);
        daemon.gpus.update("GPU-123", |gpu| {
            if unsupported {
                gpu.clocks.memory.support = crate::gpu::ControlSupport::Unsupported;
            } else {
                gpu.clocks.memory.max_offset_mhz = Some(80);
            }
        });
        let before = daemon.gpus.gpu("GPU-123");
        daemon.reboot();
        let state = completed_startup(&daemon);
        let outcome = &state["outcomes"][0];
        assert_eq!(outcome["status"], "notAttempted");
        assert!(outcome["reason"]
            .as_str()
            .unwrap()
            .contains(if unsupported {
                "unsupported"
            } else {
                "outside"
            }));
        assert_eq!(outcome["application"]["power"]["status"], "unattempted");
        // Boot outcomes are host-wide; the profile's saved values stay private.
        let bob = daemon.user(1001, "bob");
        let shared = startup(&bob);
        assert!(shared["outcomes"][0]["application"]
            .get("corrections")
            .is_none());
        assert!(!shared["outcomes"].to_string().contains("requested 100"));
        drop(bob);
        assert!(daemon.gpus.writes().is_empty());
        assert_eq!(daemon.gpus.gpu("GPU-123"), before);
        if !unsupported {
            let alice = daemon.user(1000, "alice");
            let corrected = alice.send_json(json!({ "type": "correct_and_apply_profile", "payload": {
                "profileId": "quiet", "corrections": [{ "setting": "memoryOffsetMHz", "savedValue": 100, "proposedValue": 80, "bound": "maximum" }]
            } }));
            assert_eq!(
                corrected["payload"]["application"]["memory"]["status"],
                "succeeded"
            );
            drop(alice);
            daemon.reboot();
            assert_eq!(
                completed_startup(&daemon)["outcomes"][0]["status"],
                "applied"
            );
            assert_eq!(
                daemon.gpus.gpu("GPU-123").clocks.memory.current_offset_mhz,
                Some(80)
            );
        }
    }
}

#[test]
fn a_partial_startup_clears_the_current_profile_and_reports_each_setting() {
    use super::harness::Setter;
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![startup_gpu()]));
    let alice = daemon.user(1000, "alice");
    alice.seed_profiles(json!([{
        "id": "quiet", "name": "Quiet", "gpuId": "GPU-123", "settings": {
            "powerLimitWatts": { "kind": "value", "value": 300 },
            "graphicsOffsetMHz": { "kind": "value", "value": 50 },
            "memoryOffsetMHz": { "kind": "value", "value": 100 }
        }
    }]));
    alice.send_json(json!({ "type": "apply_profile_by_id", "payload": { "profileId": "quiet" } }));
    designate(&alice, Some("quiet"));
    enable(&alice);
    drop(alice);
    daemon.gpus.fail(
        "GPU-123",
        Setter::GraphicsOffset,
        "Driver rejected the offset.",
    );
    daemon.reboot();
    let state = completed_startup(&daemon);
    assert_eq!(state["outcomes"][0]["status"], "failed");
    let application = &state["outcomes"][0]["application"];
    assert_eq!(application["power"]["status"], "succeeded");
    assert_eq!(application["graphics"]["status"], "failed");
    assert_eq!(
        application["graphics"]["error"],
        "Driver rejected the offset."
    );
    assert_eq!(application["memory"]["status"], "unattempted");
    let alice = daemon.user(1000, "alice");
    assert_eq!(
        alice.send_json(json!({ "type": "get_tuning_states" }))["payload"][0]["currentProfile"],
        Value::Null
    );
}

#[test]
fn a_late_gpu_gets_startup_before_manual_writes_while_reads_and_other_gpus_remain_available() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![startup_gpu()]));
    let alice = daemon.user(1000, "alice");
    seed(&alice, "quiet", "Quiet", 300);
    designate(&alice, Some("quiet"));
    enable(&alice);
    drop(alice);
    daemon.gpus.depart("GPU-123");
    daemon.reboot();
    assert!(daemon.performance().gpus.is_empty());
    let mut manual = daemon.send_in_background(
        serde_json::from_value(json!({
            "type": "set_power_limit", "payload": { "gpuId": "GPU-123", "limitWatts": 400 }
        }))
        .unwrap(),
    );
    assert!(manual.is_pending());
    daemon
        .gpus
        .arrive(super::harness::tunable_gpu_with_id("GPU-456"));
    let other = daemon.send_json(
        json!({ "type": "set_power_limit", "payload": { "gpuId": "GPU-456", "limitWatts": 400 } }),
    );
    assert_eq!(other["payload"]["success"], true);
    daemon.clock.set(29_000);
    daemon.gpus.arrive(startup_gpu());
    assert_eq!(
        manual.response(),
        crate::protocol::DaemonResponse::Command(crate::protocol::DaemonCommandResponse {
            success: true,
            error: None
        })
    );
    let outcome = completed_startup(&daemon);
    assert_eq!(outcome["outcomes"][0]["status"], "applied");
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(400)
    );
}

#[test]
fn a_gpu_arriving_after_the_deadline_is_reported_absent_and_never_applied_later_that_boot() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![startup_gpu()]));
    let alice = daemon.user(1000, "alice");
    seed(&alice, "quiet", "Quiet", 300);
    designate(&alice, Some("quiet"));
    enable(&alice);
    drop(alice);
    daemon.gpus.depart("GPU-123");
    daemon.reboot();
    let reading = daemon.gpus.block_performance();
    reading.wait_until_entered();
    daemon.clock.set(31_000);
    // Arrival before the next poll must not bypass the already expired deadline.
    daemon.gpus.arrive(startup_gpu());
    reading.release();
    let state = completed_startup(&daemon);
    assert_eq!(state["outcomes"][0]["status"], "notAttempted");
    assert!(state["outcomes"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("absent at startup"));
    daemon.restart();
    assert_eq!(completed_startup(&daemon), state);
    assert!(daemon.gpus.writes().is_empty());
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(475)
    );
}

#[test]
fn readiness_waits_for_gpu_arrival_and_the_last_startup_write_while_reads_are_served() {
    use super::harness::Setter;
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![startup_gpu()]));
    let alice = daemon.user(1000, "alice");
    seed(&alice, "quiet", "Quiet", 300);
    designate(&alice, Some("quiet"));
    enable(&alice);
    drop(alice);
    daemon.gpus.depart("GPU-123");
    daemon.reboot();
    let core = daemon.core();
    let (ready, notification) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        core.wait_for_startup();
        ready.send(()).unwrap();
    });
    assert!(notification
        .recv_timeout(std::time::Duration::from_millis(20))
        .is_err());
    assert!(daemon.performance().gpus.is_empty());
    let write = daemon.gpus.block("GPU-123", Setter::PowerLimit);
    daemon.gpus.arrive(startup_gpu());
    write.wait_until_entered();
    assert!(notification.try_recv().is_err());
    assert_eq!(daemon.performance().gpus.len(), 1);
    write.release();
    notification
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert_eq!(
        completed_startup(&daemon)["outcomes"][0]["status"],
        "applied"
    );
}

#[test]
fn an_interrupted_startup_reports_the_interruption_and_never_retries_this_boot() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![startup_gpu()]));
    let alice = daemon.user(1000, "alice");
    seed(&alice, "quiet", "Quiet", 300);
    designate(&alice, Some("quiet"));
    enable(&alice);
    drop(alice);
    daemon.gpus.depart("GPU-123");
    daemon.reboot();
    let alice = daemon.user(1000, "alice");
    assert_eq!(
        startup(&alice)["outcomes"][0]["reason"],
        "Startup is in progress."
    );
    drop(alice);
    daemon.restart();
    let state = completed_startup(&daemon);
    assert_eq!(state["outcomes"][0]["status"], "notAttempted");
    assert!(state["outcomes"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("interrupted"));
    daemon.gpus.arrive(startup_gpu());
    daemon.restart();
    assert_eq!(completed_startup(&daemon), state);
    assert!(daemon.gpus.writes().is_empty());
}

#[test]
fn saving_renaming_and_deleting_follow_the_startup_link_across_boots() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![startup_gpu()]));
    let alice = daemon.user(1000, "alice");
    seed(&alice, "quiet", "Quiet", 300);
    designate(&alice, Some("quiet"));
    enable(&alice);
    drop(alice);
    daemon.reboot();
    completed_startup(&daemon);
    let alice = daemon.user(1000, "alice");
    alice.send_json(
        json!({ "type": "set_power_limit", "payload": { "gpuId": "GPU-123", "limitWatts": 350 } }),
    );
    let revision =
        alice.send_json(json!({ "type": "get_tuning_states" }))["payload"][0]["revision"].clone();
    let saved = alice.send_json(json!({ "type": "save_to_current_profile", "payload": { "gpuId": "GPU-123", "revision": revision } }));
    assert_eq!(saved["type"], "profile");
    alice.send_json(
        json!({ "type": "rename_profile", "payload": { "profileId": "quiet", "name": "Silent" } }),
    );
    assert_eq!(startup(&alice)["profiles"][0]["profile"]["name"], "Silent");
    drop(alice);
    daemon
        .gpus
        .update("GPU-123", |gpu| gpu.power.current_limit_watts = Some(475));
    daemon.reboot();
    let state = completed_startup(&daemon);
    assert_eq!(state["outcomes"][0]["profile"]["name"], "Silent");
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(350)
    );
    let alice = daemon.user(1000, "alice");
    let writes = daemon.gpus.writes();
    alice.send_json(json!({ "type": "delete_profile", "payload": { "profileId": "quiet" } }));
    assert_eq!(startup(&alice)["profiles"], json!([]));
    assert_eq!(daemon.gpus.writes(), writes);
    drop(alice);
    daemon.reboot();
    assert_eq!(
        completed_startup(&daemon)["outcomes"][0]["reason"],
        "No startup profile."
    );
    assert_eq!(daemon.gpus.writes(), writes);
}

#[test]
fn an_unhealthy_store_suspends_startup() {
    use super::tuning_store::TUNING_STORE_FILE_NAME;
    let corrupt = b"{\"schemaVersion\":4,\"startupBehavior\":\"startupProfiles\",\"users\":[";
    let mut daemon = Harness::start_with_persistent_files(
        FakeGpuBackend::with_gpus(vec![startup_gpu()]),
        &[(TUNING_STORE_FILE_NAME, corrupt)],
    );
    assert!(daemon.gpus.writes().is_empty());
    assert_eq!(
        daemon.persistent_file(TUNING_STORE_FILE_NAME).as_deref(),
        Some(corrupt.as_slice())
    );
    daemon.restart();
    assert!(daemon.gpus.writes().is_empty());
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(475)
    );
}

#[test]
fn startup_outcome_dismissals_belong_to_the_user_survive_restart_and_reset_at_reboot() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![startup_gpu()]));
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");
    assert_eq!(
        startup(&alice)["outcomes"][0]["reason"],
        "Startup behavior is Off."
    );
    assert_eq!(
        alice.send_json(
            json!({ "type": "dismiss_startup_outcome", "payload": { "gpuId": "GPU-123" } })
        )["payload"]["success"],
        true
    );
    assert_eq!(startup(&alice)["outcomes"], json!([]));
    assert_eq!(startup(&bob)["outcomes"].as_array().unwrap().len(), 1);
    assert_eq!(
        startup(&daemon.user(1000, "replacement"))["outcomes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    drop(alice);
    drop(bob);
    daemon.restart();
    let alice = daemon.user(1000, "alice");
    assert_eq!(startup(&alice)["outcomes"], json!([]));
    drop(alice);
    daemon.reboot();
    assert_eq!(
        startup(&daemon.user(1000, "alice"))["outcomes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn unreadable_boot_state_is_preserved_and_prevents_startup_writes_while_manual_tuning_works() {
    for bytes in [
        b"{incomplete".as_slice(),
        b"{\"schemaVersion\":99}".as_slice(),
    ] {
        let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![startup_gpu()]));
        let alice = daemon.user(1000, "alice");
        seed(&alice, "quiet", "Quiet", 300);
        designate(&alice, Some("quiet"));
        enable(&alice);
        drop(alice);
        std::fs::write(daemon.boot_path("startup-state.json"), bytes).unwrap();
        daemon.restart();
        assert!(daemon.gpus.writes().is_empty());
        let alice = daemon.user(1000, "alice");
        let state = alice.send_json(json!({ "type": "get_startup_state" }));
        assert_eq!(state["type"], "error");
        assert!(state["payload"]["message"]
            .as_str()
            .unwrap()
            .contains("boot state"));
        assert_eq!(
            std::fs::read(daemon.boot_path("startup-state.json")).unwrap(),
            bytes
        );
        assert_eq!(alice.send_json(json!({ "type": "set_power_limit", "payload": { "gpuId": "GPU-123", "limitWatts": 350 } }))["payload"]["success"], true);
        assert_eq!(
            daemon.gpus.gpu("GPU-123").power.current_limit_watts,
            Some(350)
        );
    }
}
