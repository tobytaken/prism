//! Seam 1: each GPU's current profile and the per-GPU state revision that guards saves.

use serde_json::{json, Value};

use super::harness::{tunable_gpu, FakeGpuBackend, FakeSession, Harness, Setter};

const STALE: &str = "The GPU's values changed since you last looked. Review them and save again.";

fn import(session: &FakeSession, profiles: Value) {
    let response = session
        .send_json(json!({ "type": "import_profiles", "payload": { "profiles": profiles } }));
    assert_eq!(response["type"], "profile_import", "{response}");
}

fn quiet_and_loud() -> Value {
    json!([
        { "id": "quiet", "name": "Quiet", "gpuId": "GPU-123",
          "settings": { "powerLimitWatts": { "kind": "value", "value": 300 } } },
        { "id": "loud", "name": "Loud", "gpuId": "GPU-123",
          "settings": {
              "powerLimitWatts": { "kind": "value", "value": 500 },
              "memoryOffsetMHz": { "kind": "value", "value": 100 }
          } }
    ])
}

fn apply_by_id(session: &FakeSession, profile_id: &str) -> Value {
    let response = session.send_json(
        json!({ "type": "apply_profile_by_id", "payload": { "profileId": profile_id } }),
    );
    assert_eq!(response["type"], "profile_application", "{response}");
    response["payload"].clone()
}

/// The GPU's tuning state as this session sees it.
fn state(session: &FakeSession) -> Value {
    let response = session.send_json(json!({ "type": "get_tuning_states" }));
    assert_eq!(response["type"], "tuning_states", "{response}");
    response["payload"]
        .as_array()
        .unwrap()
        .iter()
        .find(|state| state["gpuId"] == "GPU-123")
        .cloned()
        .expect("GPU-123 has a tuning state")
}

fn current(session: &FakeSession) -> Value {
    state(session)["currentProfile"].clone()
}

fn revision(session: &FakeSession) -> Value {
    state(session)["revision"].clone()
}

fn save_as_new(session: &FakeSession, name: &str, revision: Value) -> Value {
    session.send_json(json!({
        "type": "save_profile_as_new",
        "payload": { "gpuId": "GPU-123", "name": name, "revision": revision }
    }))
}

fn save_to_current(session: &FakeSession, revision: Value) -> Value {
    session.send_json(json!({
        "type": "save_to_current_profile",
        "payload": { "gpuId": "GPU-123", "revision": revision }
    }))
}

fn profiles(session: &FakeSession) -> Vec<Value> {
    let response = session.send_json(json!({ "type": "list_profiles" }));
    response["payload"].as_array().unwrap().clone()
}

fn error_message(response: &Value) -> &str {
    assert_eq!(response["type"], "error", "{response}");
    response["payload"]["message"].as_str().unwrap()
}

#[test]
fn a_gpu_starts_with_no_current_profile_and_an_applied_profile_becomes_current() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    import(&alice, quiet_and_loud());
    assert_eq!(current(&alice), Value::Null);

    let applied = apply_by_id(&alice, "quiet");
    assert_eq!(applied["power"]["status"], "succeeded");
    assert_eq!(current(&alice), json!({ "id": "quiet", "name": "Quiet" }));

    apply_by_id(&alice, "loud");
    assert_eq!(current(&alice), json!({ "id": "loud", "name": "Loud" }));
    assert_eq!(
        daemon.gpus.gpu("GPU-123").clocks.memory.current_offset_mhz,
        Some(100)
    );
}

#[test]
fn a_reviewed_or_blocked_application_leaves_the_previous_current_profile_and_the_gpu_unchanged() {
    let mut gpu = tunable_gpu();
    gpu.power.max_limit_watts = Some(450);
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![gpu]));
    let alice = daemon.user(1000, "alice");
    import(&alice, quiet_and_loud());
    apply_by_id(&alice, "quiet");
    let writes = daemon.gpus.writes();

    // Loud's 500 W exceeds the 450 W maximum, so it needs a review and nothing is written.
    let review = apply_by_id(&alice, "loud");
    assert_eq!(review["corrections"][0]["proposedValue"], 450);
    assert_eq!(current(&alice), json!({ "id": "quiet", "name": "Quiet" }));
    assert_eq!(daemon.gpus.writes(), writes);
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(300)
    );

    // An application whose first write fails changes nothing on the GPU either.
    daemon
        .gpus
        .update("GPU-123", |gpu| gpu.power.max_limit_watts = Some(600));
    daemon
        .gpus
        .fail("GPU-123", Setter::PowerLimit, "Power write denied");
    let failed = apply_by_id(&alice, "loud");
    assert_eq!(failed["power"]["status"], "failed");
    assert_eq!(failed["memory"]["status"], "unattempted");
    assert_eq!(current(&alice), json!({ "id": "quiet", "name": "Quiet" }));
}

#[test]
fn a_partly_applied_profile_leaves_the_gpu_with_no_current_profile() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    import(&alice, quiet_and_loud());
    apply_by_id(&alice, "quiet");
    daemon
        .gpus
        .fail("GPU-123", Setter::MemoryOffset, "Memory write denied");

    let partial = apply_by_id(&alice, "loud");
    assert_eq!(partial["power"]["status"], "succeeded");
    assert_eq!(partial["memory"]["status"], "failed");
    assert_eq!(current(&alice), Value::Null);
}

#[test]
fn a_corrected_and_applied_profile_becomes_current() {
    let mut gpu = tunable_gpu();
    gpu.power.max_limit_watts = Some(450);
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![gpu]));
    let alice = daemon.user(1000, "alice");
    import(&alice, quiet_and_loud());
    let review = apply_by_id(&alice, "loud");

    let corrected = alice.send_json(json!({
        "type": "correct_and_apply_profile",
        "payload": { "profileId": "loud", "corrections": review["corrections"] }
    }));
    assert_eq!(
        corrected["payload"]["application"]["power"]["status"],
        "succeeded"
    );
    assert_eq!(current(&alice), json!({ "id": "loud", "name": "Loud" }));
}

#[test]
fn save_to_current_overwrites_the_current_profile_with_the_applied_values() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    import(&alice, quiet_and_loud());
    apply_by_id(&alice, "quiet");
    alice.send_json(json!({
        "type": "set_clock_offsets",
        "payload": { "gpuId": "GPU-123", "graphicsOffsetMHz": "default", "memoryOffsetMHz": 40 }
    }));

    let saved = save_to_current(&alice, revision(&alice));
    assert_eq!(saved["type"], "profile", "{saved}");
    let expected = json!({
        "id": "quiet",
        "name": "Quiet",
        "gpuId": "GPU-123",
        "settings": {
            "powerLimitWatts": { "kind": "value", "value": 300 },
            "graphicsOffsetMHz": { "kind": "default" },
            "memoryOffsetMHz": { "kind": "value", "value": 40 }
        }
    });
    assert_eq!(saved["payload"], expected);
    assert!(profiles(&alice).contains(&expected));
    assert_eq!(current(&alice), json!({ "id": "quiet", "name": "Quiet" }));
}

#[test]
fn save_as_new_creates_a_profile_that_becomes_current_and_leaves_the_previous_one_alone() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    import(&alice, quiet_and_loud());
    apply_by_id(&alice, "quiet");
    alice.send_json(json!({
        "type": "set_power_limit", "payload": { "gpuId": "GPU-123", "limitWatts": 350 }
    }));

    let saved = save_as_new(&alice, "Warmer", revision(&alice));
    assert_eq!(saved["type"], "profile", "{saved}");
    assert_eq!(
        saved["payload"]["settings"]["powerLimitWatts"],
        json!({ "kind": "value", "value": 350 })
    );
    let id = saved["payload"]["id"].clone();
    assert_eq!(current(&alice), json!({ "id": id, "name": "Warmer" }));
    let quiet = profiles(&alice)
        .into_iter()
        .find(|profile| profile["id"] == "quiet")
        .unwrap();
    assert_eq!(
        quiet["settings"]["powerLimitWatts"],
        json!({ "kind": "value", "value": 300 })
    );
}

#[test]
fn save_to_current_needs_a_current_profile() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    let response = save_to_current(&alice, revision(&alice));
    assert_eq!(
        error_message(&response),
        "This GPU has no current profile. Save as a new profile instead."
    );
}

#[test]
fn a_save_with_a_stale_revision_is_rejected_and_saves_nothing() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    import(&alice, quiet_and_loud());
    apply_by_id(&alice, "quiet");
    let before = profiles(&alice);

    // Reported values changed outside Prism.
    let seen = revision(&alice);
    daemon
        .gpus
        .update("GPU-123", |gpu| gpu.power.current_limit_watts = Some(320));
    assert_ne!(revision(&alice), seen);
    assert_eq!(error_message(&save_to_current(&alice, seen.clone())), STALE);
    assert_eq!(error_message(&save_as_new(&alice, "New", seen)), STALE);

    // Recorded state changed through another session, with the same reported values.
    let bob = daemon.user(1001, "bob");
    let seen = revision(&alice);
    bob.send_json(json!({
        "type": "set_power_limit", "payload": { "gpuId": "GPU-123", "limitWatts": 320 }
    }));
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(320)
    );
    assert_ne!(revision(&alice), seen);
    assert_eq!(error_message(&save_to_current(&alice, seen.clone())), STALE);
    assert_eq!(error_message(&save_as_new(&alice, "New", seen)), STALE);

    assert_eq!(profiles(&alice), before);
    assert_eq!(current(&alice), json!({ "id": "quiet", "name": "Quiet" }));
}

#[test]
fn the_revision_is_shared_by_every_user_and_stable_while_nothing_changes() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");
    let seen = revision(&alice);
    assert!(seen.as_str().is_some_and(|revision| !revision.is_empty()));
    assert_eq!(revision(&alice), seen);
    assert_eq!(revision(&bob), seen);
}

#[test]
fn another_users_current_profile_is_shown_by_name_and_cannot_be_saved_over() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    import(&alice, quiet_and_loud());
    apply_by_id(&alice, "quiet");
    let quiet = profiles(&alice)
        .into_iter()
        .find(|profile| profile["id"] == "quiet");

    let bob = daemon.user(1001, "bob");
    assert_eq!(current(&bob), json!({ "id": "quiet", "name": "Quiet" }));
    assert!(profiles(&bob).is_empty());
    let response = save_to_current(&bob, revision(&bob));
    assert_eq!(
        error_message(&response),
        "The current profile \"Quiet\" belongs to another user, so you can't save over it. Save \
         as a new profile instead."
    );
    assert_eq!(
        profiles(&alice)
            .into_iter()
            .find(|profile| profile["id"] == "quiet"),
        quiet
    );

    // Bob can still save his own profile, which then becomes current.
    let saved = save_as_new(&bob, "Bob's", revision(&bob));
    assert_eq!(
        current(&alice),
        json!({ "id": saved["payload"]["id"], "name": "Bob's" })
    );
}

#[test]
fn the_current_profile_survives_a_daemon_restart_and_a_reboot() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    import(&alice, quiet_and_loud());
    apply_by_id(&alice, "quiet");
    drop(alice);

    daemon.restart();
    let alice = daemon.user(1000, "alice");
    assert_eq!(current(&alice), json!({ "id": "quiet", "name": "Quiet" }));
    drop(alice);

    daemon.reboot();
    let alice = daemon.user(1000, "alice");
    assert_eq!(current(&alice), json!({ "id": "quiet", "name": "Quiet" }));
}

#[test]
fn current_profile_and_revision_changes_are_pushed_to_every_session() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");
    import(&alice, quiet_and_loud());
    alice.pushed_events();
    bob.pushed_events();

    apply_by_id(&alice, "quiet");
    let pushed = json!({ "type": "tuningStateChanged", "payload": state(&bob) });
    assert_eq!(pushed["payload"]["currentProfile"]["name"], "Quiet");
    assert_eq!(bob.pushed_events(), vec![pushed.clone()]);
    assert_eq!(alice.pushed_events(), vec![pushed]);

    // A live change keeps the current profile but changes the revision.
    let before = revision(&bob);
    alice.send_json(json!({
        "type": "set_power_limit", "payload": { "gpuId": "GPU-123", "limitWatts": 320 }
    }));
    let pushed = bob.pushed_events();
    assert_eq!(
        pushed,
        vec![json!({ "type": "tuningStateChanged", "payload": state(&bob) })]
    );
    assert_ne!(pushed[0]["payload"]["revision"], before);
    assert_eq!(pushed[0]["payload"]["currentProfile"]["name"], "Quiet");
}
