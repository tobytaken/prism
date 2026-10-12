//! Seam 1: the caller's profiles, with each GPU's startup and current profile by name, reach
//! sessions in their snapshot and as `ProfilesChanged` whenever they change.

use serde_json::{Value, json};

use super::harness::{FakeGpuBackend, FakeSession, Harness, tunable_gpu};

fn quiet_and_loud() -> Value {
    json!([
        { "id": "quiet", "name": "Quiet", "gpu_id": "GPU-123",
          "settings": { "power_limit_watts": { "kind": "value", "value": 300 } } },
        { "id": "loud", "name": "Loud", "gpu_id": "GPU-123",
          "settings": { "power_limit_watts": { "kind": "value", "value": 500 } } }
    ])
}

/// The profiles pushed to the session since it last looked.
fn pushed_profiles(session: &FakeSession) -> Vec<Value> {
    session
        .pushed_events_of("profiles_changed")
        .into_iter()
        .map(|event| event["payload"].clone())
        .collect()
}

/// The one profiles state pushed to the session since it last looked.
fn pushed_once(session: &FakeSession) -> Value {
    let pushed = pushed_profiles(session);
    assert_eq!(pushed.len(), 1, "{pushed:?}");
    pushed[0].clone()
}

fn snapshot_profiles(session: &FakeSession) -> Value {
    serde_json::to_value(&session.snapshot().profiles).unwrap()
}

fn names(state: &Value) -> Vec<&str> {
    state["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|profile| profile["name"].as_str().unwrap())
        .collect()
}

fn send_ok(session: &FakeSession, request: Value) -> Value {
    let response = session.send_json(request);
    assert_ne!(response["type"], "error", "{response}");
    response
}

fn apply_by_id(session: &FakeSession, profile_id: &str) {
    send_ok(
        session,
        json!({ "type": "apply_profile_by_id", "payload": { "profile_id": profile_id } }),
    );
}

fn set_startup(session: &FakeSession, profile_id: Option<&str>) {
    send_ok(
        session,
        json!({ "type": "set_startup_profile", "payload": { "gpu_id": "GPU-123", "profile_id": profile_id } }),
    );
}

#[test]
fn the_snapshot_carries_the_callers_profiles_and_every_gpus_startup_and_current_names() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    assert_eq!(
        snapshot_profiles(&alice),
        json!({ "profiles": [], "startup_profiles": {}, "current_profiles": {} })
    );
    alice.seed_profiles(quiet_and_loud());
    set_startup(&alice, Some("quiet"));
    apply_by_id(&alice, "loud");

    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");

    let profiles = snapshot_profiles(&alice);
    assert_eq!(names(&profiles), ["Quiet", "Loud"]);
    assert_eq!(
        profiles["startup_profiles"],
        json!({ "GPU-123": { "id": "quiet", "name": "Quiet" } })
    );
    assert_eq!(
        profiles["current_profiles"],
        json!({ "GPU-123": { "id": "loud", "name": "Loud" } })
    );
    // Bob sees which profiles are in use by name, but not Alice's profiles themselves.
    let profiles = snapshot_profiles(&bob);
    assert_eq!(profiles["profiles"], json!([]));
    assert_eq!(profiles["startup_profiles"]["GPU-123"]["name"], "Quiet");
    assert_eq!(profiles["current_profiles"]["GPU-123"]["name"], "Loud");
}

#[test]
fn saving_a_profile_pushes_it_to_its_owner_and_its_name_to_everyone_as_current() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");

    send_ok(
        &alice,
        json!({ "type": "save_profile_as_new", "payload": {
            "gpu_id": "GPU-123", "name": "Quiet", "revision": daemon.revision("GPU-123")
        } }),
    );

    let to_alice = pushed_once(&alice);
    assert_eq!(names(&to_alice), ["Quiet"]);
    let id = to_alice["profiles"][0]["id"].clone();
    assert_eq!(
        to_alice["current_profiles"]["GPU-123"],
        json!({ "id": id, "name": "Quiet" })
    );
    let to_bob = pushed_once(&bob);
    assert_eq!(to_bob["profiles"], json!([]));
    assert_eq!(to_bob["current_profiles"]["GPU-123"]["name"], "Quiet");
}

#[test]
fn applying_a_profile_pushes_the_gpus_new_current_profile() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    alice.seed_profiles(quiet_and_loud());
    pushed_profiles(&alice);

    apply_by_id(&alice, "quiet");
    assert_eq!(
        pushed_once(&alice)["current_profiles"],
        json!({ "GPU-123": { "id": "quiet", "name": "Quiet" } })
    );

    apply_by_id(&alice, "loud");
    assert_eq!(
        pushed_once(&alice)["current_profiles"],
        json!({ "GPU-123": { "id": "loud", "name": "Loud" } })
    );
}

#[test]
fn designating_a_startup_profile_pushes_it_and_clearing_it_pushes_none() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");
    alice.seed_profiles(quiet_and_loud());
    pushed_profiles(&alice);
    pushed_profiles(&bob);

    set_startup(&alice, Some("quiet"));
    assert_eq!(
        pushed_once(&alice)["startup_profiles"],
        json!({ "GPU-123": { "id": "quiet", "name": "Quiet" } })
    );
    assert_eq!(
        pushed_once(&bob)["startup_profiles"]["GPU-123"]["name"],
        "Quiet"
    );

    set_startup(&alice, None);
    assert_eq!(pushed_once(&alice)["startup_profiles"], json!({}));
}

#[test]
fn renaming_or_deleting_a_profile_pushes_the_change_to_every_session_it_shows_in() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");
    alice.seed_profiles(quiet_and_loud());
    set_startup(&alice, Some("quiet"));
    apply_by_id(&alice, "quiet");
    pushed_profiles(&alice);
    pushed_profiles(&bob);

    send_ok(
        &alice,
        json!({ "type": "rename_profile", "payload": { "profile_id": "quiet", "name": "Silent" } }),
    );
    let to_alice = pushed_once(&alice);
    assert_eq!(names(&to_alice), ["Silent", "Loud"]);
    assert_eq!(to_alice["startup_profiles"]["GPU-123"]["name"], "Silent");
    let to_bob = pushed_once(&bob);
    assert_eq!(to_bob["current_profiles"]["GPU-123"]["name"], "Silent");

    send_ok(
        &alice,
        json!({ "type": "delete_profile", "payload": { "profile_id": "quiet" } }),
    );
    let to_alice = pushed_once(&alice);
    assert_eq!(names(&to_alice), ["Loud"]);
    assert_eq!(to_alice["startup_profiles"], json!({}));
    assert_eq!(to_alice["current_profiles"], json!({}));
    assert_eq!(pushed_once(&bob)["current_profiles"], json!({}));
}

#[test]
fn a_write_that_leaves_profiles_unchanged_pushes_no_profiles() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    alice.seed_profiles(quiet_and_loud());
    apply_by_id(&alice, "quiet");
    pushed_profiles(&alice);

    // A staged Apply records last applied tuning values and keeps the current profile.
    let response = alice.apply(
        "GPU-123",
        json!({ "graphics_offset_mhz": { "kind": "value", "value": 50 } }),
    );
    assert_eq!(response["type"], "profile_application", "{response}");

    assert_eq!(pushed_profiles(&alice), Vec::<Value>::new());
}

#[test]
fn another_users_profiles_never_reach_a_session() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");

    alice.seed_profiles(quiet_and_loud());

    assert_eq!(names(&pushed_once(&alice)), ["Quiet", "Loud"]);
    // Nothing Bob sees changed.
    assert_eq!(pushed_profiles(&bob), Vec::<Value>::new());
}

#[test]
fn a_user_who_reconnects_is_pushed_a_change_back_to_what_they_last_saw_before_leaving() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");
    bob.seed_profiles(json!([
        { "id": "loud", "name": "Loud", "gpu_id": "GPU-123",
          "settings": { "power_limit_watts": { "kind": "value", "value": 500 } } }
    ]));
    pushed_profiles(&alice);
    drop(alice);

    apply_by_id(&bob, "loud");
    let alice = daemon.user(1000, "alice");
    assert_eq!(
        snapshot_profiles(&alice)["current_profiles"]["GPU-123"]["name"],
        "Loud"
    );
    send_ok(
        &bob,
        json!({ "type": "delete_profile", "payload": { "profile_id": "loud" } }),
    );

    assert_eq!(pushed_once(&alice)["current_profiles"], json!({}));
}
