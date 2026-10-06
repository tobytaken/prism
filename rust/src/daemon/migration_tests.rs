//! Seam 1: legacy migration through daemon sessions and host lifecycle.
use super::harness::{tunable_gpu, FakeGpuBackend, Harness};
use serde_json::json;

#[test]
fn legacy_uuid_settings_become_an_ownerless_startup_profile_once() {
    let legacy = br#"{"gpus":{"GPU-123":{"desired":{"powerLimitWatts":400,"graphicsOffsetMHz":50,"gpuClockLock":{"minMhz":1500,"maxMhz":1500}}}}}"#;
    let mut daemon = Harness::start_with_persistent_files(
        FakeGpuBackend::with_gpus(vec![tunable_gpu()]),
        &[("state.json", legacy)],
    );
    daemon.core().wait_for_startup();
    let alice = daemon.user(1000, "alice");
    let migration = alice.send_json(json!({"type":"get_migration_state"}));
    assert_eq!(migration["type"], "migration_state");
    let profile = &migration["payload"]["profiles"][0];
    assert_eq!(profile["name"], "Previous startup settings");
    assert_eq!(profile["gpuId"], "GPU-123");
    assert_eq!(
        profile["settings"],
        json!({"powerLimitWatts":{"kind":"value","value":400},"graphicsOffsetMHz":{"kind":"value","value":50}})
    );
    assert_eq!(migration["payload"]["canAdopt"], true);
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(400)
    );
    assert_eq!(daemon.persistent_file("state.json"), None);
    assert_eq!(
        daemon.persistent_file("state.json.migrated").unwrap(),
        legacy
    );
    drop(alice);
    daemon.restart();
    daemon.core().wait_for_startup();
    assert_eq!(
        daemon
            .user(1000, "alice")
            .send_json(json!({"type":"get_migration_state"})),
        migration
    );
    daemon
        .gpus
        .update("GPU-123", |gpu| gpu.power.current_limit_watts = Some(475));
    daemon.reboot();
    daemon.core().wait_for_startup();
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(400)
    );
}

#[test]
fn only_the_first_user_can_adopt_and_notices_are_acknowledged_per_account() {
    let legacy = br#"{"gpus":{"GPU-123":{"desired":{"powerLimitWatts":400}}}}"#;
    let mut daemon = Harness::start_with_persistent_files(
        FakeGpuBackend::with_gpus(vec![tunable_gpu()]),
        &[("state.json", legacy)],
    );
    daemon.core().wait_for_startup();
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");
    let state = bob.send_json(json!({"type":"get_migration_state"}))["payload"].clone();
    assert_eq!(state["canAdopt"], false);
    assert!(state["notice"].is_object());
    let id = state["profiles"][0]["id"].as_str().unwrap();
    for session in [&alice, &bob] {
        assert_eq!(
            session.send_json(
                json!({"type":"rename_profile","payload":{"profileId":id,"name":"Mine"}})
            )["type"],
            "error"
        );
        assert_eq!(
            session.send_json(json!({"type":"delete_profile","payload":{"profileId":id}}))["type"],
            "error"
        );
    }
    assert_eq!(
        bob.send_json(json!({"type":"adopt_migrated_profile","payload":{"profileId":id}}))["type"],
        "error"
    );
    let adopted =
        alice.send_json(json!({"type":"adopt_migrated_profile","payload":{"profileId":id}}));
    assert_eq!(adopted["type"], "profile", "{adopted}");
    assert_eq!(
        alice.send_json(json!({"type":"rename_profile","payload":{"profileId":id,"name":"Mine"}}))
            ["type"],
        "profile"
    );
    assert_eq!(
        bob.send_json(json!({"type":"rename_profile","payload":{"profileId":id,"name":"Mine"}}))
            ["type"],
        "error"
    );
    assert_eq!(
        alice.send_json(json!({"type":"dismiss_migration_notice"}))["payload"]["success"],
        true
    );
    assert!(alice.send_json(json!({"type":"get_migration_state"}))["payload"]["notice"].is_null());
    assert!(bob.send_json(json!({"type":"get_migration_state"}))["payload"]["notice"].is_object());
    drop(alice);
    drop(bob);
    daemon.reboot();
    daemon.core().wait_for_startup();
    assert!(daemon
        .user(1000, "alice")
        .send_json(json!({"type":"get_migration_state"}))["payload"]["notice"]
        .is_null());
    assert!(daemon
        .user(1000, "new-account")
        .send_json(json!({"type":"get_migration_state"}))["payload"]["notice"]
        .is_object());
}

#[test]
fn migration_keeps_absent_uuids_resolves_unique_bus_ids_and_lists_ambiguous_values() {
    use super::harness::tunable_gpu_with_id;
    let mut unique = tunable_gpu_with_id("GPU-unique");
    unique.bus_id = Some("0000:01:00.0".into());
    let mut first = tunable_gpu_with_id("GPU-first");
    first.bus_id = Some("0000:02:00.0".into());
    let mut second = tunable_gpu_with_id("GPU-second");
    second.bus_id = first.bus_id.clone();
    let legacy = br#"{"gpus":{
        "GPU-absent":{"desired":{"powerLimitWatts":425}},
        "0000:01:00.0":{"desired":{"graphicsOffsetMHz":50}},
        "0000:02:00.0":{"desired":{"powerLimitWatts":450,"graphicsOffsetMHz":100,"memoryClockLock":{"minMhz":6000,"maxMhz":7000}}},
        "nvidia-9":{"desired":{"memoryOffsetMHz":75}}
    }}"#;
    let mut daemon = Harness::start_with_persistent_files(
        FakeGpuBackend::with_gpus(vec![unique, first, second]),
        &[("state.json", legacy)],
    );
    daemon.clock.set(30_001);
    daemon.core().wait_for_startup();
    let alice = daemon.user(1000, "alice");
    let state = alice.send_json(json!({"type":"get_migration_state"}))["payload"].clone();
    let profiles = state["profiles"].as_array().unwrap();
    assert_eq!(profiles.len(), 2);
    assert!(profiles
        .iter()
        .any(|profile| profile["gpuId"] == "GPU-absent"));
    assert!(profiles
        .iter()
        .any(|profile| profile["gpuId"] == "GPU-unique"));
    let dropped = state["notice"]["dropped"].as_array().unwrap();
    assert_eq!(dropped.len(), 2);
    assert_eq!(dropped[0]["key"], "0000:02:00.0");
    assert_eq!(dropped[0]["values"]["powerLimitWatts"], 450);
    assert_eq!(
        dropped[0]["values"]["memoryClockLock"],
        json!({"minMhz":6000,"maxMhz":7000})
    );
    drop(alice);
    daemon.gpus.depart("GPU-second");
    daemon.restart();
    daemon.core().wait_for_startup();
    assert_eq!(
        daemon
            .user(1000, "alice")
            .send_json(json!({"type":"get_migration_state"}))["payload"],
        state
    );
}

#[test]
fn a_missing_inventory_slot_is_not_reassigned_to_a_different_gpu() {
    let mut gpu = tunable_gpu();
    gpu.nvidia_index = Some(7);
    let legacy = br#"{"gpus":{"nvidia-0":{"desired":{"powerLimitWatts":400}},"nvidia-7":{"desired":{"graphicsOffsetMHz":50}}}}"#;
    let daemon = Harness::start_with_persistent_files(
        FakeGpuBackend::with_gpus(vec![gpu]),
        &[("state.json", legacy)],
    );
    daemon.core().wait_for_startup();
    let state = daemon
        .user(1000, "alice")
        .send_json(json!({"type":"get_migration_state"}))["payload"]
        .clone();
    assert_eq!(state["profiles"].as_array().unwrap().len(), 1);
    assert_eq!(
        state["profiles"][0]["settings"],
        json!({"graphicsOffsetMHz":{"kind":"value","value":50}})
    );
    assert_eq!(state["notice"]["dropped"][0]["key"], "nvidia-0");
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(475)
    );
}

#[test]
fn an_interrupted_archive_retries_without_migrating_or_overwriting_profiles_again() {
    let legacy = br#"{"gpus":{"GPU-123":{"desired":{"powerLimitWatts":400}}}}"#;
    let mut daemon = Harness::start_with_persistent_files(
        FakeGpuBackend::with_gpus(vec![tunable_gpu()]),
        &[("state.json", legacy)],
    );
    daemon.core().wait_for_startup();
    let before = daemon
        .user(1000, "alice")
        .send_json(json!({"type":"get_migration_state"}));
    let saved = daemon.persistent_file("tuning-state.json").unwrap();
    // Simulate committing schema 6 but crashing before the legacy rename.
    std::fs::rename(
        daemon.persistent_path("state.json.migrated"),
        daemon.persistent_path("state.json"),
    )
    .unwrap();
    daemon.restart();
    daemon.core().wait_for_startup();
    assert_eq!(
        daemon
            .user(1000, "alice")
            .send_json(json!({"type":"get_migration_state"})),
        before
    );
    assert_eq!(daemon.persistent_file("tuning-state.json").unwrap(), saved);
    assert_eq!(
        daemon.persistent_file("state.json.migrated").unwrap(),
        legacy
    );
    assert!(daemon.persistent_file("state.json").is_none());
}

#[test]
fn unreadable_legacy_settings_are_preserved_and_block_startup_but_not_live_tuning() {
    let legacy = b"{broken";
    let daemon = Harness::start_with_persistent_files(
        FakeGpuBackend::with_gpus(vec![tunable_gpu()]),
        &[("state.json", legacy)],
    );
    assert!(daemon.gpus.writes().is_empty());
    assert_eq!(daemon.persistent_file("state.json").unwrap(), legacy);
    assert!(daemon.persistent_file("tuning-state.json").is_none());
    assert!(matches!(
        daemon.status().tuning_store,
        Some(crate::gpu::TuningStoreHealth::Unhealthy { .. })
    ));
    assert_eq!(
        daemon.send_json(
            json!({"type":"set_power_limit","payload":{"gpuId":"GPU-123","limitWatts":410}})
        )["payload"]["success"],
        true
    );
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(410)
    );
    assert_eq!(daemon.persistent_file("state.json").unwrap(), legacy);
}

#[test]
fn reopening_schema_six_preserves_the_original_archive_if_legacy_prism_recreated_state() {
    let original = br#"{"gpus":{"GPU-123":{"desired":{"powerLimitWatts":400,"gpuClockLock":{"minMhz":1500,"maxMhz":1800}}}}}"#;
    let mut daemon = Harness::start_with_persistent_files(
        FakeGpuBackend::with_gpus(vec![tunable_gpu()]),
        &[("state.json", original)],
    );
    daemon.core().wait_for_startup();
    let before = daemon
        .user(1000, "alice")
        .send_json(json!({"type":"get_migration_state"}));
    let saved = daemon.persistent_file("tuning-state.json").unwrap();
    let recreated = br#"{"gpus":{}}"#;
    std::fs::write(daemon.persistent_path("state.json"), recreated).unwrap();
    daemon.restart();
    daemon.core().wait_for_startup();
    assert_eq!(
        daemon.persistent_file("state.json.migrated").unwrap(),
        original
    );
    assert_eq!(daemon.persistent_file("state.json").unwrap(), recreated);
    assert_eq!(daemon.persistent_file("tuning-state.json").unwrap(), saved);
    assert_eq!(
        daemon
            .user(1000, "alice")
            .send_json(json!({"type":"get_migration_state"})),
        before
    );
}

#[test]
fn schema_five_restores_are_kept_when_legacy_settings_migrate() {
    let store = br#"{"schemaVersion":5,"startupBehavior":"restoreLastApplied","lastApplied":{"GPU-123":{"powerLimitWatts":{"kind":"value","value":420}}}}"#;
    let legacy = br#"{"gpus":{"GPU-123":{"desired":{"powerLimitWatts":400}}}}"#;
    let mut daemon = Harness::start_with_persistent_files(
        FakeGpuBackend::with_gpus(vec![tunable_gpu()]),
        &[("tuning-state.json", store), ("state.json", legacy)],
    );
    daemon.core().wait_for_startup();
    let alice = daemon.user(1000, "alice");
    let migration = alice.send_json(json!({"type":"get_migration_state"}));
    assert_eq!(
        migration["payload"]["profiles"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(400)
    );
    assert_eq!(
        alice.send_json(json!({"type":"get_startup_state"}))["payload"]["behavior"],
        "startupProfiles"
    );
    assert_eq!(
        alice.send_json(
            json!({"type":"set_startup_behavior","payload":{"behavior":"restoreLastApplied"}})
        )["payload"]["success"],
        true
    );
    drop(alice);
    daemon.reboot();
    daemon.core().wait_for_startup();
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(420)
    );
    assert_eq!(
        daemon.persistent_file("state.json.migrated").unwrap(),
        legacy
    );
}
