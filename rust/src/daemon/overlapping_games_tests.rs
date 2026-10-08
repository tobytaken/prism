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
fn example_gpu() -> Harness {
    let mut gpu = tunable_gpu_with_id(GPU);
    gpu.power.min_limit_watts = Some(100);
    gpu.clocks.memory.min_offset_mhz = Some(-1000);
    gpu.clocks.memory.max_offset_mhz = Some(1000);
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![gpu]));
    daemon.clock.advance(5_000);
    daemon
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
    set_switching(daemon, &session, true);
    session
}

fn set_switching(daemon: &Harness, session: &FakeSession, enabled: bool) {
    ok(session.send_json(
        json!({ "type": "set_automatic_switching", "payload": { "enabled": enabled } }),
    ));
    daemon.settle();
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
    // These scenarios assert winner selection; detailed operation outcomes have their own tests.
    json!(response["payload"]["gpus"]
        .as_array()
        .unwrap()
        .iter()
        .map(|gpu| json!({ "gpuId": gpu["gpuId"], "state": gpu["state"] }))
        .collect::<Vec<_>>())
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
fn overlapping_games_keep_the_original_pre_game_values_and_never_write_graphics() {
    // Quiet (200 W, +0 memory) → A (300 W) → B (320 W, +500 memory) → B ends → A ends.
    let daemon = example_gpu();
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
    assert_eq!(
        gpu_status(&alice),
        json!([{ "gpuId": GPU, "state": { "status": "restored" } }])
    );

    assert!(!writes(&daemon).contains(&Setter::GraphicsOffset));
    assert_eq!(last_applied(&daemon), manual);
}

/// Alice with Dota 2 linked to her Gaming profile, and Bob with Counter-Strike linked to his
/// Competitive profile, both on GPU-A and both opted in. Alice's Daily is current.
fn shared_gpu() -> (Harness, FakeSession, FakeSession) {
    let daemon = example_gpu();
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
const GAMING: (Option<u32>, Option<i32>, Option<i32>) = (Some(350), Some(150), Some(0));
const COMPETITIVE: (Option<u32>, Option<i32>, Option<i32>) = (Some(400), Some(120), Some(100));

fn assert_alice_wins(daemon: &Harness, alice: &FakeSession, bob: &FakeSession) {
    assert_eq!(values(daemon), GAMING);
    assert_eq!(current_profile(alice), "Gaming");
    assert_eq!(
        gpu_status(alice),
        active(DOTA, "Dota 2", "gaming", "Gaming")
    );
    assert_eq!(gpu_status(bob), another_user());
}

fn assert_bob_wins(daemon: &Harness, alice: &FakeSession, bob: &FakeSession) {
    assert_eq!(values(daemon), COMPETITIVE);
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
    assert_eq!(
        gpu_status(alice),
        json!([{ "gpuId": GPU, "state": { "status": "restored" } }])
    );
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

/// Restarts the daemon, runs its first scan and reconnects both users.
fn restart(daemon: &mut Harness) -> (FakeSession, FakeSession) {
    daemon.restart();
    daemon.clock.advance(5_000);
    daemon.settle();
    daemon.scan();
    (daemon.user(1000, "alice"), daemon.user(1001, "bob"))
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
    let (alice, bob) = restart(&mut daemon);
    assert_bob_wins(&daemon, &alice, &bob);
    drop((alice, bob));

    // Another restart with both games still running writes nothing.
    let before = writes(&daemon);
    let (alice, bob) = restart(&mut daemon);
    assert_eq!(writes(&daemon), before);
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
fn enabling_switching_while_an_older_game_runs_keeps_the_newer_games_precedence() {
    let (daemon, alice, bob) = shared_gpu();
    set_switching(&daemon, &alice, false);
    daemon.run_steam_game(100, 1000, 1_000, DOTA);
    daemon.run_steam_game(200, 1001, 2_000, CS);
    daemon.scan();
    assert_eq!(values(&daemon), COMPETITIVE);

    let before = writes(&daemon);
    set_switching(&daemon, &alice, true);
    daemon.scan();
    assert_eq!(writes(&daemon), before);
    assert_bob_wins(&daemon, &alice, &bob);
}

#[test]
fn equal_start_times_are_broken_by_the_lower_pid() {
    for (alice_pid, bob_pid, alice_wins) in [(600, 700, true), (700, 600, false)] {
        let (mut daemon, alice, bob) = shared_gpu();
        // Both games start between scans, at the same clock tick.
        daemon.run_steam_game(alice_pid, 1000, 5_000, DOTA);
        daemon.run_steam_game(bob_pid, 1001, 5_000, CS);
        daemon.scan();
        if alice_wins {
            assert_alice_wins(&daemon, &alice, &bob);
        } else {
            assert_bob_wins(&daemon, &alice, &bob);
        }

        // A later helper with a lower pid does not replace the game's earliest process.
        daemon.run_steam_game(100, 1000, 6_000, DOTA);
        daemon.run_steam_game(200, 1001, 6_000, CS);
        let before = writes(&daemon);
        daemon.scan();
        assert_eq!(writes(&daemon), before);
        drop((alice, bob));
        let (alice, bob) = restart(&mut daemon);
        assert_eq!(writes(&daemon), before);
        if alice_wins {
            assert_alice_wins(&daemon, &alice, &bob);
        } else {
            assert_bob_wins(&daemon, &alice, &bob);
        }
    }
}

#[test]
fn a_renamed_accounts_orphaned_links_never_drive_its_games() {
    let (daemon, _alice, _bob) = shared_gpu();
    // uid 1000 is renamed to alicia, a different user who inherits none of alice's links.
    let alicia = opted_in_user(
        &daemon,
        1000,
        "alicia",
        json!([profile(
            "tuned",
            "Tuned",
            json!({ "powerLimitWatts": value(300) })
        )]),
        &[(DOTA, "Dota 2")],
    );
    let before = writes(&daemon);
    daemon.run_steam_game(100, 1000, 1_000, DOTA);
    daemon.scan();
    assert_eq!(writes(&daemon), before);
    assert_eq!(values(&daemon), PRE_GAME);

    link(&alicia, DOTA, "tuned");
    daemon.settle();
    assert_eq!(values(&daemon), (Some(300), Some(120), Some(0)));
    assert_eq!(current_profile(&alicia), "Tuned");
}

fn shared_curve() -> Value {
    json!({ "arrangement": "shared", "setting": { "mode": "curve", "points": [
        { "temperatureC": 40, "speedPercent": 30 },
        { "temperatureC": 80, "speedPercent": 100 }
    ] } })
}

fn separate_cooling(second_fan: Value) -> Value {
    json!({ "arrangement": "perFan", "fans": [
        { "fanIndex": 0, "setting": shared_curve()["setting"] },
        { "fanIndex": 1, "setting": second_fan }
    ] })
}

fn apply_cooling(session: &FakeSession, configuration: &Value) {
    let result = ok(
        session.send_json(json!({ "type": "apply_cooling", "payload": {
        "gpuId": GPU, "configuration": configuration
    } })),
    );
    assert_eq!(result["outcome"]["status"], "succeeded", "{result}");
}

fn cooling(session: &FakeSession) -> Value {
    ok(session.send_json(json!({ "type": "get_performance" })))["gpus"][0]["cooling"]["control"]
        .clone()
}

fn link_cooling(session: &FakeSession, app_id: u32, id: &str, configuration: Value) {
    ok(session.send_json(
        json!({ "type": "import_profiles", "payload": { "profiles": [
        profile(id, id, json!({ "cooling": configuration }))
    ] } }),
    ));
    link(session, app_id, id);
}

fn shared_cooled_gpu() -> (Harness, FakeSession, FakeSession) {
    let (daemon, alice, bob) = shared_gpu();
    daemon.gpus.update(GPU, |gpu| {
        gpu.cooling = super::harness::cooled_gpu().cooling;
        gpu.telemetry.temperature_c = Some(40);
    });
    (daemon, alice, bob)
}

#[test]
fn overlapping_users_restore_the_original_unsaved_cooling_configuration() {
    use crate::gpu::FanControlMode;

    for baseline in [
        shared_curve(),
        separate_cooling(json!({ "mode": "automatic" })),
        separate_cooling(json!({ "mode": "fixed", "speedPercent": 55 })),
    ] {
        for restart_between_games in [false, true] {
            let (mut daemon, alice, bob) = shared_cooled_gpu();
            apply_cooling(&alice, &baseline);
            let manual = last_applied(&daemon);
            let remembered = cooling(&alice)["rememberedCurve"].clone();
            let alice_cooling = json!({ "arrangement": "shared", "setting": {
                "mode": "fixed", "speedPercent": 75
            } });
            let bob_cooling = separate_cooling(json!({ "mode": "automatic" }));
            link_cooling(&alice, DOTA, "alice-fans", alice_cooling.clone());
            link_cooling(&bob, CS, "bob-fans", bob_cooling.clone());

            daemon.run_steam_game(100, 1000, 1_000, DOTA);
            daemon.scan();
            assert_eq!(cooling(&alice)["applied"], alice_cooling);
            assert_eq!(current_profile(&alice), "alice-fans");

            drop((alice, bob));
            if restart_between_games {
                daemon.stop_daemon();
            }
            daemon.run_steam_game(200, 1001, 2_000, CS);
            if restart_between_games {
                daemon.start_daemon();
                // Let the restarted daemon's edit-hold grace period finish before switching.
                daemon.clock.advance(5_000);
            }
            daemon.scan();
            let alice = daemon.user(1000, "alice");
            let bob = daemon.user(1001, "bob");
            assert_eq!(cooling(&bob)["applied"], bob_cooling);
            assert_eq!(current_profile(&bob), "bob-fans");
            assert_eq!(gpu_status(&alice), another_user());

            daemon.set_temperature(GPU, Some(80));
            daemon.run_curves_for(500);
            assert_eq!(
                daemon.gpus.gpu(GPU).cooling.fans[0].current_speed_percent,
                Some(100)
            );
            assert_eq!(last_applied(&daemon), manual);
            assert_eq!(cooling(&alice)["rememberedCurve"], remembered);

            daemon.end_process(200);
            daemon.scan();
            assert_eq!(cooling(&alice)["applied"], alice_cooling);
            assert_eq!(current_profile(&alice), "alice-fans");
            assert_eq!(gpu_status(&bob), another_user());

            daemon.set_temperature(GPU, Some(60));
            daemon.end_process(100);
            daemon.scan();
            assert_eq!(cooling(&alice)["applied"], baseline);
            let fans = daemon.gpus.gpu(GPU).cooling.fans;
            assert_eq!(fans[0].mode, Some(FanControlMode::Manual));
            assert_eq!(fans[0].current_speed_percent, Some(65));
            if baseline["arrangement"] == "shared" {
                assert_eq!(fans[1].mode, Some(FanControlMode::Manual));
                assert_eq!(fans[1].current_speed_percent, Some(65));
            } else if baseline["fans"][1]["setting"]["mode"] == "automatic" {
                assert_eq!(fans[1].mode, Some(FanControlMode::Auto));
            } else {
                assert_eq!(fans[1].mode, Some(FanControlMode::Manual));
                assert_eq!(fans[1].current_speed_percent, Some(55));
            }
            assert_eq!(current_profile(&alice), "Daily");
            assert_eq!(
                gpu_status(&alice),
                json!([{ "gpuId": GPU, "state": { "status": "restored" } }])
            );
            assert_eq!(last_applied(&daemon), manual);
            assert_eq!(cooling(&alice)["rememberedCurve"], remembered);
            assert!(!writes(&daemon).contains(&Setter::GraphicsOffset));
            assert!(!writes(&daemon).contains(&Setter::MemoryOffset));
        }
    }
}

#[test]
fn overlapping_games_capture_cooling_only_when_first_written_and_restore_it_when_omitted() {
    let (daemon, alice, bob) = shared_cooled_gpu();
    let baseline = separate_cooling(json!({ "mode": "fixed", "speedPercent": 55 }));
    apply_cooling(&alice, &baseline);
    let manual = last_applied(&daemon);
    let bob_cooling = json!({ "arrangement": "shared", "setting": { "mode": "automatic" } });
    link_cooling(&bob, CS, "bob-fans", bob_cooling.clone());
    let before = cooling(&alice);

    // Alice's numeric profile must apply even when cooling's policy can't currently be read.
    daemon
        .gpus
        .update(GPU, |gpu| gpu.cooling.fans[1].mode = None);
    daemon.run_steam_game(100, 1000, 1_000, DOTA);
    daemon.scan();
    assert_eq!(values(&daemon), GAMING);
    assert_eq!(current_profile(&alice), "Gaming");
    assert_eq!(cooling(&alice), before);
    daemon.gpus.update(GPU, |gpu| {
        gpu.cooling.fans[1].mode = Some(crate::gpu::FanControlMode::Manual);
    });
    daemon.set_temperature(GPU, Some(60));
    daemon.run_curves_for(500);
    assert_eq!(
        daemon.gpus.gpu(GPU).cooling.fans[0].current_speed_percent,
        Some(65)
    );

    // Cooling is readable by the time Bob's profile first writes it. Capture its policy,
    // including the curve definition, rather than the curve's current output of 65%.
    daemon.run_steam_game(200, 1001, 2_000, CS);
    daemon.scan();
    assert_eq!(cooling(&bob)["applied"], bob_cooling);
    assert_eq!(current_profile(&bob), "bob-fans");

    daemon.set_temperature(GPU, Some(80));
    daemon.end_process(200);
    daemon.scan();
    assert_eq!(cooling(&alice)["applied"], baseline);
    assert_eq!(current_profile(&alice), "Gaming");
    let fans = daemon.gpus.gpu(GPU).cooling.fans;
    assert_eq!(fans[0].current_speed_percent, Some(100));
    assert_eq!(fans[1].current_speed_percent, Some(55));

    // Bob can start again during the same activation without losing the original curve.
    daemon.run_steam_game(201, 1001, 3_000, CS);
    daemon.scan();
    assert_eq!(cooling(&bob)["applied"], bob_cooling);
    daemon.set_temperature(GPU, Some(60));
    daemon.end_process(201);
    daemon.scan();
    assert_eq!(cooling(&alice)["applied"], baseline);
    assert_eq!(
        daemon.gpus.gpu(GPU).cooling.fans[0].current_speed_percent,
        Some(65)
    );

    let restored = cooling(&alice);
    let before = writes(&daemon);
    daemon.end_process(100);
    daemon.scan();
    assert_eq!(cooling(&alice), restored, "cooling was already restored");
    assert!(writes(&daemon)[before.len()..]
        .iter()
        .all(|setter| { !matches!(setter, Setter::FanSpeed | Setter::FanModeAuto) }));
    assert_restored(&daemon, &alice, &manual);
}
