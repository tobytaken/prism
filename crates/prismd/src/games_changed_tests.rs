//! Seam 1: the caller's games, with their associations and the automatic switching opt-in, reach
//! sessions in their snapshot and as `GamesChanged` whenever they change.

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

/// The games pushed to the session since it last looked.
fn pushed_games(session: &FakeSession) -> Vec<Value> {
    session
        .pushed_events_of("games_changed")
        .into_iter()
        .map(|event| event["payload"].clone())
        .collect()
}

/// The one games state pushed to the session since it last looked.
fn pushed_once(session: &FakeSession) -> Value {
    let pushed = pushed_games(session);
    assert_eq!(pushed.len(), 1, "{pushed:?}");
    pushed[0].clone()
}

fn snapshot_games(session: &FakeSession) -> Value {
    serde_json::to_value(&session.snapshot().games).unwrap()
}

fn send_ok(session: &FakeSession, request: Value) -> Value {
    let response = session.send_json(request);
    assert_ne!(response["type"], "error", "{response}");
    response
}

fn add_dota(session: &FakeSession) {
    send_ok(
        session,
        json!({ "type": "add_steam_game", "payload": { "app_id": 570 } }),
    );
}

fn link_dota(session: &FakeSession, profile_id: Option<&str>) -> Value {
    session.send_json(json!({
        "type": "set_game_profile",
        "payload": {
            "game": { "kind": "steam", "app_id": 570 },
            "gpu_id": "GPU-123",
            "profile_id": profile_id
        }
    }))
}

fn dota(profiles: Value) -> Value {
    json!({
        "game": { "kind": "steam", "app_id": 570 },
        "name": "Dota 2",
        "installed": true,
        "profiles": profiles
    })
}

#[test]
fn the_snapshot_carries_the_callers_games_and_switching_opt_in() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    daemon.install_steam_app("alice", 570, "Dota 2");
    let alice = daemon.user(1000, "alice");
    assert_eq!(
        snapshot_games(&alice),
        json!({ "automatic_switching": false, "games": [] })
    );
    alice.seed_profiles(quiet_and_loud());
    add_dota(&alice);
    link_dota(&alice, Some("quiet"));
    send_ok(
        &alice,
        json!({ "type": "set_automatic_switching", "payload": { "enabled": true } }),
    );

    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");

    assert_eq!(
        snapshot_games(&alice),
        json!({ "automatic_switching": true, "games": [dota(json!({ "GPU-123": "quiet" }))] })
    );
    // Alice's games are her own.
    assert_eq!(
        snapshot_games(&bob),
        json!({ "automatic_switching": false, "games": [] })
    );
}

#[test]
fn adding_linking_and_removing_a_game_pushes_the_callers_games_each_time() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    daemon.install_steam_app("alice", 570, "Dota 2");
    let alice = daemon.user(1000, "alice");
    alice.seed_profiles(quiet_and_loud());

    add_dota(&alice);
    assert_eq!(pushed_once(&alice)["games"], json!([dota(json!({}))]));

    link_dota(&alice, Some("loud"));
    assert_eq!(
        pushed_once(&alice)["games"],
        json!([dota(json!({ "GPU-123": "loud" }))])
    );

    link_dota(&alice, None);
    assert_eq!(pushed_once(&alice)["games"], json!([dota(json!({}))]));

    send_ok(
        &alice,
        json!({ "type": "remove_game", "payload": { "game": { "kind": "steam", "app_id": 570 } } }),
    );
    assert_eq!(pushed_once(&alice)["games"], json!([]));
}

#[test]
fn turning_automatic_switching_on_or_off_pushes_the_opt_in() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");

    send_ok(
        &alice,
        json!({ "type": "set_automatic_switching", "payload": { "enabled": true } }),
    );
    assert_eq!(pushed_once(&alice)["automatic_switching"], true);

    send_ok(
        &alice,
        json!({ "type": "set_automatic_switching", "payload": { "enabled": false } }),
    );
    assert_eq!(pushed_once(&alice)["automatic_switching"], false);
}

#[test]
fn deleting_a_linked_profile_pushes_the_game_without_its_link() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    daemon.install_steam_app("alice", 570, "Dota 2");
    let alice = daemon.user(1000, "alice");
    alice.seed_profiles(quiet_and_loud());
    add_dota(&alice);
    link_dota(&alice, Some("quiet"));
    pushed_games(&alice);

    send_ok(
        &alice,
        json!({ "type": "delete_profile", "payload": { "profile_id": "quiet" } }),
    );

    assert_eq!(pushed_once(&alice)["games"], json!([dota(json!({}))]));
}

#[test]
fn a_change_that_leaves_the_games_as_they_were_pushes_none() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    daemon.install_steam_app("alice", 570, "Dota 2");
    let alice = daemon.user(1000, "alice");
    alice.seed_profiles(quiet_and_loud());
    add_dota(&alice);
    pushed_games(&alice);

    // Adding it again keeps it as it is, and a refused link changes nothing.
    add_dota(&alice);
    let refused = alice.send_json(json!({
        "type": "set_game_profile",
        "payload": {
            "game": { "kind": "steam", "app_id": 620 },
            "gpu_id": "GPU-123",
            "profile_id": "quiet"
        }
    }));
    assert_eq!(refused["type"], "error", "{refused}");
    // Deleting a profile no game links to, and renaming a linked one, leave the games alone.
    link_dota(&alice, Some("quiet"));
    pushed_games(&alice);
    send_ok(
        &alice,
        json!({ "type": "delete_profile", "payload": { "profile_id": "loud" } }),
    );
    send_ok(
        &alice,
        json!({ "type": "rename_profile", "payload": { "profile_id": "quiet", "name": "Silent" } }),
    );

    assert_eq!(pushed_games(&alice), Vec::<Value>::new());
}

#[test]
fn every_session_of_the_caller_is_pushed_their_games_and_no_one_elses() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    daemon.install_steam_app("alice", 570, "Dota 2");
    let alice = daemon.user(1000, "alice");
    let alice_elsewhere = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");

    add_dota(&alice);

    assert_eq!(pushed_once(&alice)["games"], json!([dota(json!({}))]));
    assert_eq!(
        pushed_once(&alice_elsewhere)["games"],
        json!([dota(json!({}))])
    );
    assert_eq!(pushed_games(&bob), Vec::<Value>::new());
}
