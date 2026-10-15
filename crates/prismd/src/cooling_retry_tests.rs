//! Retrying, or resuming, a suspended fan curve (ADR 0006) through daemon session requests. A
//! game's curve comes back as the game's: it never becomes the caller's remembered fan curve or
//! last applied manual cooling, and it doesn't start a manual override.

use serde_json::{Value, json};

use super::harness::{FakeGpuBackend, FakeSession, Harness, cooled_gpu, tunable_gpu_with_id};
use super::tuning_store::TUNING_STORE_FILE_NAME;

const GPU: &str = "GPU-A";
const DOTA: u32 = 570;

fn value(value: i64) -> Value {
    json!({ "kind": "value", "value": value })
}

fn ok(response: Value) -> Value {
    assert_ne!(response["type"], "error", "{response}");
    response["payload"].clone()
}

fn game_curve() -> Value {
    json!({ "arrangement": "shared", "setting": { "mode": "curve", "points": [
        { "temperature_c": 40, "speed_percent": 30 },
        { "temperature_c": 80, "speed_percent": 100 }
    ] } })
}

fn manual_cooling() -> Value {
    json!({ "arrangement": "per_fan", "fans": [
        { "fan_index": 0, "setting": { "mode": "curve", "points": [
            { "temperature_c": 30, "speed_percent": 40 },
            { "temperature_c": 70, "speed_percent": 100 }
        ] } },
        { "fan_index": 1, "setting": { "mode": "fixed", "speed_percent": 55 } }
    ] })
}

/// Alice with Dota 2 linked to `linked` on GPU-A, which has two fans. Daily is GPU-A's current
/// profile, and her manually applied cooling is `manual_cooling`, also her remembered curve.
fn alice_with_dota(linked: &str) -> (Harness, FakeSession) {
    let mut gpu = tunable_gpu_with_id(GPU);
    gpu.cooling = cooled_gpu().cooling;
    gpu.telemetry.temperature_c = Some(40);
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![gpu]));
    daemon.clock.advance(5_000);
    daemon.install_steam_app("alice", DOTA, "Dota 2");
    let alice = daemon.user(1000, "alice");
    alice.seed_profiles(json!([
        { "id": "daily", "name": "Daily", "gpu_id": GPU,
          "settings": { "power_limit_watts": value(400) } },
        { "id": "fans", "name": "Fans", "gpu_id": GPU,
          "settings": { "power_limit_watts": value(330), "cooling": game_curve() } },
        { "id": "power", "name": "Power", "gpu_id": GPU,
          "settings": { "power_limit_watts": value(330) } }
    ]));
    ok(alice
        .send_json(json!({ "type": "apply_profile_by_id", "payload": { "profile_id": "daily" } })));
    apply_cooling(&alice, manual_cooling());
    ok(alice.send_json(json!({ "type": "add_steam_game", "payload": { "app_id": DOTA } })));
    ok(
        alice.send_json(json!({ "type": "set_game_profile", "payload": {
        "game": { "kind": "steam", "app_id": DOTA }, "gpu_id": GPU, "profile_id": linked
    } })),
    );
    ok(alice
        .send_json(json!({ "type": "set_automatic_switching", "payload": { "enabled": true } })));
    daemon.settle();
    (daemon, alice)
}

/// Alice's game running with its own curve, which automatic switching applied.
fn game_with_curve() -> (Harness, FakeSession) {
    let (daemon, alice) = alice_with_dota("fans");
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    assert_eq!(cooling(&alice)["applied"], game_curve());
    assert_eq!(current_profile(&alice), "fans");
    (daemon, alice)
}

fn apply_cooling(session: &FakeSession, configuration: Value) {
    let result = ok(session.apply(GPU, json!({ "cooling": configuration })));
    assert_eq!(
        result["cooling"]["outcome"]["status"], "succeeded",
        "{result}"
    );
}

/// Makes the GPU's temperature unreadable until Prism suspends its curve, then readable again.
fn fault(daemon: &Harness, session: &FakeSession) {
    daemon.set_temperature(GPU, None);
    daemon.run_curves_for(500);
    assert_eq!(cooling(session)["suspension"]["kind"], "fault");
    daemon.set_temperature(GPU, Some(60));
}

fn retry_request(daemon: &Harness, revision: Option<&str>, fans: &[u32]) -> Value {
    json!({ "type": "retry_cooling", "payload": {
        "gpu_id": GPU,
        "expected_revision": revision.map_or_else(|| daemon.revision(GPU), str::to_owned),
        "fan_indexes": fans,
    } })
}

/// Retries the GPU's suspended curve as the session's user, as reviewed now.
fn retry(daemon: &Harness, session: &FakeSession) -> Value {
    session.send_json(retry_request(daemon, None, &[0, 1]))
}

fn retried(daemon: &Harness, session: &FakeSession) {
    let answer = ok(retry(daemon, session));
    assert_eq!(
        answer["cooling"]["outcome"]["status"], "succeeded",
        "{answer}"
    );
    assert_eq!(cooling(session)["suspension"], Value::Null);
}

fn cooling(session: &FakeSession) -> Value {
    session.performance_json()["gpus"][0]["cooling"]["control"].clone()
}

fn remembered(session: &FakeSession) -> Value {
    cooling(session)["remembered_curve"].clone()
}

fn last_applied(daemon: &Harness) -> Value {
    let document: Value =
        serde_json::from_slice(&daemon.persistent_file(TUNING_STORE_FILE_NAME).unwrap()).unwrap();
    document["last_applied"][GPU].clone()
}

fn current_profile(session: &FakeSession) -> Value {
    ok(session.send_json(json!({ "type": "get_tuning_states" })))[0]["current_profile"]["id"]
        .clone()
}

fn switching_state(session: &FakeSession) -> Value {
    let status = ok(session.send_json(json!({ "type": "get_automatic_switching_status" })));
    status["gpus"][0]["state"]["status"].clone()
}

fn power(daemon: &Harness) -> Option<u32> {
    daemon.gpus.gpu(GPU).power.current_limit_watts
}

#[test]
fn retrying_a_games_curve_brings_it_back_as_the_games_and_the_games_end_restores_everything() {
    let (daemon, alice) = game_with_curve();
    let manual = last_applied(&daemon);
    assert_eq!(manual["cooling"], manual_cooling());
    fault(&daemon, &alice);

    retried(&daemon, &alice);

    assert_eq!(cooling(&alice)["applied"], game_curve());
    assert_eq!(remembered(&alice), manual_cooling());
    assert_eq!(last_applied(&daemon), manual);
    assert_eq!(current_profile(&alice), "fans");
    assert_eq!(switching_state(&alice), "active");

    daemon.end_process(123);
    daemon.scan();
    assert_eq!(cooling(&alice)["applied"], manual_cooling());
    assert_eq!(power(&daemon), Some(400));
    assert_eq!(current_profile(&alice), "daily");
    assert_eq!(remembered(&alice), manual_cooling());
    assert_eq!(last_applied(&daemon), manual);
}

#[test]
fn resuming_a_games_curve_after_external_control_follows_the_same_rules() {
    let (daemon, alice) = game_with_curve();
    let manual = last_applied(&daemon);
    daemon.gpus.update(GPU, |gpu| {
        gpu.cooling.fans[0].mode = Some(prism_gpu::FanControlMode::Auto)
    });
    daemon.run_curves_for(500);
    assert_eq!(cooling(&alice)["suspension"]["kind"], "external_control");

    retried(&daemon, &alice);

    assert_eq!(remembered(&alice), manual_cooling());
    assert_eq!(last_applied(&daemon), manual);
    assert_eq!(switching_state(&alice), "active");
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(cooling(&alice)["applied"], manual_cooling());
    assert_eq!(current_profile(&alice), "daily");
}

#[test]
fn retrying_a_manual_curve_is_the_callers_manual_choice_and_overrides_a_running_game() {
    let (daemon, alice) = alice_with_dota("power");
    let bob = daemon.user(1001, "bob");
    apply_cooling(&bob, game_curve());
    assert_eq!(remembered(&alice), manual_cooling());
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    assert_eq!(power(&daemon), Some(330));
    fault(&daemon, &alice);

    retried(&daemon, &alice);

    assert_eq!(remembered(&alice), game_curve());
    assert_eq!(last_applied(&daemon)["cooling"], game_curve());
    assert_eq!(switching_state(&alice), "manual_override");
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(power(&daemon), Some(330), "the override keeps the values");
}

#[test]
fn retrying_after_the_curves_game_ended_counts_as_manual() {
    let (daemon, alice) = game_with_curve();
    fault(&daemon, &alice);
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(cooling(&alice)["suspension"]["kind"], "fault");

    retried(&daemon, &alice);

    assert_eq!(cooling(&alice)["applied"], game_curve());
    assert_eq!(remembered(&alice), game_curve());
    assert_eq!(last_applied(&daemon)["cooling"], game_curve());
}

#[test]
fn a_stale_unreviewed_or_unneeded_retry_is_refused_and_writes_nothing() {
    let (daemon, alice) = game_with_curve();
    let before = daemon.gpus.writes();
    let response = retry(&daemon, &alice);
    assert_eq!(
        response["type"], "error",
        "nothing is suspended: {response}"
    );
    assert_eq!(daemon.gpus.writes(), before);

    fault(&daemon, &alice);
    let suspended = cooling(&alice);
    let before = daemon.gpus.writes();
    for (request, code) in [
        (
            retry_request(&daemon, Some("stale"), &[0, 1]),
            "stale_revision",
        ),
        (retry_request(&daemon, None, &[0]), "invalid_argument"),
        (retry_request(&daemon, None, &[0, 1, 2]), "invalid_argument"),
    ] {
        let response = alice.send_json(request.clone());
        assert_eq!(response["type"], "error", "{request}: {response}");
        assert_eq!(response["payload"]["code"], code, "{request}: {response}");
    }
    assert_eq!(daemon.gpus.writes(), before);
    assert_eq!(cooling(&alice), suspended);
    assert_eq!(switching_state(&alice), "active");
}

#[test]
fn a_retry_needs_a_caller_allowed_to_tune() {
    let (daemon, alice) = game_with_curve();
    fault(&daemon, &alice);
    let before = daemon.gpus.writes();
    let viewer = daemon.read_only_user(1002, "carol");
    for response in [
        viewer.send_json(retry_request(&daemon, None, &[0, 1])),
        daemon.send_json(retry_request(&daemon, None, &[0, 1])),
    ] {
        assert_eq!(response["type"], "error", "{response}");
        assert_eq!(response["payload"]["code"], "no_permission", "{response}");
    }
    assert_eq!(daemon.gpus.writes(), before);
    assert_eq!(cooling(&alice)["suspension"]["kind"], "fault");
}
