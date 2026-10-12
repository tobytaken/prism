//! Seam 1: the daemon's boot-scoped Activity: profile applications, automatic switches and
//! startup outcomes, in each session's snapshot and pushed as `ActivityRecorded`, kept across a
//! daemon restart and cleared at reboot. Also each user's startup state, pushed as
//! `StartupStateChanged` when an outcome finishes or is dismissed.

use serde_json::{Value, json};

use super::harness::{FakeGpuBackend, FakeSession, Harness, tunable_gpu, tunable_gpu_with_id};

const GPU: &str = "GPU-123";

fn value(watts: i64) -> Value {
    json!({ "kind": "value", "value": watts })
}

fn seed_quiet(session: &FakeSession) {
    session.seed_profiles(json!([{ "id": "quiet", "name": "Quiet", "gpu_id": GPU,
        "settings": { "power_limit_watts": value(300) } }]));
}

fn ok(response: Value) -> Value {
    assert_ne!(response["type"], "error", "{response}");
    response["payload"].clone()
}

fn apply_quiet(session: &FakeSession) {
    ok(session
        .send_json(json!({ "type": "apply_profile_by_id", "payload": { "profile_id": "quiet" } })));
}

/// The entries pushed to the session since it last looked.
fn pushed_activity(session: &FakeSession) -> Vec<Value> {
    session
        .pushed_events_of("activity_recorded")
        .into_iter()
        .map(|event| event["payload"].clone())
        .collect()
}

fn snapshot_activity(session: &FakeSession) -> Vec<Value> {
    serde_json::to_value(&session.snapshot().activity)
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

fn kinds(entries: &[Value]) -> Vec<&str> {
    entries
        .iter()
        .map(|entry| entry["event"]["kind"].as_str().unwrap())
        .collect()
}

#[test]
fn a_profile_application_is_pushed_to_its_user_and_in_their_next_snapshot_only() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    daemon.clock.set(1_791_800_000_000);
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");
    seed_quiet(&alice);

    apply_quiet(&alice);

    let pushed = pushed_activity(&alice);
    assert_eq!(pushed.len(), 1, "{pushed:?}");
    assert_eq!(pushed[0]["gpu_id"], GPU);
    assert_eq!(pushed[0]["at_unix_ms"], 1_791_800_000_000u64);
    assert_eq!(pushed[0]["event"]["kind"], "profile_applied");
    assert_eq!(
        pushed[0]["event"]["profile"],
        json!({ "id": "quiet", "name": "Quiet" })
    );
    assert_eq!(
        pushed[0]["event"]["application"]["power"]["status"],
        "succeeded"
    );
    // Bob doesn't learn what Alice applies, now or in his next session.
    assert_eq!(pushed_activity(&bob), Vec::<Value>::new());
    assert_eq!(snapshot_activity(&daemon.user(1000, "alice")), pushed);
    assert_eq!(
        snapshot_activity(&daemon.user(1001, "bob")),
        Vec::<Value>::new()
    );
}

#[test]
fn applying_unapplied_tuning_changes_is_recorded_without_a_profile() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");

    ok(alice.apply(GPU, json!({ "power_limit_watts": value(320) })));

    let pushed = pushed_activity(&alice);
    assert_eq!(kinds(&pushed), ["profile_applied"]);
    assert_eq!(pushed[0]["event"]["profile"], Value::Null);
}

#[test]
fn activity_survives_a_daemon_restart_within_a_boot_and_is_cleared_at_reboot() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    seed_quiet(&alice);
    apply_quiet(&alice);
    apply_quiet(&alice);
    let before = snapshot_activity(&daemon.user(1000, "alice"));
    assert_eq!(kinds(&before), ["profile_applied", "profile_applied"]);
    drop(alice);

    daemon.restart();
    let alice = daemon.user(1000, "alice");
    assert_eq!(snapshot_activity(&alice), before);
    // Entries recorded after the restart carry on from the ones before it.
    apply_quiet(&alice);
    let ids: Vec<_> = snapshot_activity(&daemon.user(1000, "alice"))
        .iter()
        .map(|entry| entry["id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids.len(), 3);
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]), "{ids:?}");
    drop(alice);

    daemon.reboot();
    assert_eq!(
        snapshot_activity(&daemon.user(1000, "alice")),
        Vec::<Value>::new()
    );
}

fn startup_outcomes(session: &FakeSession) -> Vec<Value> {
    serde_json::to_value(&session.snapshot().startup.outcomes)
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

/// A daemon that applied Alice's Quiet to the GPU at this boot's startup, while no Prism was open.
fn started_with_quiet() -> Harness {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    seed_quiet(&alice);
    ok(
        alice.send_json(json!({ "type": "set_startup_profile", "payload": {
        "gpu_id": GPU, "profile_id": "quiet"
    } })),
    );
    ok(
        alice.send_json(json!({ "type": "set_startup_behavior", "payload": {
        "behavior": "startup_profiles"
    } })),
    );
    drop(alice);
    daemon.reboot();
    daemon.core().wait_for_startup();
    daemon
}

#[test]
fn startup_outcomes_reach_activity_and_the_snapshot_while_prism_was_closed() {
    let daemon = started_with_quiet();
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");

    for session in [&alice, &bob] {
        let activity = snapshot_activity(session);
        assert_eq!(kinds(&activity), ["startup_ran"], "{activity:?}");
        let outcome = &activity[0]["event"]["outcome"];
        assert_eq!(outcome["status"], "applied");
        assert_eq!(outcome["profile"]["name"], "Quiet");
        assert_eq!(outcome["application"]["power"]["status"], "succeeded");
        assert_eq!(startup_outcomes(session), [outcome.clone()]);
        assert_eq!(session.snapshot().startup.behavior.is_off(), false);
    }
}

#[test]
fn startup_with_nothing_to_apply_records_no_activity() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    daemon.reboot();
    daemon.core().wait_for_startup();
    let alice = daemon.user(1000, "alice");
    assert_eq!(snapshot_activity(&alice), Vec::<Value>::new());
    // The outcome still says why nothing was applied.
    assert_eq!(startup_outcomes(&alice)[0]["status"], "not_attempted");
}

#[test]
fn dismissing_a_startup_outcome_pushes_the_callers_startup_state_without_it() {
    let daemon = started_with_quiet();
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");

    ok(alice.send_json(json!({ "type": "dismiss_startup_outcome", "payload": { "gpu_id": GPU } })));

    let pushed = alice.pushed_events_of("startup_state_changed");
    assert_eq!(pushed.len(), 1, "{pushed:?}");
    assert_eq!(pushed[0]["payload"]["outcomes"], json!([]));
    assert_eq!(pushed[0]["payload"]["behavior"], "startup_profiles");
    // Bob's outcomes are his own to dismiss.
    assert_eq!(
        bob.pushed_events_of("startup_state_changed"),
        Vec::<Value>::new()
    );
    assert_eq!(startup_outcomes(&daemon.user(1001, "bob")).len(), 1);
    assert_eq!(
        startup_outcomes(&daemon.user(1000, "alice")),
        Vec::<Value>::new()
    );
    // The startup outcome stays in Activity as what happened.
    assert_eq!(
        kinds(&snapshot_activity(&daemon.user(1000, "alice"))),
        ["startup_ran"]
    );
}

#[test]
fn changing_the_startup_behavior_pushes_the_new_startup_state_to_everyone() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");

    ok(
        alice.send_json(json!({ "type": "set_startup_behavior", "payload": {
        "behavior": "restore_last_applied"
    } })),
    );

    for session in [&alice, &bob] {
        let pushed = session.pushed_events_of("startup_state_changed");
        assert_eq!(pushed.len(), 1, "{pushed:?}");
        assert_eq!(pushed[0]["payload"]["behavior"], "restore_last_applied");
    }
}

const LINKED: &str = "GPU-A";
const DOTA: u32 = 570;

#[test]
fn an_automatic_switch_and_its_restoration_are_recorded_for_the_games_user_only() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu_with_id(LINKED)]));
    daemon.clock.advance(5_000);
    daemon.install_steam_app("alice", DOTA, "Dota 2");
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");
    alice.seed_profiles(json!([{ "id": "gaming", "name": "Gaming", "gpu_id": LINKED,
        "settings": { "power_limit_watts": value(350) } }]));
    ok(alice.send_json(json!({ "type": "add_steam_game", "payload": { "app_id": DOTA } })));
    ok(
        alice.send_json(json!({ "type": "set_game_profile", "payload": {
        "game": { "kind": "steam", "app_id": DOTA }, "gpu_id": LINKED, "profile_id": "gaming"
    } })),
    );
    ok(alice
        .send_json(json!({ "type": "set_automatic_switching", "payload": { "enabled": true } })));
    daemon.settle();

    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    daemon.end_process(123);
    daemon.scan();

    let pushed = pushed_activity(&alice);
    assert_eq!(
        kinds(&pushed),
        ["game_profile_applied", "pre_game_values_restored"]
    );
    let applied = &pushed[0]["event"];
    assert_eq!(applied["game_name"], "Dota 2");
    assert_eq!(applied["game"], json!({ "kind": "steam", "app_id": DOTA }));
    assert_eq!(
        applied["profile"],
        json!({ "id": "gaming", "name": "Gaming" })
    );
    assert_eq!(applied["application"]["power"]["status"], "succeeded");
    assert_eq!(
        pushed[1]["event"]["application"]["power"]["status"],
        "succeeded"
    );
    assert_eq!(pushed_activity(&bob), Vec::<Value>::new());
}
