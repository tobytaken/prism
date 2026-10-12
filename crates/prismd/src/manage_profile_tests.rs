//! Seam 1: managing the caller's GPU tuning profiles through daemon session requests.

use serde_json::{Value, json};

use super::harness::{FakeGpuBackend, FakeSession, Harness, Setter, tunable_gpu};

fn profile(id: &str, name: &str, gpu_id: &str) -> Value {
    json!({ "id": id, "name": name, "gpu_id": gpu_id,
        "settings": { "power_limit_watts": { "kind": "value", "value": 300 } } })
}

fn profiles(session: &FakeSession) -> Value {
    session.send_json(json!({ "type": "list_profiles" }))["payload"].clone()
}

fn state(session: &FakeSession) -> Value {
    session.send_json(json!({ "type": "get_tuning_states" }))["payload"][0].clone()
}

fn rename(session: &FakeSession, id: &str, name: &str) -> Value {
    session.send_json(json!({
        "type": "rename_profile", "payload": { "profile_id": id, "name": name }
    }))
}

fn delete(session: &FakeSession, id: &str) -> Value {
    session.send_json(json!({ "type": "delete_profile", "payload": { "profile_id": id } }))
}

#[test]
fn a_profile_deleted_while_an_application_waits_is_never_applied_or_made_current_again() {
    use std::sync::mpsc;
    use std::time::Duration;
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    alice.seed_profiles(json!([profile("quiet", "Quiet", "GPU-123")]));
    let gate = daemon.gpus.block("GPU-123", Setter::PowerLimit);
    let live_write = daemon.send_in_background(daemon.apply_request(
        "GPU-123",
        json!({ "power_limit_watts": { "kind": "value", "value": 350 } }),
    ));
    gate.wait_until_entered();

    let deleting_session = daemon.user(1000, "alice");
    let (deleted, deletion) = mpsc::channel();
    std::thread::spawn(move || {
        deleted.send(delete(&deleting_session, "quiet")).unwrap();
    });
    assert!(matches!(
        deletion.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    let applying_session = daemon.user(1000, "alice");
    let (applied, application) = mpsc::channel();
    std::thread::spawn(move || {
        applied
            .send(applying_session.send_json(json!({
                "type": "apply_profile_by_id", "payload": { "profile_id": "quiet" }
            })))
            .unwrap();
    });
    assert!(matches!(
        application.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    gate.release();
    live_write.response();
    assert_eq!(
        deletion.recv_timeout(Duration::from_secs(5)).unwrap()["payload"]["success"],
        true
    );
    let response = application.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(response["payload"]["code"], "not_found", "{response}");
    assert_eq!(profiles(&alice), json!([]));
    assert_eq!(state(&alice)["current_profile"], Value::Null);
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(350)
    );
    assert_eq!(
        daemon.gpus.writes(),
        vec![("GPU-123".into(), Setter::PowerLimit)]
    );
}

#[test]
fn rename_rejects_a_case_insensitive_clash_on_the_same_gpu_but_allows_the_name_on_another_gpu() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    alice.seed_profiles(json!([
        profile("quiet", "Quiet", "GPU-123"),
        profile("loud", "Loud", "GPU-123"),
        profile("other", "Quiet", "GPU-456")
    ]));
    let before = profiles(&alice);
    let clash = rename(&alice, "quiet", "  lOuD  ");
    assert_eq!(clash["payload"]["code"], "invalid_argument", "{clash}");
    assert_eq!(
        clash["payload"]["message"],
        "A profile named \"lOuD\" already exists for this GPU. Choose another name."
    );
    assert_eq!(profiles(&alice), before);
    assert_eq!(
        rename(&alice, "other", "Loud")["payload"],
        profile("other", "Loud", "GPU-456")
    );
    assert_eq!(
        rename(&alice, "quiet", "  qUIET  ")["payload"],
        profile("quiet", "qUIET", "GPU-123")
    );
    assert!(daemon.gpus.writes().is_empty());
}

#[test]
fn only_the_profiles_owner_can_rename_or_delete_it_and_empty_names_are_rejected() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    alice.seed_profiles(json!([profile("quiet", "Quiet", "GPU-123")]));
    let before = profiles(&alice);
    for other in [daemon.user(1001, "bob"), daemon.user(1000, "carol")] {
        assert_eq!(profiles(&other), json!([]));
        assert_eq!(
            rename(&other, "quiet", "Silent")["payload"]["code"],
            "not_found"
        );
        assert_eq!(delete(&other, "quiet")["payload"]["code"], "not_found");
    }
    let invalid = rename(&alice, "quiet", " \n\t ");
    assert_eq!(
        invalid["payload"]["message"],
        "Enter a name for the profile."
    );
    assert_eq!(
        rename(&alice, "missing", "Silent")["payload"]["code"],
        "not_found"
    );
    assert_eq!(delete(&alice, "missing")["payload"]["code"], "not_found");
    assert_eq!(profiles(&alice), before);
    assert!(daemon.gpus.writes().is_empty());
}

#[test]
fn deleting_a_non_current_profile_keeps_the_current_profile_and_absent_gpus_profiles_can_be_managed()
 {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    alice.seed_profiles(json!([
        profile("quiet", "Quiet", "GPU-123"),
        profile("loud", "Loud", "GPU-123"),
        profile("absent", "Quiet", "GPU-456")
    ]));
    alice.send_json(json!({ "type": "apply_profile_by_id", "payload": { "profile_id": "quiet" } }));
    let before = state(&alice);
    let writes = daemon.gpus.writes();
    assert_eq!(delete(&alice, "loud")["payload"]["success"], true);
    assert_eq!(state(&alice), before);
    assert_eq!(
        rename(&alice, "absent", "Silent")["payload"]["name"],
        "Silent"
    );
    assert_eq!(delete(&alice, "absent")["payload"]["success"], true);
    assert_eq!(
        profiles(&alice),
        json!([profile("quiet", "Quiet", "GPU-123")])
    );
    assert_eq!(daemon.gpus.writes(), writes);
}

#[test]
fn deleting_the_current_profile_clears_its_reference_without_retuning_and_persists_the_deletion() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");
    alice.seed_profiles(json!([
        profile("quiet", "Quiet", "GPU-123"),
        profile("loud", "Loud", "GPU-123")
    ]));
    alice.send_json(json!({
        "type": "apply_profile_by_id", "payload": { "profile_id": "quiet" }
    }));
    alice.pushed_events();
    bob.pushed_events();
    let before = state(&alice);
    let writes = daemon.gpus.writes();

    let deleted = alice.send_json(json!({
        "type": "delete_profile", "payload": { "profile_id": "quiet" }
    }));
    assert_eq!(
        deleted,
        json!({ "type": "command", "payload": { "success": true, "error": null } })
    );
    assert_eq!(
        profiles(&alice),
        json!([profile("loud", "Loud", "GPU-123")])
    );
    assert_eq!(state(&alice)["current_profile"], Value::Null);
    assert_eq!(state(&alice)["reported"], before["reported"]);
    assert_ne!(state(&alice)["revision"], before["revision"]);
    assert_eq!(daemon.gpus.writes(), writes);
    assert_eq!(
        bob.pushed_events_of("tuning_state_changed"),
        vec![json!({
            "type": "tuning_state_changed", "payload": state(&bob)
        })]
    );
    drop(alice);
    drop(bob);
    daemon.reboot();
    let alice = daemon.user(1000, "alice");
    assert_eq!(
        profiles(&alice),
        json!([profile("loud", "Loud", "GPU-123")])
    );
    assert_eq!(state(&alice)["current_profile"], Value::Null);
    let application = alice.send_json(json!({
        "type": "apply_profile_by_id", "payload": { "profile_id": "quiet" }
    }));
    assert_eq!(application["payload"]["code"], "not_found");
}

#[test]
fn renaming_trims_the_name_keeps_the_id_and_updates_the_current_profile_across_sessions_and_restarts()
 {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");
    alice.seed_profiles(json!([profile("quiet", "Quiet", "GPU-123")]));
    alice.send_json(json!({
        "type": "apply_profile_by_id", "payload": { "profile_id": "quiet" }
    }));
    alice.pushed_events();
    bob.pushed_events();
    let before = state(&alice);
    let writes = daemon.gpus.writes();

    let renamed = rename(&alice, "quiet", "  Silent  ");
    assert_eq!(renamed["type"], "profile", "{renamed}");
    assert_eq!(renamed["payload"], profile("quiet", "Silent", "GPU-123"));
    assert_eq!(
        profiles(&alice),
        json!([profile("quiet", "Silent", "GPU-123")])
    );
    assert_eq!(
        state(&bob)["current_profile"],
        json!({ "id": "quiet", "name": "Silent" })
    );
    assert_ne!(state(&alice)["revision"], before["revision"]);
    assert_eq!(
        bob.pushed_events_of("tuning_state_changed"),
        vec![json!({
            "type": "tuning_state_changed", "payload": state(&bob)
        })]
    );
    assert_eq!(daemon.gpus.writes(), writes);
    drop(alice);
    drop(bob);

    daemon.restart();
    let alice = daemon.user(1000, "alice");
    assert_eq!(
        profiles(&alice),
        json!([profile("quiet", "Silent", "GPU-123")])
    );
    assert_eq!(
        state(&alice)["current_profile"],
        json!({ "id": "quiet", "name": "Silent" })
    );
}
