//! Seam 1: restoring last applied tuning values at boot, its unclean-shutdown guard, and
//! manual writes whose last applied values could not be recorded.

use serde_json::{Value, json};

use super::harness::{FakeGpuBackend, FakeSession, Harness, Setter, tunable_gpu};
use super::possibly_stale::POSSIBLY_STALE_FILE_NAME;
use super::tuning_store::TUNING_STORE_FILE_NAME;

const NOT_RECORDED: &str = "This change was not recorded for startup.";

fn restore_gpu() -> prism_gpu::GpuPerformanceDevice {
    let mut gpu = tunable_gpu();
    gpu.power.min_limit_watts = Some(100);
    gpu
}

fn send(session: &FakeSession, request: Value) -> Value {
    let response = session.send_json(request);
    assert_ne!(response["type"], "error", "{response}");
    response["payload"].clone()
}

fn set_power(session: &FakeSession, watts: u32) -> Value {
    send(
        session,
        json!({ "type": "set_power_limit", "payload": { "gpuId": "GPU-123", "limitWatts": watts } }),
    )
}

fn startup(session: &FakeSession) -> Value {
    send(session, json!({ "type": "get_startup_state" }))
}

fn restore_on_boot(session: &FakeSession) {
    let response = send(
        session,
        json!({ "type": "set_startup_behavior", "payload": { "behavior": "restoreLastApplied" } }),
    );
    assert_eq!(response["success"], true, "{response}");
}

/// The startup outcome for GPU-123 once startup has finished with it.
fn completed_outcome(daemon: &Harness) -> Value {
    let session = daemon.user(1000, "alice");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let state = startup(&session);
        if let Some(outcome) = state["outcomes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|outcome| outcome["gpuId"] == "GPU-123")
            .filter(|outcome| outcome["reason"] != "Startup is in progress.")
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

/// What the GPU reports after the host restarted its controls.
fn power_cycled(daemon: &Harness) {
    daemon.gpus.update("GPU-123", |gpu| {
        gpu.power.current_limit_watts = Some(575);
        gpu.clocks.graphics.current_offset_mhz = Some(0);
        gpu.clocks.memory.current_offset_mhz = Some(0);
    });
}

fn values(daemon: &Harness) -> (Option<u32>, Option<i32>, Option<i32>) {
    let gpu = daemon.gpus.gpu("GPU-123");
    (
        gpu.power.current_limit_watts,
        gpu.clocks.graphics.current_offset_mhz,
        gpu.clocks.memory.current_offset_mhz,
    )
}

#[test]
fn restore_puts_each_settings_latest_manual_value_into_effect_and_keeps_the_current_profile() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![restore_gpu()]));
    let alice = daemon.user(1000, "alice");
    set_power(&alice, 280);
    send(
        &alice,
        json!({ "type": "set_clock_offsets", "payload": {
            "gpuId": "GPU-123", "graphicsOffsetMHz": 30, "memoryOffsetMHz": 40
        } }),
    );
    alice.seed_profiles(json!([{
        "id": "quiet", "name": "Quiet", "gpuId": "GPU-123", "settings": {
            "powerLimitWatts": { "kind": "value", "value": 300 },
            "memoryOffsetMHz": { "kind": "default" }
        }
    }]));
    let application = send(
        &alice,
        json!({ "type": "apply_profile_by_id", "payload": { "profileId": "quiet" } }),
    );
    assert_eq!(application["power"]["status"], "succeeded", "{application}");
    send(
        &alice,
        json!({ "type": "set_clock_offsets", "payload": {
            "gpuId": "GPU-123", "graphicsOffsetMHz": "default", "memoryOffsetMHz": null
        } }),
    );
    // A setting whose latest manual value is a default must be restored as the default.
    send(
        &alice,
        json!({ "type": "set_clock_offsets", "payload": {
            "gpuId": "GPU-123", "graphicsOffsetMHz": null, "memoryOffsetMHz": null
        } }),
    );
    restore_on_boot(&alice);
    assert_eq!(startup(&alice)["behavior"], "restoreLastApplied");
    drop(alice);

    daemon.gpus.update("GPU-123", |gpu| {
        gpu.power.current_limit_watts = Some(575);
        gpu.clocks.graphics.current_offset_mhz = Some(77);
        gpu.clocks.memory.current_offset_mhz = Some(55);
    });
    daemon.reboot();

    let outcome = completed_outcome(&daemon);
    assert_eq!(outcome["status"], "applied", "{outcome}");
    assert_eq!(outcome["behavior"], "restoreLastApplied", "{outcome}");
    assert_eq!(outcome["profile"], Value::Null);
    assert_eq!(values(&daemon), (Some(300), Some(0), Some(0)));
    let alice = daemon.user(1000, "alice");
    let states = send(&alice, json!({ "type": "get_tuning_states" }));
    assert_eq!(
        states[0]["currentProfile"],
        json!({ "id": "quiet", "name": "Quiet" })
    );
}

#[test]
fn a_partial_or_blocked_restore_clears_the_current_profile_and_values_are_never_clamped() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![restore_gpu()]));
    let alice = daemon.user(1000, "alice");
    alice.seed_profiles(json!([{
        "id": "quiet", "name": "Quiet", "gpuId": "GPU-123", "settings": {
            "powerLimitWatts": { "kind": "value", "value": 300 },
            "graphicsOffsetMHz": { "kind": "value", "value": 150 }
        }
    }]));
    send(
        &alice,
        json!({ "type": "apply_profile_by_id", "payload": { "profileId": "quiet" } }),
    );
    restore_on_boot(&alice);
    drop(alice);

    power_cycled(&daemon);
    daemon.gpus.update("GPU-123", |gpu| {
        gpu.clocks.graphics.max_offset_mhz = Some(100)
    });
    let writes = daemon.gpus.writes().len();
    daemon.reboot();
    let outcome = completed_outcome(&daemon);
    assert_eq!(outcome["status"], "notAttempted", "{outcome}");
    assert_eq!(daemon.gpus.writes().len(), writes);
    assert_eq!(values(&daemon), (Some(575), Some(0), Some(0)));
    let alice = daemon.user(1000, "alice");
    assert_eq!(
        send(&alice, json!({ "type": "get_tuning_states" }))[0]["currentProfile"],
        Value::Null
    );
}

#[test]
fn a_crash_after_a_restore_skips_the_next_restore_once_but_a_daemon_restart_does_not() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![restore_gpu()]));
    let alice = daemon.user(1000, "alice");
    set_power(&alice, 300);
    restore_on_boot(&alice);
    drop(alice);

    power_cycled(&daemon);
    daemon.reboot();
    assert_eq!(completed_outcome(&daemon)["status"], "applied");
    // systemd restarts the daemon mid-boot; that stop is not a clean host shutdown.
    daemon.restart();
    power_cycled(&daemon);
    daemon.reboot();
    assert_eq!(completed_outcome(&daemon)["status"], "applied");
    assert_eq!(values(&daemon).0, Some(300));

    // The host crashes after a restore, even though the daemon restarted during that boot.
    daemon.restart();
    power_cycled(&daemon);
    let writes = daemon.gpus.writes().len();
    daemon.crash_and_reboot();
    let outcome = completed_outcome(&daemon);
    assert_eq!(outcome["status"], "notAttempted", "{outcome}");
    assert!(
        outcome["reason"]
            .as_str()
            .unwrap()
            .contains("didn't shut down cleanly"),
        "{outcome}"
    );
    assert_eq!(values(&daemon).0, Some(575));
    assert_eq!(daemon.gpus.writes().len(), writes);

    // The skip happens once: this boot attempted nothing, so its crash doesn't skip again.
    daemon.crash_and_reboot();
    assert_eq!(completed_outcome(&daemon)["status"], "applied");
    assert_eq!(values(&daemon).0, Some(300));
}

#[test]
fn startup_profiles_skip_the_unclean_shutdown_guard() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![restore_gpu()]));
    let alice = daemon.user(1000, "alice");
    set_power(&alice, 300);
    alice.seed_profiles(json!([{
        "id": "quiet", "name": "Quiet", "gpuId": "GPU-123",
        "settings": { "powerLimitWatts": { "kind": "value", "value": 250 } }
    }]));
    send(
        &alice,
        json!({ "type": "set_startup_profile", "payload": { "gpuId": "GPU-123", "profileId": "quiet" } }),
    );
    restore_on_boot(&alice);
    drop(alice);
    power_cycled(&daemon);
    daemon.reboot();
    assert_eq!(completed_outcome(&daemon)["status"], "applied");
    assert_eq!(values(&daemon).0, Some(300));

    // The host crashes after that restore, but startup profiles apply regardless.
    let alice = daemon.user(1000, "alice");
    send(
        &alice,
        json!({ "type": "set_startup_behavior", "payload": { "behavior": "startupProfiles" } }),
    );
    drop(alice);
    for _ in 0..2 {
        power_cycled(&daemon);
        daemon.crash_and_reboot();
        assert_eq!(completed_outcome(&daemon)["status"], "applied");
        assert_eq!(values(&daemon).0, Some(250));
    }
}

#[test]
fn a_write_that_could_not_be_recorded_succeeds_and_marks_last_applied_possibly_stale() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![restore_gpu()]));
    let alice = daemon.user(1000, "alice");
    set_power(&alice, 300);
    restore_on_boot(&alice);
    assert_eq!(startup(&alice)["possiblyStale"], json!([]));

    // Recording fails while the store's staged file can't be created.
    let blocker = daemon.persistent_path(&format!("{TUNING_STORE_FILE_NAME}.tmp"));
    std::fs::create_dir(&blocker).unwrap();
    assert_eq!(
        set_power(&alice, 320),
        json!({ "success": true, "error": NOT_RECORDED })
    );
    let offsets = send(
        &alice,
        json!({ "type": "set_clock_offsets", "payload": {
            "gpuId": "GPU-123", "graphicsOffsetMHz": 50, "memoryOffsetMHz": null
        } }),
    );
    assert_eq!(
        offsets["graphics"],
        json!({ "status": "succeeded", "error": NOT_RECORDED })
    );
    // The GPU write is never undone.
    assert_eq!(values(&daemon).0, Some(320));
    assert_eq!(values(&daemon).1, Some(50));
    drop(alice);

    std::fs::remove_dir(&blocker).unwrap();
    daemon.restart();
    let alice = daemon.user(1000, "alice");
    assert_eq!(startup(&alice)["possiblyStale"], json!(["GPU-123"]));
    assert_eq!(
        set_power(&alice, 330),
        json!({ "success": true, "error": null })
    );
    // The graphics offset still hasn't been recorded since its write.
    assert_eq!(startup(&alice)["possiblyStale"], json!(["GPU-123"]));
    let offsets = send(
        &alice,
        json!({ "type": "set_clock_offsets", "payload": {
            "gpuId": "GPU-123", "graphicsOffsetMHz": 60, "memoryOffsetMHz": null
        } }),
    );
    assert_eq!(offsets["graphics"]["error"], Value::Null);
    assert_eq!(startup(&alice)["possiblyStale"], json!([]));
    drop(alice);

    // A profile application reports each setting that reached the GPU as succeeded, and the
    // marker survives a reboot until the next successful record.
    let alice = daemon.user(1000, "alice");
    alice.seed_profiles(json!([{
        "id": "loud", "name": "Loud", "gpuId": "GPU-123", "settings": {
            "graphicsOffsetMHz": { "kind": "value", "value": 70 },
            "memoryOffsetMHz": { "kind": "value", "value": 10 }
        }
    }]));
    std::fs::create_dir(&blocker).unwrap();
    daemon
        .gpus
        .fail("GPU-123", Setter::MemoryOffset, "rejected");
    let application = send(
        &alice,
        json!({ "type": "apply_profile_by_id", "payload": { "profileId": "loud" } }),
    );
    assert_eq!(
        application["graphics"],
        json!({ "status": "succeeded", "error": NOT_RECORDED })
    );
    assert_eq!(application["memory"]["status"], "failed");
    assert_eq!(values(&daemon).1, Some(70));
    drop(alice);
    std::fs::remove_dir(&blocker).unwrap();
    daemon.reboot();
    let alice = daemon.user(1000, "alice");
    assert_eq!(startup(&alice)["possiblyStale"], json!(["GPU-123"]));
}

#[test]
fn a_skipped_restore_is_reported_for_each_gpu_with_last_applied_values_even_when_absent() {
    let mut second = restore_gpu();
    second.id = "GPU-456".into();
    second.stable_id = Some("GPU-456".into());
    let mut third = second.clone();
    third.id = "GPU-789".into();
    third.stable_id = Some("GPU-789".into());
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![
        restore_gpu(),
        second,
        third,
    ]));
    let alice = daemon.user(1000, "alice");
    set_power(&alice, 300);
    send(
        &alice,
        json!({ "type": "set_power_limit", "payload": { "gpuId": "GPU-456", "limitWatts": 300 } }),
    );
    restore_on_boot(&alice);
    drop(alice);
    daemon.reboot();
    assert_eq!(completed_outcome(&daemon)["status"], "applied");

    daemon.gpus.depart("GPU-456");
    daemon.crash_and_reboot();
    let alice = daemon.user(1000, "alice");
    let outcomes = startup(&alice)["outcomes"].clone();
    let reason = |gpu_id: &str| {
        outcomes
            .as_array()
            .unwrap()
            .iter()
            .find(|outcome| outcome["gpuId"] == gpu_id)
            .unwrap_or_else(|| panic!("no outcome for {gpu_id}: {outcomes}"))["reason"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert!(reason("GPU-123").contains("didn't shut down cleanly"));
    assert!(reason("GPU-456").contains("didn't shut down cleanly"));
    assert_eq!(reason("GPU-789"), "No last applied tuning values.");
}

#[test]
fn an_unreadable_possibly_stale_marker_is_preserved_and_marks_every_recorded_gpu() {
    let daemon = Harness::start_with_persistent_files(
        FakeGpuBackend::with_gpus(vec![restore_gpu()]),
        &[(POSSIBLY_STALE_FILE_NAME, b"{\"GPU-123\": [")],
    );
    let alice = daemon.user(1000, "alice");
    assert_eq!(startup(&alice)["possiblyStale"], json!([]));
    assert_eq!(
        set_power(&alice, 300),
        json!({ "success": true, "error": null })
    );
    assert_eq!(startup(&alice)["possiblyStale"], json!(["GPU-123"]));
    assert_eq!(
        daemon.persistent_file(POSSIBLY_STALE_FILE_NAME).unwrap(),
        b"{\"GPU-123\": ["
    );
}
