//! Seam 1: automatic switching while more than one linked game runs on a GPU, including games of
//! different users sharing it, driven by fixture processes and complete scans.

use serde_json::{json, Value};

use super::harness::{tunable_gpu_with_id, FakeGpuBackend, FakeSession, Harness, Setter};
use super::tuning_store::TUNING_STORE_FILE_NAME;

const GPU: &str = "GPU-A";
const DOTA: u32 = 570;
const CS: u32 = 730;

fn value(value: i64) -> Value {
    json!({ "kind": "value", "value": value })
}

fn ok(response: Value) -> Value {
    assert_ne!(response["type"], "error", "{response}");
    response["payload"].clone()
}

/// GPU-A, whose ranges fit the example from #8: power 100–600 W, memory offset ±1000 MHz.
fn harness() -> Harness {
    let mut gpu = tunable_gpu_with_id(GPU);
    gpu.power.min_limit_watts = Some(100);
    gpu.clocks.memory.min_offset_mhz = Some(-1000);
    gpu.clocks.memory.max_offset_mhz = Some(1000);
    Harness::start(FakeGpuBackend::with_gpus(vec![gpu]))
}

/// A user who opted in, with the given profiles on GPU-A and the given Steam games added.
fn opted_in_user(
    daemon: &Harness,
    uid: u32,
    username: &str,
    profiles: Value,
    games: &[(u32, &str)],
) -> FakeSession {
    for (app_id, name) in games {
        daemon.install_steam_app(username, *app_id, name);
    }
    let session = daemon.user(uid, username);
    ok(session
        .send_json(json!({ "type": "import_profiles", "payload": { "profiles": profiles } })));
    for (app_id, _) in games {
        ok(session.send_json(json!({ "type": "add_steam_game", "payload": { "appId": app_id } })));
    }
    ok(session
        .send_json(json!({ "type": "set_automatic_switching", "payload": { "enabled": true } })));
    daemon.settle();
    session
}

fn profile(id: &str, name: &str, settings: Value) -> Value {
    json!({ "id": id, "name": name, "gpuId": GPU, "settings": settings })
}

fn link(session: &FakeSession, app_id: u32, profile_id: &str) {
    ok(
        session.send_json(json!({ "type": "set_game_profile", "payload": {
        "game": { "kind": "steam", "appId": app_id }, "gpuId": GPU, "profileId": profile_id
    } })),
    );
}

fn apply(daemon: &Harness, session: &FakeSession, profile_id: &str) {
    ok(session.send_json(
        json!({ "type": "apply_profile_by_id", "payload": { "profileId": profile_id } }),
    ));
    daemon.settle();
}

/// GPU-A's power limit and graphics and memory clock offsets as the hardware reports them.
fn values(daemon: &Harness) -> (Option<u32>, Option<i32>, Option<i32>) {
    let gpu = daemon.gpus.gpu(GPU);
    (
        gpu.power.current_limit_watts,
        gpu.clocks.graphics.current_offset_mhz,
        gpu.clocks.memory.current_offset_mhz,
    )
}

fn current_profile(session: &FakeSession) -> Value {
    ok(session.send_json(json!({ "type": "get_tuning_states" })))
        .as_array()
        .unwrap()
        .iter()
        .find(|state| state["gpuId"] == GPU)
        .expect("the GPU has a tuning state")["currentProfile"]["name"]
        .clone()
}

fn gpu_status(session: &FakeSession) -> Value {
    let response = session.send_json(json!({ "type": "get_automatic_switching_status" }));
    assert_eq!(response["type"], "automatic_switching_status", "{response}");
    response["payload"]["gpus"].clone()
}

fn active(app_id: u32, game_name: &str, profile_id: &str, profile_name: &str) -> Value {
    json!([{ "gpuId": GPU, "state": {
        "status": "active",
        "game": { "kind": "steam", "appId": app_id },
        "gameName": game_name,
        "profile": { "id": profile_id, "name": profile_name }
    } }])
}

fn another_user() -> Value {
    json!([{ "gpuId": GPU, "state": { "status": "anotherUser" } }])
}

fn writes(daemon: &Harness) -> Vec<Setter> {
    daemon
        .gpus
        .writes()
        .into_iter()
        .map(|(_, setter)| setter)
        .collect()
}

fn last_applied(daemon: &Harness) -> Value {
    let document: Value =
        serde_json::from_slice(&daemon.persistent_file(TUNING_STORE_FILE_NAME).unwrap()).unwrap();
    document["lastApplied"].clone()
}

#[test]
fn overlapping_games_keep_the_original_baseline_and_never_write_graphics() {
    // Quiet (200 W, +0 memory) → A (300 W) → B (320 W, +500 memory) → B ends → A ends.
    let daemon = harness();
    let alice = opted_in_user(
        &daemon,
        1000,
        "alice",
        json!([
            profile(
                "quiet",
                "Quiet",
                json!({ "powerLimitWatts": value(200),
                "memoryOffsetMHz": value(0) })
            ),
            profile("a", "A", json!({ "powerLimitWatts": value(300) })),
            profile(
                "b",
                "B",
                json!({ "powerLimitWatts": value(320),
                "memoryOffsetMHz": value(500) })
            ),
        ]),
        &[(DOTA, "Dota 2"), (CS, "Counter-Strike")],
    );
    link(&alice, DOTA, "a");
    link(&alice, CS, "b");
    apply(&daemon, &alice, "quiet");
    let manual = last_applied(&daemon);
    assert_eq!(values(&daemon), (Some(200), Some(120), Some(0)));

    daemon.run_steam_game(100, 1000, 1_000, DOTA);
    daemon.scan();
    assert_eq!(values(&daemon), (Some(300), Some(120), Some(0)));
    assert_eq!(current_profile(&alice), "A");
    assert_eq!(gpu_status(&alice), active(DOTA, "Dota 2", "a", "A"));

    daemon.run_steam_game(200, 1000, 2_000, CS);
    daemon.scan();
    assert_eq!(values(&daemon), (Some(320), Some(120), Some(500)));
    assert_eq!(current_profile(&alice), "B");
    assert_eq!(gpu_status(&alice), active(CS, "Counter-Strike", "b", "B"));

    // A omits memory, so memory returns to its pre-game +0 rather than staying at B's +500.
    daemon.end_process(200);
    daemon.scan();
    assert_eq!(values(&daemon), (Some(300), Some(120), Some(0)));
    assert_eq!(current_profile(&alice), "A");

    // Power returns to 200 W, captured when A first wrote it, not to B's or A's values.
    daemon.end_process(100);
    daemon.scan();
    assert_eq!(values(&daemon), (Some(200), Some(120), Some(0)));
    assert_eq!(current_profile(&alice), "Quiet");
    assert_eq!(gpu_status(&alice), json!([]));

    assert!(!writes(&daemon).contains(&Setter::GraphicsOffset));
    assert_eq!(last_applied(&daemon), manual);
}

/// Alice with Dota 2 linked to her Gaming profile, and Bob with Counter-Strike linked to his
/// Competitive profile, both on GPU-A and both opted in. Alice's Daily is current.
fn shared_gpu() -> (Harness, FakeSession, FakeSession) {
    let daemon = harness();
    let alice = opted_in_user(
        &daemon,
        1000,
        "alice",
        json!([
            profile("daily", "Daily", json!({ "powerLimitWatts": value(250) })),
            profile(
                "gaming",
                "Gaming",
                json!({ "powerLimitWatts": value(350),
                "graphicsOffsetMHz": value(150) })
            ),
        ]),
        &[(DOTA, "Dota 2")],
    );
    let bob = opted_in_user(
        &daemon,
        1001,
        "bob",
        json!([profile(
            "competitive",
            "Competitive",
            json!({ "powerLimitWatts": value(400),
            "memoryOffsetMHz": value(100) })
        )]),
        &[(CS, "Counter-Strike")],
    );
    link(&alice, DOTA, "gaming");
    link(&bob, CS, "competitive");
    apply(&daemon, &alice, "daily");
    (daemon, alice, bob)
}

const PRE_GAME: (Option<u32>, Option<i32>, Option<i32>) = (Some(250), Some(120), Some(0));
const ALICES: (Option<u32>, Option<i32>, Option<i32>) = (Some(350), Some(150), Some(0));
const BOBS: (Option<u32>, Option<i32>, Option<i32>) = (Some(400), Some(120), Some(100));

fn assert_alice_wins(daemon: &Harness, alice: &FakeSession, bob: &FakeSession) {
    assert_eq!(values(daemon), ALICES);
    assert_eq!(current_profile(alice), "Gaming");
    assert_eq!(
        gpu_status(alice),
        active(DOTA, "Dota 2", "gaming", "Gaming")
    );
    assert_eq!(gpu_status(bob), another_user());
}

fn assert_bob_wins(daemon: &Harness, alice: &FakeSession, bob: &FakeSession) {
    assert_eq!(values(daemon), BOBS);
    assert_eq!(current_profile(bob), "Competitive");
    assert_eq!(
        gpu_status(bob),
        active(CS, "Counter-Strike", "competitive", "Competitive")
    );
    assert_eq!(gpu_status(alice), another_user());
}

fn assert_restored(daemon: &Harness, alice: &FakeSession, manual: &Value) {
    assert_eq!(values(daemon), PRE_GAME);
    assert_eq!(current_profile(alice), "Daily");
    assert_eq!(gpu_status(alice), json!([]));
    assert_eq!(&last_applied(daemon), manual);
}

#[test]
fn on_a_shared_gpu_the_most_recently_started_game_of_either_user_wins() {
    let (daemon, alice, bob) = shared_gpu();
    let manual = last_applied(&daemon);

    daemon.run_steam_game(100, 1000, 1_000, DOTA);
    daemon.scan();
    assert_alice_wins(&daemon, &alice, &bob);

    daemon.run_steam_game(200, 1001, 2_000, CS);
    daemon.scan();
    assert_bob_wins(&daemon, &alice, &bob);

    // A helper Alice's game starts later doesn't make it the most recently started game.
    daemon.run_process(300, 1000, 3_000, "SteamAppId=570\0", "wineserver\0");
    let before = writes(&daemon);
    daemon.scan();
    assert_eq!(writes(&daemon), before);
    assert_bob_wins(&daemon, &alice, &bob);

    daemon.end_process(200);
    daemon.scan();
    assert_alice_wins(&daemon, &alice, &bob);

    daemon.end_process(100);
    daemon.end_process(300);
    daemon.scan();
    assert_restored(&daemon, &alice, &manual);
}

#[test]
fn on_a_shared_gpu_the_most_recently_started_game_wins_across_daemon_restarts() {
    let (mut daemon, alice, bob) = shared_gpu();
    let manual = last_applied(&daemon);
    daemon.run_steam_game(100, 1000, 1_000, DOTA);
    daemon.scan();
    assert_alice_wins(&daemon, &alice, &bob);
    drop((alice, bob));

    // Bob starts his game while the daemon is down.
    daemon.run_steam_game(200, 1001, 2_000, CS);
    daemon.restart();
    daemon.settle();
    daemon.scan();
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");
    assert_bob_wins(&daemon, &alice, &bob);
    drop((alice, bob));

    // Another restart with both games still running writes nothing.
    let before = writes(&daemon);
    daemon.restart();
    daemon.settle();
    daemon.scan();
    assert_eq!(writes(&daemon), before);
    let alice = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");
    assert_bob_wins(&daemon, &alice, &bob);

    daemon.end_process(200);
    daemon.scan();
    assert_alice_wins(&daemon, &alice, &bob);

    // The pre-game values captured before the restarts return.
    daemon.end_process(100);
    daemon.scan();
    assert_restored(&daemon, &alice, &manual);
}

#[test]
fn an_older_game_becoming_eligible_later_doesnt_take_over() {
    let (daemon, alice, bob) = shared_gpu();
    ok(alice
        .send_json(json!({ "type": "set_automatic_switching", "payload": { "enabled": false } })));
    daemon.run_steam_game(100, 1000, 1_000, DOTA);
    daemon.run_steam_game(200, 1001, 2_000, CS);
    daemon.scan();
    assert_eq!(values(&daemon), BOBS);

    let before = writes(&daemon);
    ok(alice
        .send_json(json!({ "type": "set_automatic_switching", "payload": { "enabled": true } })));
    daemon.settle();
    daemon.scan();
    assert_eq!(writes(&daemon), before);
    assert_bob_wins(&daemon, &alice, &bob);
}

#[test]
fn equal_start_times_are_broken_by_the_lower_pid() {
    for (alice_pid, bob_pid, alice_wins) in [(600, 700, true), (700, 600, false)] {
        let (daemon, alice, bob) = shared_gpu();
        // Both games start between scans, at the same clock tick.
        daemon.run_steam_game(alice_pid, 1000, 5_000, DOTA);
        daemon.run_steam_game(bob_pid, 1001, 5_000, CS);
        daemon.scan();
        if alice_wins {
            assert_alice_wins(&daemon, &alice, &bob);
        } else {
            assert_bob_wins(&daemon, &alice, &bob);
        }
    }
}
