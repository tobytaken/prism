//! Seam 1: automatic switching while one linked game runs on a GPU, driven by fixture processes
//! and complete scans, through daemon session requests and pushed events.

use serde_json::{json, Value};

use super::harness::{tunable_gpu_with_id, FakeGpuBackend, FakeSession, Harness, Setter};
use super::tuning_store::TUNING_STORE_FILE_NAME;

const LINKED: &str = "GPU-A";
const UNLINKED: &str = "GPU-B";
const DOTA: u32 = 570;

fn pause(daemon: &Harness, session: &FakeSession, paused: bool) {
    ok(
        session.send_json(json!({ "type": "set_gpu_switching_paused", "payload": {
        "gpuId": LINKED, "paused": paused
    } })),
    );
    daemon.settle();
}

#[test]
fn resuming_after_gpu_return_keeps_the_cooling_baseline_when_driver_fan_policy_was_reset() {
    let (daemon, alice) = cooled_dota();
    apply_cooling(&alice, separate_cooling());
    ok(alice.send_json(json!({ "type": "import_profiles", "payload": { "profiles": [
        { "id": "fixed", "name": "Fixed", "gpuId": LINKED, "settings": {
            "powerLimitWatts": value(350),
            "cooling": { "arrangement": "shared", "setting": { "mode": "fixed", "speedPercent": 50 } }
        } }
    ] } })));
    link(&alice, LINKED, Some("fixed"));
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    pause(&daemon, &alice, true);
    let mut returning = daemon.gpus.gpu(LINKED);
    for fan in &mut returning.cooling.fans {
        fan.mode = Some(crate::gpu::FanControlMode::Auto);
    }
    daemon.gpus.depart(LINKED);
    daemon.scan();
    daemon.gpus.arrive(returning);
    daemon.scan();
    pause(&daemon, &alice, false);
    assert_eq!(cooling(&alice)["applied"]["setting"]["mode"], "fixed");
    assert_eq!(cooling(&alice)["suspension"], Value::Null);
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(values(&daemon), PRE_GAME);
    assert_eq!(cooling(&alice)["applied"], separate_cooling());
}

#[test]
fn a_paused_gpu_that_disappears_preserves_its_baseline_when_resumed_after_return() {
    let (daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    pause(&daemon, &alice, true);
    daemon.gpus.depart(LINKED);
    daemon.scan();
    daemon.gpus.arrive(tunable_gpu_with_id(LINKED));
    daemon.scan();
    pause(&daemon, &alice, false);
    assert_eq!(values(&daemon), GAMING);
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(values(&daemon), PRE_GAME);
}

#[test]
fn pause_is_shared_across_sessions_and_survives_daemon_restart_and_reboot() {
    let (mut daemon, alice) = linked_dota();
    link(&alice, UNLINKED, Some("quiet"));
    set_switching(&daemon, &alice, true);
    pause(&daemon, &alice, true);
    drop(alice);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    assert_eq!(values(&daemon), PRE_GAME);
    assert_eq!(
        daemon.gpus.gpu(UNLINKED).power.current_limit_watts,
        Some(300)
    );
    let bob = daemon.user(1001, "bob");
    assert_eq!(status(&bob)["gpus"][0]["state"]["status"], "paused");
    drop(bob);

    daemon.restart();
    daemon.clock.advance(5_000);
    daemon.scan();
    assert_eq!(values(&daemon), PRE_GAME);
    let alice = daemon.user(1000, "alice");
    assert_eq!(status(&alice)["gpus"][0]["state"]["status"], "paused");
    drop(alice);

    daemon.reboot();
    daemon.clock.advance(5_000);
    daemon.scan();
    let alice = daemon.user(1000, "alice");
    assert_eq!(values(&daemon), PRE_GAME);
    assert_eq!(status(&alice)["gpus"][0]["state"]["status"], "paused");
    pause(&daemon, &alice, false);
    assert_eq!(values(&daemon), GAMING);
    drop(alice);
    daemon.restart();
    daemon.clock.advance(5_000);
    daemon.scan();
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(values(&daemon), PRE_GAME);
}

#[test]
fn resume_without_manual_changes_preserves_the_original_baseline_even_after_restart() {
    for game_ended in [false, true] {
        let (mut daemon, alice) = cooled_dota();
        apply_cooling(&alice, separate_cooling());
        set_switching(&daemon, &alice, true);
        daemon.run_steam_game(123, 1000, 420, DOTA);
        daemon.scan();
        pause(&daemon, &alice, true);
        drop(alice);
        daemon.restart();
        daemon.clock.advance(5_000);
        daemon.scan();
        if game_ended {
            daemon.end_process(123);
            daemon.scan();
        }
        let alice = daemon.user(1000, "alice");
        assert_eq!(values(&daemon).0, Some(330));
        daemon.set_temperature(LINKED, Some(60));
        daemon.run_curves_for(500);
        pause(&daemon, &alice, false);
        if !game_ended {
            assert_eq!(values(&daemon).0, Some(330));
            daemon.end_process(123);
            daemon.scan();
        }
        assert_eq!(values(&daemon), PRE_GAME);
        assert_eq!(cooling(&alice)["applied"], separate_cooling());
        assert_eq!(
            daemon.gpus.gpu(LINKED).cooling.fans[0].current_speed_percent,
            Some(65)
        );
        assert_eq!(current_profile(&alice, LINKED)["id"], "daily");
    }
}

#[test]
fn resuming_a_manual_override_still_waits_for_edit_holds() {
    let (daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    ok(
        alice.send_json(json!({ "type": "set_power_limit", "payload": {
        "gpuId": LINKED, "limitWatts": 380
    } })),
    );
    ok(alice.send_json(json!({ "type": "set_edit_holds", "payload": { "gpuIds": [LINKED] } })));
    pause(&daemon, &alice, false);
    assert_eq!(values(&daemon).0, Some(380));
    assert_eq!(
        status(&alice)["gpus"][0]["state"]["status"],
        "waitingForEdits"
    );
    ok(alice.send_json(json!({ "type": "set_edit_holds", "payload": { "gpuIds": [] } })));
    daemon.settle();
    assert_eq!(values(&daemon), GAMING);
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(values(&daemon).0, Some(380));
}

#[test]
fn resume_captures_external_changes_even_when_the_same_game_is_still_running() {
    let (daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    pause(&daemon, &alice, true);
    daemon
        .gpus
        .update(LINKED, |gpu| gpu.power.current_limit_watts = Some(380));
    pause(&daemon, &alice, false);
    assert_eq!(values(&daemon), GAMING);
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(values(&daemon), (Some(380), Some(150), Some(0)));
}

#[test]
fn resume_applies_the_running_game_then_restores_the_manual_numeric_and_cooling_values() {
    let (daemon, alice) = cooled_dota();
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    pause(&daemon, &alice, true);
    ok(
        alice.send_json(json!({ "type": "set_power_limit", "payload": {
        "gpuId": LINKED, "limitWatts": 380
    } })),
    );
    apply_cooling(&alice, separate_cooling());
    pause(&daemon, &alice, false);
    assert_eq!(values(&daemon).0, Some(330));
    assert_eq!(cooling(&alice)["applied"], shared_curve());
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(values(&daemon).0, Some(380));
    assert_eq!(cooling(&alice)["applied"], separate_cooling());
}

#[test]
fn pausing_keeps_tuning_through_game_starts_and_ends_while_the_applied_curve_keeps_running() {
    let (daemon, alice) = cooled_dota();
    apply_cooling(&alice, separate_cooling());
    set_switching(&daemon, &alice, true);
    pause(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    assert_eq!(values(&daemon), PRE_GAME);
    assert_eq!(cooling(&alice)["applied"], separate_cooling());
    assert_eq!(
        status(&alice)["gpus"],
        json!([
            { "gpuId": LINKED, "state": { "status": "paused" } }
        ])
    );

    daemon.set_temperature(LINKED, Some(80));
    daemon.run_curves_for(500);
    assert_eq!(
        daemon.gpus.gpu(LINKED).cooling.fans[0].current_speed_percent,
        Some(100)
    );
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(values(&daemon), PRE_GAME);
    assert_eq!(cooling(&alice)["applied"], separate_cooling());

    pause(&daemon, &alice, false);
    daemon.run_steam_game(124, 1000, 500, DOTA);
    daemon.scan();
    assert_eq!(cooling(&alice)["applied"], shared_curve());
    pause(&daemon, &alice, true);
    daemon.end_process(124);
    daemon.scan();
    assert_eq!(values(&daemon).0, Some(330));
    assert_eq!(cooling(&alice)["applied"], shared_curve());
}

fn value(value: i64) -> Value {
    json!({ "kind": "value", "value": value })
}

fn ok(response: Value) -> Value {
    assert_ne!(response["type"], "error", "{response}");
    response["payload"].clone()
}

/// Alice with Dota 2 linked to Gaming on GPU-A and nothing linked on GPU-B, which has a
/// profile of hers. Daily is GPU-A's current profile, with an unsaved power tweak on top.
fn linked_dota() -> (Harness, FakeSession) {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![
        tunable_gpu_with_id(LINKED),
        tunable_gpu_with_id(UNLINKED),
    ]));
    daemon.clock.advance(5_000);
    daemon.install_steam_app("alice", DOTA, "Dota 2");
    let alice = daemon.user(1000, "alice");
    ok(alice.send_json(
        json!({ "type": "import_profiles", "payload": { "profiles": [
        { "id": "daily", "name": "Daily", "gpuId": LINKED,
          "settings": { "powerLimitWatts": value(400), "graphicsOffsetMHz": value(60) } },
        { "id": "gaming", "name": "Gaming", "gpuId": LINKED,
          "settings": { "powerLimitWatts": value(350), "graphicsOffsetMHz": value(150) } },
        { "id": "cool", "name": "Cool", "gpuId": LINKED,
          "settings": { "powerLimitWatts": value(320) } },
        { "id": "quiet", "name": "Quiet", "gpuId": UNLINKED,
          "settings": { "powerLimitWatts": value(300) } }
    ] } }),
    ));
    ok(alice
        .send_json(json!({ "type": "apply_profile_by_id", "payload": { "profileId": "daily" } })));
    ok(alice.send_json(json!({ "type": "set_power_limit",
        "payload": { "gpuId": LINKED, "limitWatts": 450 } })));
    ok(alice.send_json(json!({ "type": "add_steam_game", "payload": { "appId": DOTA } })));
    link(&alice, LINKED, Some("gaming"));
    (daemon, alice)
}

fn link(session: &FakeSession, gpu_id: &str, profile_id: Option<&str>) {
    ok(
        session.send_json(json!({ "type": "set_game_profile", "payload": {
        "game": { "kind": "steam", "appId": DOTA }, "gpuId": gpu_id, "profileId": profile_id
    } })),
    );
}

fn set_switching(daemon: &Harness, session: &FakeSession, enabled: bool) -> Value {
    let response = session
        .send_json(json!({ "type": "set_automatic_switching", "payload": { "enabled": enabled } }));
    daemon.settle();
    response
}

/// GPU-A's power limit and graphics and memory clock offsets as the hardware reports them.
fn values(daemon: &Harness) -> (Option<u32>, Option<i32>, Option<i32>) {
    let gpu = daemon.gpus.gpu(LINKED);
    (
        gpu.power.current_limit_watts,
        gpu.clocks.graphics.current_offset_mhz,
        gpu.clocks.memory.current_offset_mhz,
    )
}

const PRE_GAME: (Option<u32>, Option<i32>, Option<i32>) = (Some(450), Some(60), Some(0));
const GAMING: (Option<u32>, Option<i32>, Option<i32>) = (Some(350), Some(150), Some(0));

fn current_profile(session: &FakeSession, gpu_id: &str) -> Value {
    ok(session.send_json(json!({ "type": "get_tuning_states" })))
        .as_array()
        .unwrap()
        .iter()
        .find(|state| state["gpuId"] == gpu_id)
        .expect("the GPU has a tuning state")["currentProfile"]
        .clone()
}

fn last_applied(daemon: &Harness) -> Value {
    let document: Value =
        serde_json::from_slice(&daemon.persistent_file(TUNING_STORE_FILE_NAME).unwrap()).unwrap();
    document["lastApplied"].clone()
}

fn status(session: &FakeSession) -> Value {
    let response = session.send_json(json!({ "type": "get_automatic_switching_status" }));
    assert_eq!(response["type"], "automatic_switching_status", "{response}");
    response["payload"].clone()
}

fn active_on_linked() -> Value {
    json!({ "gpuId": LINKED, "state": {
        "status": "active",
        "game": { "kind": "steam", "appId": DOTA },
        "gameName": "Dota 2",
        "profile": { "id": "gaming", "name": "Gaming" }
    } })
}

fn writes_to(daemon: &Harness, gpu_id: &str) -> Vec<Setter> {
    daemon
        .gpus
        .writes()
        .into_iter()
        .filter(|(id, _)| id == gpu_id)
        .map(|(_, setter)| setter)
        .collect()
}

#[test]
fn a_linked_game_applies_its_profile_to_linked_gpus_only_and_its_end_restores_the_exact_pre_game_values(
) {
    let (daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    let manual = last_applied(&daemon);
    let unlinked_writes = writes_to(&daemon, UNLINKED);
    assert_eq!(values(&daemon), PRE_GAME);

    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    assert_eq!(values(&daemon), GAMING);
    assert_eq!(
        current_profile(&alice, LINKED),
        json!({ "id": "gaming", "name": "Gaming" })
    );
    assert_eq!(status(&alice)["gpus"], json!([active_on_linked()]));

    // A later scan of the same running game writes nothing more.
    let during = writes_to(&daemon, LINKED);
    daemon.scan();
    assert_eq!(writes_to(&daemon, LINKED), during);

    daemon.end_process(123);
    daemon.scan();
    // The unsaved power tweak returns, rather than Daily's saved 400 W.
    assert_eq!(values(&daemon), PRE_GAME);
    assert_eq!(
        current_profile(&alice, LINKED),
        json!({ "id": "daily", "name": "Daily" })
    );
    assert!(!writes_to(&daemon, LINKED).contains(&Setter::MemoryOffset));
    assert_eq!(writes_to(&daemon, UNLINKED), unlinked_writes);
    assert_eq!(last_applied(&daemon), manual);
    assert_eq!(
        status(&alice),
        json!({ "condition": { "status": "ready" }, "gpus": [] })
    );
}

#[test]
fn a_crashed_game_whose_processes_are_all_gone_restores_like_a_normal_exit() {
    let (daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    daemon.run_process(200, 1000, 900, "SteamAppId=570\0", "wine64\0dota2.exe\0");
    daemon.run_process(201, 1000, 905, "SteamAppId=570\0", "wineserver\0");
    daemon.scan();
    assert_eq!(values(&daemon), GAMING);

    daemon.end_process(200);
    daemon.scan();
    assert_eq!(values(&daemon), GAMING, "a helper keeps the game running");

    daemon.end_process(201);
    daemon.scan();
    assert_eq!(values(&daemon), PRE_GAME);
}

#[test]
fn enabling_switching_mid_game_applies_and_disabling_restores() {
    let (daemon, alice) = linked_dota();
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    assert_eq!(values(&daemon), PRE_GAME);
    assert_eq!(
        status(&alice),
        json!({ "condition": { "status": "off" }, "gpus": [] })
    );

    set_switching(&daemon, &alice, true);
    assert_eq!(values(&daemon), GAMING);

    set_switching(&daemon, &alice, false);
    assert_eq!(values(&daemon), PRE_GAME);
    assert_eq!(
        current_profile(&alice, LINKED),
        json!({ "id": "daily", "name": "Daily" })
    );
}

#[test]
fn association_changes_and_profile_deletes_re_evaluate_a_running_game() {
    let (daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    assert_eq!(values(&daemon), GAMING);

    // Cool omits the graphics offset, so it returns to its pre-game value.
    link(&alice, LINKED, Some("cool"));
    daemon.settle();
    assert_eq!(values(&daemon), (Some(320), Some(60), Some(0)));
    assert_eq!(
        current_profile(&alice, LINKED),
        json!({ "id": "cool", "name": "Cool" })
    );

    // Leave unchanged restores every setting automation wrote.
    link(&alice, LINKED, None);
    daemon.settle();
    assert_eq!(values(&daemon), PRE_GAME);

    link(&alice, LINKED, Some("gaming"));
    daemon.settle();
    assert_eq!(values(&daemon), GAMING);

    ok(alice.send_json(json!({ "type": "delete_profile", "payload": { "profileId": "gaming" } })));
    daemon.settle();
    assert_eq!(values(&daemon), PRE_GAME);
    assert_eq!(
        current_profile(&alice, LINKED),
        json!({ "id": "daily", "name": "Daily" })
    );
}

#[test]
fn saving_a_linked_profile_mid_activation_takes_effect_at_its_next_application() {
    let (daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    assert_eq!(values(&daemon), GAMING);

    // Gaming is current, so its owner can save the GPU's values over it.
    ok(alice.send_json(json!({ "type": "set_power_limit",
        "payload": { "gpuId": LINKED, "limitWatts": 380 } })));
    let revision =
        ok(alice.send_json(json!({ "type": "get_tuning_states" })))[0]["revision"].clone();
    ok(alice.send_json(json!({ "type": "save_to_current_profile",
        "payload": { "gpuId": LINKED, "revision": revision } })));
    ok(alice.send_json(json!({ "type": "set_power_limit",
        "payload": { "gpuId": LINKED, "limitWatts": 340 } })));
    let before = writes_to(&daemon, LINKED);
    daemon.scan();
    assert_eq!(writes_to(&daemon, LINKED), before);
    assert_eq!(values(&daemon).0, Some(340));

    daemon.end_process(123);
    daemon.scan();
    assert_eq!(
        values(&daemon),
        (Some(340), Some(150), Some(0)),
        "manual values are kept"
    );
    daemon.run_steam_game(124, 1000, 800, DOTA);
    daemon.scan();
    assert_eq!(values(&daemon), (Some(380), Some(150), Some(0)));
}

#[test]
fn recognition_unavailable_mid_game_holds_values_shows_the_uncertainty_and_restores_nothing() {
    let (daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    alice.pushed_events();
    let applied = writes_to(&daemon, LINKED);

    // The game ends while another candidate's details can't be read.
    daemon.end_process(123);
    daemon.run_steam_game(300, 1000, 990, 620);
    std::fs::remove_file(daemon.host_path("/proc/300/stat")).unwrap();
    daemon.scan();
    assert_eq!(values(&daemon), GAMING);
    assert_eq!(writes_to(&daemon, LINKED), applied);
    let held = status(&alice);
    assert_eq!(held["condition"]["status"], "recognitionUnavailable");
    assert_eq!(held["gpus"], json!([active_on_linked()]));
    assert!(alice
        .pushed_events()
        .contains(&json!({ "type": "automaticSwitchingChanged", "payload": held })));

    daemon.end_process(300);
    daemon.scan();
    assert_eq!(values(&daemon), PRE_GAME);
    assert_eq!(status(&alice)["condition"]["status"], "ready");
}

#[test]
fn the_current_profile_swaps_to_the_game_profile_and_back_and_last_applied_values_stay_manual() {
    let (daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    let manual = last_applied(&daemon);
    assert_eq!(
        manual[LINKED],
        json!({ "powerLimitWatts": value(450), "graphicsOffsetMHz": value(60) })
    );
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    assert_eq!(current_profile(&alice, LINKED)["id"], "gaming");
    assert_eq!(last_applied(&daemon), manual);
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(current_profile(&alice, LINKED)["id"], "daily");
    assert_eq!(last_applied(&daemon), manual);
}

#[test]
fn an_unhealthy_tuning_store_blocks_every_automatic_write() {
    let (daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    std::fs::create_dir(daemon.persistent_path("tuning-state.json.tmp")).unwrap();
    assert_eq!(set_switching(&daemon, &alice, true)["type"], "error");
    let before = writes_to(&daemon, LINKED);

    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    assert_eq!(writes_to(&daemon, LINKED), before);
    assert_eq!(values(&daemon), PRE_GAME);
    assert_eq!(status(&alice)["condition"]["status"], "blocked");
}

#[test]
fn status_is_pushed_live_and_another_users_game_shows_without_its_details() {
    let (daemon, alice) = linked_dota();
    let bob = daemon.user(1001, "bob");
    set_switching(&daemon, &alice, true);
    alice.pushed_events();
    bob.pushed_events();

    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    let active = json!({ "condition": { "status": "ready" }, "gpus": [active_on_linked()] });
    assert_eq!(status(&alice), active);
    assert!(alice
        .pushed_events()
        .contains(&json!({ "type": "automaticSwitchingChanged", "payload": active })));
    let other = json!({ "condition": { "status": "off" },
        "gpus": [{ "gpuId": LINKED, "state": { "status": "anotherUser" } }] });
    assert_eq!(status(&bob), other);
    assert!(bob
        .pushed_events()
        .contains(&json!({ "type": "automaticSwitchingChanged", "payload": other })));

    // An unchanged status isn't pushed again.
    daemon.scan();
    assert!(!alice
        .pushed_events()
        .iter()
        .any(|event| event["type"] == "automaticSwitchingChanged"));
}

#[test]
fn a_daemon_restart_mid_game_keeps_the_pre_game_values_for_restoration() {
    let (mut daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    drop(alice);

    let before = writes_to(&daemon, LINKED);
    daemon.restart();
    daemon.clock.advance(5_000);
    daemon.settle();
    daemon.scan();
    assert_eq!(writes_to(&daemon, LINKED), before);
    let alice = daemon.user(1000, "alice");
    assert_eq!(status(&alice)["gpus"], json!([active_on_linked()]));

    daemon.end_process(123);
    daemon.scan();
    assert_eq!(values(&daemon), PRE_GAME);
    assert_eq!(current_profile(&alice, LINKED)["id"], "daily");
}

#[test]
fn a_previous_boots_pre_game_values_are_never_restored() {
    let (mut daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    drop(alice);

    let before = writes_to(&daemon, LINKED);

    // The game ended with the host, so the new boot's first scan finds nothing running.
    daemon.end_process(123);
    daemon.reboot_with_retained_boot_state();
    daemon.settle();
    daemon.scan();
    assert_eq!(writes_to(&daemon, LINKED), before);
    assert_eq!(values(&daemon), GAMING);
}

#[test]
fn a_partial_restoration_leaves_no_current_profile_and_reports_the_failure() {
    let (daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    daemon.gpus.fail(
        LINKED,
        Setter::GraphicsOffset,
        "Graphics offset write failed.",
    );

    daemon.end_process(123);
    daemon.scan();
    assert_eq!(values(&daemon), (Some(450), Some(150), Some(0)));
    assert_eq!(current_profile(&alice, LINKED), Value::Null);
    assert_eq!(
        status(&alice)["gpus"],
        json!([{ "gpuId": LINKED, "state": {
            "status": "failed", "message": "Graphics offset write failed."
        } }])
    );
    // A failed restoration isn't retried by later scans.
    let before = writes_to(&daemon, LINKED);
    daemon.scan();
    assert_eq!(writes_to(&daemon, LINKED), before);
}

fn shared_curve() -> Value {
    json!({ "arrangement": "shared", "setting": { "mode": "curve", "points": [
        { "temperatureC": 40, "speedPercent": 30 },
        { "temperatureC": 80, "speedPercent": 100 }
    ] } })
}

fn separate_cooling() -> Value {
    json!({ "arrangement": "perFan", "fans": [
        { "fanIndex": 0, "setting": shared_curve()["setting"] },
        { "fanIndex": 1, "setting": { "mode": "fixed", "speedPercent": 55 } }
    ] })
}

fn apply_cooling(session: &FakeSession, configuration: Value) {
    let result = ok(
        session.send_json(json!({ "type": "apply_cooling", "payload": {
        "gpuId": LINKED, "configuration": configuration
    } })),
    );
    assert_eq!(result["outcome"]["status"], "succeeded", "{result}");
}

fn cooling(session: &FakeSession) -> Value {
    ok(session.send_json(json!({ "type": "get_performance" })))["gpus"]
        .as_array()
        .unwrap()
        .iter()
        .find(|gpu| gpu["id"] == LINKED)
        .unwrap()["cooling"]["control"]
        .clone()
}

fn cooled_dota() -> (Harness, FakeSession) {
    let (daemon, alice) = linked_dota();
    daemon.gpus.update(LINKED, |gpu| {
        gpu.cooling = super::harness::cooled_gpu().cooling;
        gpu.telemetry.temperature_c = Some(40);
    });
    ok(alice.send_json(
        json!({ "type": "import_profiles", "payload": { "profiles": [
        { "id": "fans", "name": "Fans", "gpuId": LINKED,
          "settings": { "powerLimitWatts": value(330), "cooling": shared_curve() } }
    ] } }),
    ));
    link(&alice, LINKED, Some("fans"));
    (daemon, alice)
}

#[test]
fn a_game_restores_an_unsaved_curve_and_fan_arrangement_at_the_new_temperature() {
    let (daemon, alice) = cooled_dota();
    daemon.set_temperature(LINKED, Some(40));
    apply_cooling(&alice, separate_cooling());
    let manual = last_applied(&daemon);
    let remembered = cooling(&alice)["rememberedCurve"].clone();
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    assert_eq!(cooling(&alice)["applied"], shared_curve());
    assert_eq!(current_profile(&alice, LINKED)["id"], "fans");
    assert_eq!(last_applied(&daemon), manual);

    daemon.set_temperature(LINKED, Some(80));
    daemon.run_curves_for(500);
    assert_eq!(
        daemon.gpus.gpu(LINKED).cooling.fans[1].current_speed_percent,
        Some(100)
    );
    assert_eq!(last_applied(&daemon), manual);
    daemon.set_temperature(LINKED, Some(60));
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(cooling(&alice)["applied"], separate_cooling());
    let fans = daemon.gpus.gpu(LINKED).cooling.fans;
    assert_eq!(fans[0].current_speed_percent, Some(65));
    assert_eq!(fans[1].current_speed_percent, Some(55));
    assert_eq!(last_applied(&daemon), manual);
    assert_eq!(cooling(&alice)["rememberedCurve"], remembered);
    assert_eq!(current_profile(&alice, LINKED)["id"], "daily");
}

#[test]
fn relinking_to_a_profile_without_cooling_returns_the_pre_game_configuration_immediately() {
    let (daemon, alice) = cooled_dota();
    apply_cooling(&alice, separate_cooling());
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    assert_eq!(cooling(&alice)["applied"], shared_curve());

    daemon.set_temperature(LINKED, Some(60));
    link(&alice, LINKED, Some("cool"));
    daemon.settle();
    assert_eq!(cooling(&alice)["applied"], separate_cooling());
    assert_eq!(
        daemon.gpus.gpu(LINKED).cooling.fans[0].current_speed_percent,
        Some(65)
    );
    assert_eq!(current_profile(&alice, LINKED)["id"], "cool");
    let revision = cooling(&alice)["revision"].clone();
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(
        cooling(&alice)["revision"],
        revision,
        "cooling was already restored"
    );
}

#[test]
fn cooling_is_untouched_when_omitted_and_captured_only_when_automation_first_writes_it() {
    let (daemon, alice) = cooled_dota();
    apply_cooling(&alice, shared_curve());
    link(&alice, LINKED, Some("gaming"));
    set_switching(&daemon, &alice, true);
    let before = cooling(&alice);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    assert_eq!(cooling(&alice), before);
    daemon.set_temperature(LINKED, Some(60));
    daemon.run_curves_for(500);
    assert_eq!(
        daemon.gpus.gpu(LINKED).cooling.fans[0].current_speed_percent,
        Some(65)
    );
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(cooling(&alice)["revision"], before["revision"]);
    assert_eq!(cooling(&alice)["applied"], shared_curve());

    // No automatic cooling write yet: this unsaved manual arrangement is the next baseline.
    apply_cooling(&alice, separate_cooling());
    daemon.run_steam_game(124, 1000, 800, DOTA);
    daemon.scan();
    link(&alice, LINKED, Some("fans"));
    daemon.settle();
    assert_eq!(cooling(&alice)["applied"], shared_curve());
    daemon.end_process(124);
    daemon.scan();
    assert_eq!(cooling(&alice)["applied"], separate_cooling());
}

#[test]
fn version_one_switching_state_is_preserved_and_blocks_automatic_writes() {
    let (mut daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    drop(alice);
    daemon.stop_daemon();
    let path = daemon.boot_path(super::switching::SWITCHING_STATE_FILE_NAME);
    let old = serde_json::to_vec(&json!({
        "schemaVersion": 1, "bootId": "boot-1", "activations": {
            "GPU-A": {
                "driving": { "uid": 1000, "username": "alice", "game": { "kind": "steam", "appId": DOTA }, "profileId": "gaming" },
                "baseline": { "powerLimitWatts": 400 },
                "previousProfile": "daily", "failure": null
            }
        }
    })).unwrap();
    std::fs::write(&path, &old).unwrap();
    let before = daemon.gpus.writes();
    daemon.start_daemon();
    daemon.settle();
    let alice = daemon.user(1000, "alice");
    assert_eq!(status(&alice)["condition"]["status"], "blocked");
    assert_eq!(daemon.gpus.writes(), before);
    assert_eq!(std::fs::read(&path).unwrap(), old);

    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    assert_eq!(daemon.gpus.writes(), before);
    assert_eq!(std::fs::read(&path).unwrap(), old);
}

#[test]
fn invalid_cooling_blocks_the_whole_game_profile_without_clamping_or_later_restoring_it() {
    let (daemon, alice) = cooled_dota();
    apply_cooling(&alice, separate_cooling());
    daemon.gpus.update(LINKED, |gpu| {
        gpu.cooling.max_operating_temperature_c = Some(75);
    });
    set_switching(&daemon, &alice, true);
    let before = daemon.gpus.writes();
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    assert_eq!(daemon.gpus.writes(), before);
    assert_eq!(status(&alice)["gpus"][0]["state"]["status"], "failed");
    assert_eq!(current_profile(&alice, LINKED)["id"], "daily");
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(daemon.gpus.writes(), before);
}

#[test]
fn cooling_only_profiles_restore_automatic_fixed_and_shared_curve_configurations() {
    for baseline in [
        json!({ "arrangement": "shared", "setting": { "mode": "automatic" } }),
        json!({ "arrangement": "shared", "setting": { "mode": "fixed", "speedPercent": 47 } }),
        shared_curve(),
    ] {
        let (daemon, alice) = cooled_dota();
        apply_cooling(&alice, baseline.clone());
        ok(alice.send_json(
            json!({ "type": "import_profiles", "payload": { "profiles": [
            { "id": "only-fans", "name": "Only fans", "gpuId": LINKED,
              "settings": { "cooling": separate_cooling() } }
        ] } }),
        ));
        link(&alice, LINKED, Some("only-fans"));
        set_switching(&daemon, &alice, true);
        daemon.run_steam_game(123, 1000, 420, DOTA);
        daemon.scan();
        assert_eq!(cooling(&alice)["applied"], separate_cooling());
        assert_eq!(current_profile(&alice, LINKED)["id"], "only-fans");
        daemon.end_process(123);
        daemon.scan();
        assert_eq!(cooling(&alice)["applied"], baseline);
        assert_eq!(current_profile(&alice, LINKED)["id"], "daily");
    }
}

#[test]
fn a_cooling_failure_does_not_mark_a_partly_applied_game_profile_as_current() {
    let (daemon, alice) = cooled_dota();
    apply_cooling(&alice, separate_cooling());
    let manual = last_applied(&daemon);
    daemon
        .gpus
        .fail(LINKED, Setter::FanSpeed, "Fan command failed.");
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    assert_eq!(values(&daemon).0, Some(330));
    assert_eq!(current_profile(&alice, LINKED), Value::Null);
    assert_eq!(status(&alice)["gpus"][0]["state"]["status"], "failed");
    assert_eq!(last_applied(&daemon), manual);
}

#[test]
fn unreadable_pre_game_cooling_prevents_any_profile_write() {
    let (daemon, alice) = cooled_dota();
    // A manual mode reported without a known configured target is not a restorable baseline.
    daemon.gpus.update(LINKED, |gpu| {
        gpu.cooling.fans[0].mode = Some(crate::gpu::FanControlMode::Manual);
    });
    set_switching(&daemon, &alice, true);
    let before = daemon.gpus.writes();
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    assert_eq!(daemon.gpus.writes(), before);
    assert_eq!(status(&alice)["gpus"][0]["state"]["status"], "failed");
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(daemon.gpus.writes(), before);
}

#[test]
fn a_manual_attempt_overrides_only_its_gpu_and_keeps_its_values_after_the_game_ends() {
    let (daemon, alice) = linked_dota();
    link(&alice, UNLINKED, Some("quiet"));
    let other_before = daemon.gpus.gpu(UNLINKED).power.current_limit_watts;
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    ok(alice.send_json(json!({ "type": "set_power_limit",
        "payload": { "gpuId": LINKED, "limitWatts": 380 } })));
    assert_eq!(
        status(&alice)["gpus"][0]["state"],
        json!({ "status": "manualOverride", "changedOutsidePrism": false })
    );
    link(&alice, LINKED, Some("cool"));
    daemon.settle();
    assert_eq!(values(&daemon), (Some(380), Some(150), Some(0)));
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(values(&daemon), (Some(380), Some(150), Some(0)));
    assert_eq!(
        daemon.gpus.gpu(UNLINKED).power.current_limit_watts,
        other_before
    );
    // A later game captures the manual values as its new baseline.
    daemon.run_steam_game(124, 1000, 500, DOTA);
    daemon.scan();
    assert_eq!(values(&daemon).0, Some(320));
    daemon.end_process(124);
    daemon.scan();
    assert_eq!(values(&daemon), (Some(380), Some(150), Some(0)));
}

#[test]
fn a_manual_request_during_an_automatic_write_finishes_its_setters_but_drops_queued_automation() {
    let (daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    let before = writes_to(&daemon, LINKED).len();
    let gate = daemon.gpus.block(LINKED, Setter::PowerLimit);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.core().scan_running_games();
    gate.wait_until_entered();
    // Queue another automatic application while the first is inside its setter.
    link(&alice, LINKED, Some("cool"));
    let mut manual = daemon.send_in_background(
        serde_json::from_value(json!({
            "type": "set_power_limit", "payload": { "gpuId": LINKED, "limitWatts": 380 }
        }))
        .unwrap(),
    );
    assert!(manual.is_pending());
    assert_ne!(
        status(&alice)["gpus"][0]["state"]["status"],
        "manualOverride",
        "the override is recorded when the manual write starts"
    );
    gate.release();
    manual.response();
    daemon.settle();
    assert_eq!(
        &writes_to(&daemon, LINKED)[before..],
        &[
            Setter::PowerLimit,
            Setter::GraphicsOffset,
            Setter::PowerLimit
        ]
    );
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(values(&daemon), (Some(380), Some(150), Some(0)));
}

#[test]
fn an_external_change_at_game_end_preserves_every_value_and_reports_changed_outside_prism() {
    let (mut daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    daemon
        .gpus
        .update(LINKED, |gpu| gpu.power.current_limit_watts = Some(390));
    // Detection happens at automatic writes, not on every scan or read.
    daemon.scan();
    assert_eq!(status(&alice)["gpus"][0]["state"]["status"], "active");
    drop(alice);
    daemon.restart();
    daemon.clock.advance(5_000);
    let alice = daemon.user(1000, "alice");
    let before = daemon.gpus.writes();
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(daemon.gpus.writes(), before);
    assert_eq!(values(&daemon), (Some(390), Some(150), Some(0)));
    assert_eq!(
        status(&alice)["gpus"][0]["state"],
        json!({ "status": "manualOverride", "changedOutsidePrism": true })
    );
    daemon.scan();
    assert_eq!(
        status(&alice)["gpus"][0]["state"]["changedOutsidePrism"],
        true
    );
}

#[test]
fn external_cooling_yields_the_curve_and_blocks_restoration_until_explicit_manual_application() {
    let (daemon, alice) = cooled_dota();
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    // Physical speed is not an independently observable manual target.
    daemon.gpus.update(LINKED, |gpu| {
        gpu.cooling.fans[0].current_speed_percent = Some(77)
    });
    daemon.set_temperature(LINKED, Some(60));
    daemon.run_curves_for(500);
    assert_eq!(status(&alice)["gpus"][0]["state"]["status"], "active");
    daemon.gpus.update(LINKED, |gpu| {
        gpu.cooling.fans[0].mode = Some(crate::gpu::FanControlMode::Auto)
    });
    let before = daemon.gpus.writes();
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(daemon.gpus.writes(), before);
    assert_eq!(
        status(&alice)["gpus"][0]["state"]["changedOutsidePrism"],
        true
    );
    assert_eq!(cooling(&alice)["applied"], shared_curve());
    assert_eq!(cooling(&alice)["suspension"]["kind"], "externalControl");
    daemon.run_curves_for(5_000);
    assert_eq!(daemon.gpus.writes(), before);
    daemon.run_steam_game(124, 1000, 500, DOTA);
    daemon.scan();
    assert_eq!(
        daemon.gpus.writes(),
        before,
        "automation cannot resume a yielded curve"
    );
    apply_cooling(&alice, shared_curve());
    assert_eq!(cooling(&alice)["suspension"], Value::Null);
}

#[test]
fn automatic_switching_rejects_a_save_using_the_apps_old_values() {
    let (daemon, alice) = linked_dota();
    let old = ok(alice.send_json(json!({ "type": "get_tuning_states" })))[0]["revision"].clone();
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    for request in [
        json!({ "type": "save_profile_as_new", "payload": { "gpuId": LINKED, "name": "Stale", "revision": old } }),
        json!({ "type": "save_to_current_profile", "payload": { "gpuId": LINKED, "revision": old } }),
    ] {
        let response = alice.send_json(request);
        assert_eq!(response["type"], "error");
        assert_eq!(
            response["payload"]["message"],
            "The GPU's values changed since you last looked. Review them and save again."
        );
    }
    assert_eq!(values(&daemon), GAMING);
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(
        values(&daemon),
        PRE_GAME,
        "saving is not a manual tuning attempt"
    );
}

#[test]
fn manual_profile_cooling_and_failed_attempts_all_override_and_survive_a_daemon_restart() {
    for request in [
        json!({ "type": "apply_profile_by_id", "payload": { "profileId": "cool" } }),
        json!({ "type": "apply_cooling", "payload": { "gpuId": LINKED, "configuration": separate_cooling() } }),
        json!({ "type": "set_power_limit", "payload": { "gpuId": LINKED, "limitWatts": 9999 } }),
    ] {
        let (mut daemon, alice) = cooled_dota();
        set_switching(&daemon, &alice, true);
        daemon.run_steam_game(123, 1000, 420, DOTA);
        daemon.scan();
        alice.send_json(request);
        assert_eq!(
            status(&alice)["gpus"][0]["state"]["status"],
            "manualOverride"
        );
        let values_after = values(&daemon);
        let cooling_after = cooling(&alice)["applied"].clone();
        drop(alice);
        daemon.restart();
        let alice = daemon.user(1000, "alice");
        daemon.scan();
        assert_eq!(
            status(&alice)["gpus"][0]["state"]["status"],
            "manualOverride"
        );
        daemon.end_process(123);
        daemon.scan();
        assert_eq!(values(&daemon), values_after);
        assert_eq!(cooling(&alice)["applied"], cooling_after);
    }
}

#[test]
fn external_changes_are_checked_before_switching_profiles_but_only_for_controlled_settings() {
    let (daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    daemon.gpus.update(LINKED, |gpu| {
        gpu.clocks.memory.current_offset_mhz = Some(100)
    });
    link(&alice, LINKED, Some("cool"));
    daemon.settle();
    assert_eq!(values(&daemon), (Some(320), Some(60), Some(100)));
    daemon
        .gpus
        .update(LINKED, |gpu| gpu.power.current_limit_watts = Some(390));
    let before = daemon.gpus.writes();
    link(&alice, LINKED, Some("gaming"));
    daemon.settle();
    assert_eq!(daemon.gpus.writes(), before);
    assert_eq!(
        status(&alice)["gpus"][0]["state"]["changedOutsidePrism"],
        true
    );
    assert!(alice
        .pushed_events()
        .iter()
        .any(|event| event["payload"]["gpus"][0]["state"]["changedOutsidePrism"] == true));
}

#[test]
fn an_override_waits_for_all_linked_games_even_when_switching_is_disabled() {
    let (daemon, alice) = linked_dota();
    daemon.install_steam_app("alice", 730, "Counter-Strike");
    ok(alice.send_json(json!({ "type": "add_steam_game", "payload": { "appId": 730 } })));
    ok(
        alice.send_json(json!({ "type": "set_game_profile", "payload": {
        "game": { "kind": "steam", "appId": 730 }, "gpuId": LINKED, "profileId": "cool"
    } })),
    );
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.run_steam_game(124, 1000, 500, 730);
    daemon.scan();
    ok(alice.send_json(json!({ "type": "set_power_limit",
        "payload": { "gpuId": LINKED, "limitWatts": 380 } })));
    set_switching(&daemon, &alice, false);
    daemon.end_process(124);
    daemon.scan();
    assert_eq!(
        status(&alice)["gpus"][0]["state"]["status"],
        "manualOverride"
    );
    set_switching(&daemon, &alice, true);
    assert_eq!(values(&daemon).0, Some(380));
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(values(&daemon).0, Some(380));
    assert_eq!(status(&alice)["gpus"], json!([]));
}

#[test]
fn adjusting_and_applying_a_profile_reserves_manual_precedence_before_saving_the_correction() {
    let (daemon, alice) = linked_dota();
    ok(alice.send_json(
        json!({ "type": "import_profiles", "payload": { "profiles": [
        { "id": "loud", "name": "Loud", "gpuId": LINKED,
          "settings": { "powerLimitWatts": value(650) } }
    ] } }),
    ));
    set_switching(&daemon, &alice, true);
    let before = writes_to(&daemon, LINKED).len();
    let applying = daemon.user(1000, "alice");
    let gate = daemon.gpus.block(LINKED, Setter::PowerLimit);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.core().scan_running_games();
    gate.wait_until_entered();
    link(&alice, LINKED, Some("cool"));
    let (sent, response) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        sent.send(
            applying.send_json(json!({ "type": "correct_and_apply_profile", "payload": {
            "profileId": "loud", "corrections": [{ "setting": "powerLimitWatts",
                "savedValue": 650, "proposedValue": 600, "bound": "maximum" }]
        } })),
        )
        .unwrap();
    });
    assert!(matches!(
        response.recv_timeout(std::time::Duration::from_millis(200)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    gate.release();
    let result = response
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert_eq!(
        result["payload"]["application"]["power"]["status"], "succeeded",
        "{result}"
    );
    daemon.settle();
    assert_eq!(
        &writes_to(&daemon, LINKED)[before..],
        &[
            Setter::PowerLimit,
            Setter::GraphicsOffset,
            Setter::PowerLimit
        ]
    );
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(values(&daemon), (Some(600), Some(150), Some(0)));
}

#[test]
fn an_absent_gpu_keeps_its_baseline_until_the_same_uuid_returns_in_another_slot() {
    let (daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    let mut returning = daemon.gpus.gpu(LINKED);
    daemon.gpus.depart(LINKED);
    daemon.scan();
    daemon.end_process(123);
    daemon.scan();
    let before = daemon.gpus.writes();
    let replacement = tunable_gpu_with_id("GPU-replacement");
    daemon.gpus.arrive(replacement);
    daemon.scan();
    assert_eq!(daemon.gpus.writes(), before);
    assert_eq!(current_profile(&alice, "GPU-replacement"), Value::Null);

    returning.bus_id = Some("0000:08:00.0".into());
    daemon.gpus.arrive(returning);
    daemon.scan();
    assert_eq!(values(&daemon), PRE_GAME);
    assert_eq!(current_profile(&alice, LINKED)["id"], "daily");
    assert!(writes_to(&daemon, "GPU-replacement").is_empty());
}

#[test]
fn unreadable_same_boot_state_preserves_the_file_and_marks_its_gpu_as_recovery_required() {
    let (mut daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    drop(alice);
    daemon.stop_daemon();
    let path = daemon.boot_path(super::switching::SWITCHING_STATE_FILE_NAME);
    let mut document: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    document["activations"][LINKED]["baseline"] = json!("unreadable");
    let corrupt = serde_json::to_vec(&document).unwrap();
    std::fs::write(&path, &corrupt).unwrap();
    let before = daemon.gpus.writes();
    daemon.gpus.arrive(tunable_gpu_with_id("GPU-replacement"));
    daemon.start_daemon();
    daemon.scan();
    let alice = daemon.user(1000, "alice");
    let shown = status(&alice);
    assert_eq!(shown["condition"]["status"], "blocked");
    assert_eq!(shown["gpus"].as_array().unwrap().len(), 1);
    assert_eq!(shown["gpus"][0]["gpuId"], LINKED);
    assert_eq!(shown["gpus"][0]["state"]["status"], "recoveryRequired");
    assert_eq!(daemon.gpus.writes(), before);
    assert_eq!(std::fs::read(path).unwrap(), corrupt);
}

#[test]
fn a_previous_boots_unreadable_activation_does_not_block_new_game_tuning() {
    let (mut daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    drop(alice);
    daemon.stop_daemon();
    let path = daemon.boot_path(super::switching::SWITCHING_STATE_FILE_NAME);
    std::fs::write(
        &path,
        br#"{"schemaVersion":99,"bootId":"boot-1","activations":"old format"}"#,
    )
    .unwrap();
    daemon.end_process(123);
    daemon.reboot_with_retained_boot_state();
    daemon.clock.advance(5_000);
    daemon.settle();
    let alice = daemon.user(1000, "alice");
    assert_eq!(status(&alice)["condition"]["status"], "ready");
    daemon
        .gpus
        .update(LINKED, |gpu| gpu.power.current_limit_watts = Some(410));
    daemon.run_steam_game(124, 1000, 800, DOTA);
    daemon.scan();
    assert_eq!(values(&daemon), GAMING);
    daemon.end_process(124);
    daemon.scan();
    assert_eq!(values(&daemon).0, Some(410));
}

#[test]
fn a_returning_gpu_resumes_its_game_curve_then_restores_its_complete_cooling_baseline() {
    let (daemon, alice) = cooled_dota();
    apply_cooling(&alice, separate_cooling());
    let manual = last_applied(&daemon);
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    let mut returning = daemon.gpus.gpu(LINKED);
    daemon.gpus.depart(LINKED);
    daemon.run_curves_for(500);
    returning.bus_id = Some("0000:08:00.0".into());
    returning.telemetry.temperature_c = Some(80);
    daemon.gpus.arrive(returning);
    daemon.run_curves_for(500);
    assert_eq!(
        daemon.gpus.gpu(LINKED).cooling.fans[0].current_speed_percent,
        Some(100)
    );
    assert_eq!(cooling(&alice)["applied"], shared_curve());
    assert_eq!(cooling(&alice)["suspension"], Value::Null);
    daemon.set_temperature(LINKED, Some(60));
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(cooling(&alice)["applied"], separate_cooling());
    let fans = daemon.gpus.gpu(LINKED).cooling.fans;
    assert_eq!(fans[0].current_speed_percent, Some(65));
    assert_eq!(fans[1].current_speed_percent, Some(55));
    assert_eq!(last_applied(&daemon), manual);
}

#[test]
fn a_faulted_game_curve_is_not_resurrected_by_restoration_after_a_daemon_restart() {
    let (mut daemon, alice) = cooled_dota();
    apply_cooling(&alice, separate_cooling());
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    daemon.set_temperature(LINKED, None);
    daemon.run_curves_for(500);
    assert_eq!(cooling(&alice)["suspension"]["kind"], "fault");
    drop(alice);
    daemon.stop_daemon();
    daemon.end_process(123);
    daemon.set_temperature(LINKED, Some(60));
    let before = daemon.gpus.writes();
    daemon.start_daemon();
    daemon.clock.advance(5_000);
    daemon.scan();
    let alice = daemon.user(1000, "alice");
    assert_eq!(daemon.gpus.writes(), before);
    assert_eq!(cooling(&alice)["suspension"]["kind"], "fault");
    assert_eq!(status(&alice)["gpus"][0]["state"]["status"], "failed");
}

#[test]
fn a_game_ending_while_the_daemon_is_down_restores_only_after_a_complete_scan_without_rerunning_startup(
) {
    let (mut daemon, alice) = linked_dota();
    ok(
        alice.send_json(json!({ "type": "set_startup_profile", "payload": {
        "gpuId": LINKED, "profileId": "cool"
    } })),
    );
    ok(
        alice.send_json(json!({ "type": "set_startup_behavior", "payload": {
        "behavior": "startupProfiles"
    } })),
    );
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    drop(alice);
    daemon.stop_daemon();
    daemon.end_process(123);
    daemon.run_steam_game(300, 1000, 990, 620);
    std::fs::remove_file(daemon.host_path("/proc/300/stat")).unwrap();
    let before = daemon.gpus.writes();
    daemon.start_daemon();
    daemon.clock.advance(5_000);
    daemon.settle();
    let alice = daemon.user(1000, "alice");
    assert_eq!(daemon.gpus.writes(), before);
    assert_eq!(
        status(&alice)["condition"]["status"],
        "recognitionUnavailable"
    );
    daemon.end_process(300);
    daemon.scan();
    assert_eq!(values(&daemon), PRE_GAME);
    assert_eq!(current_profile(&alice, LINKED)["id"], "daily");
    assert_eq!(writes_to(&daemon, LINKED).len(), before.len() + 2);
}

#[test]
fn reboot_during_a_game_uses_startup_behavior_and_never_its_old_numeric_or_cooling_baseline() {
    for behavior in ["startupProfiles", "restoreLastApplied"] {
        let (mut daemon, alice) = cooled_dota();
        apply_cooling(&alice, separate_cooling());
        ok(
            alice.send_json(json!({ "type": "set_startup_profile", "payload": {
            "gpuId": LINKED, "profileId": "cool"
        } })),
        );
        ok(
            alice.send_json(json!({ "type": "set_startup_behavior", "payload": {
            "behavior": behavior
        } })),
        );
        set_switching(&daemon, &alice, true);
        let manual = last_applied(&daemon);
        // An external numeric tweak becomes a baseline, but never a last applied manual value.
        daemon
            .gpus
            .update(LINKED, |gpu| gpu.power.current_limit_watts = Some(420));
        daemon.run_steam_game(123, 1000, 420, DOTA);
        daemon.scan();
        drop(alice);
        daemon.end_process(123);
        // Simulate the hardware's reset while retaining old runtime files to exercise boot ids.
        daemon.gpus.update(LINKED, |gpu| {
            gpu.power.current_limit_watts = Some(575);
            gpu.cooling = super::harness::cooled_gpu().cooling;
        });
        daemon.reboot_with_retained_boot_state();
        daemon.core().wait_for_startup();
        daemon.scan();
        let alice = daemon.user(1000, "alice");
        assert_eq!(
            values(&daemon).0,
            Some(if behavior == "startupProfiles" {
                320
            } else {
                450
            })
        );
        assert_eq!(last_applied(&daemon), manual);
        assert_eq!(status(&alice)["gpus"], json!([]));
        if behavior == "restoreLastApplied" {
            assert_eq!(cooling(&alice)["applied"], separate_cooling());
        } else {
            assert_eq!(cooling(&alice)["applied"], Value::Null);
        }
        let before = daemon.gpus.writes();
        daemon.scan();
        assert_eq!(daemon.gpus.writes(), before);
    }
}

#[test]
fn restart_preserves_the_game_and_baseline_curves_while_recognition_waits() {
    let (mut daemon, alice) = cooled_dota();
    apply_cooling(&alice, separate_cooling());
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    let shown = status(&alice);
    let manual = last_applied(&daemon);
    drop(alice);
    daemon.restart();
    daemon.clock.advance(5_000);
    daemon.settle();
    let alice = daemon.user(1000, "alice");
    assert_eq!(status(&alice), shown);
    assert_eq!(cooling(&alice)["applied"], shared_curve());
    assert_eq!(
        writes_to(&daemon, LINKED)
            .iter()
            .filter(|s| **s == Setter::PowerLimit)
            .count(),
        3
    );
    std::fs::remove_file(daemon.host_path("/proc/123/stat")).unwrap();
    daemon.scan();
    daemon.set_temperature(LINKED, Some(80));
    daemon.run_curves_for(500);
    assert_eq!(
        daemon.gpus.gpu(LINKED).cooling.fans[0].current_speed_percent,
        Some(100)
    );
    assert_eq!(
        status(&alice)["condition"]["status"],
        "recognitionUnavailable"
    );
    daemon.end_process(123);
    daemon.set_temperature(LINKED, Some(60));
    daemon.scan();
    assert_eq!(cooling(&alice)["applied"], separate_cooling());
    assert_eq!(
        daemon.gpus.gpu(LINKED).cooling.fans[0].current_speed_percent,
        Some(65)
    );
    assert_eq!(last_applied(&daemon), manual);
}

#[test]
fn a_shared_cooling_baseline_is_not_restored_onto_changed_fans_when_the_gpu_returns() {
    let (daemon, alice) = cooled_dota();
    apply_cooling(&alice, shared_curve());
    ok(alice.send_json(json!({ "type": "import_profiles", "payload": { "profiles": [
        { "id": "fixed", "name": "Fixed", "gpuId": LINKED, "settings": {
            "cooling": { "arrangement": "shared", "setting": { "mode": "fixed", "speedPercent": 55 } }
        } }
    ] } })));
    link(&alice, LINKED, Some("fixed"));
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    let mut returning = daemon.gpus.gpu(LINKED);
    daemon.gpus.depart(LINKED);
    daemon.end_process(123);
    daemon.scan();
    returning.cooling.fans.pop();
    returning.cooling.fan_count = 1;
    daemon.gpus.arrive(returning);
    let before = daemon.gpus.writes();
    daemon.scan();
    assert_eq!(daemon.gpus.writes(), before);
    assert_eq!(status(&alice)["gpus"][0]["state"]["status"], "failed");
}

#[test]
fn the_same_running_game_reconciles_reset_controls_when_its_gpu_returns() {
    let (mut daemon, alice) = cooled_dota();
    apply_cooling(&alice, separate_cooling());
    let fixed =
        json!({ "arrangement": "shared", "setting": { "mode": "fixed", "speedPercent": 55 } });
    ok(alice.send_json(
        json!({ "type": "import_profiles", "payload": { "profiles": [
        { "id": "fixed", "name": "Fixed", "gpuId": LINKED, "settings": {
            "powerLimitWatts": value(330), "cooling": fixed
        } }
    ] } }),
    ));
    link(&alice, LINKED, Some("fixed"));
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    let mut returning = daemon.gpus.gpu(LINKED);
    daemon.gpus.depart(LINKED);
    daemon.scan();
    drop(alice);
    // Observed absence must survive a daemon restart as well.
    daemon.restart();
    daemon.clock.advance(5_000);
    daemon.settle();
    returning.power.current_limit_watts = Some(575);
    returning.cooling = super::harness::cooled_gpu().cooling;
    returning.bus_id = Some("0000:08:00.0".into());
    daemon.gpus.arrive(returning);
    daemon.scan();
    let alice = daemon.user(1000, "alice");
    assert_eq!(values(&daemon).0, Some(330));
    assert_eq!(cooling(&alice)["applied"], fixed);
    assert!(daemon
        .gpus
        .gpu(LINKED)
        .cooling
        .fans
        .iter()
        .all(|fan| fan.current_speed_percent == Some(55)));
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(values(&daemon), PRE_GAME);
    assert_eq!(cooling(&alice)["applied"], separate_cooling());
}

#[test]
fn returning_mid_game_reconciles_the_applied_settings_instead_of_later_saved_profile_edits() {
    let (daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    // Save fresh external readings without a manual tuning attempt: saving itself must
    // neither reapply the profile nor establish a manual override. The GPU leaves before
    // any automatic write can check these readings for an external change.
    daemon
        .gpus
        .update(LINKED, |gpu| gpu.power.current_limit_watts = Some(380));
    let revision =
        ok(alice.send_json(json!({ "type": "get_tuning_states" })))[0]["revision"].clone();
    ok(alice.send_json(json!({ "type": "save_to_current_profile", "payload": { "gpuId": LINKED, "revision": revision } })));
    let mut returning = daemon.gpus.gpu(LINKED);
    daemon.gpus.depart(LINKED);
    daemon.scan();
    returning.power.current_limit_watts = Some(575);
    daemon.gpus.arrive(returning);
    daemon.scan();
    assert_eq!(values(&daemon), GAMING);
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(values(&daemon), PRE_GAME);
}

#[test]
fn absence_is_remembered_while_recognition_is_unavailable_but_return_waits_for_a_complete_scan() {
    let (daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    let mut returning = daemon.gpus.gpu(LINKED);
    std::fs::remove_file(daemon.host_path("/proc/123/stat")).unwrap();
    daemon.gpus.depart(LINKED);
    daemon.scan();
    returning.power.current_limit_watts = Some(575);
    daemon.gpus.arrive(returning);
    let before = daemon.gpus.writes();
    daemon.scan();
    assert_eq!(daemon.gpus.writes(), before);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    assert_eq!(values(&daemon), GAMING);
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(values(&daemon), PRE_GAME);
}

#[test]
fn edit_holds_from_two_sessions_defer_until_the_last_session_closes() {
    let (daemon, alice) = linked_dota();
    let other = daemon.user(1000, "alice");
    for session in [&alice, &other] {
        ok(session
            .send_json(json!({ "type": "set_edit_holds", "payload": { "gpuIds": [LINKED] } })));
    }
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    assert_eq!(values(&daemon), PRE_GAME);
    assert_eq!(
        status(&alice)["gpus"][0]["state"],
        json!({
            "status": "waitingForEdits", "profile": { "id": "gaming", "name": "Gaming" }
        })
    );
    ok(other.send_json(json!({ "type": "set_edit_holds", "payload": { "gpuIds": [] } })));
    daemon.settle();
    assert_eq!(values(&daemon), PRE_GAME);
    drop(other);
    daemon.scan();
    assert_eq!(values(&daemon), PRE_GAME);
    drop(alice);
    daemon.scan();
    assert_eq!(values(&daemon), GAMING);
}

#[test]
fn restart_grace_allows_a_reconnected_session_to_resend_its_edit_holds() {
    let (mut daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    drop(alice);
    daemon.restart();
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.clock.advance(4_999);
    daemon.scan();
    assert_eq!(
        values(&daemon),
        PRE_GAME,
        "no automatic write in the grace period"
    );
    let alice = daemon.user(1000, "alice");
    ok(alice.send_json(json!({ "type": "set_edit_holds", "payload": { "gpuIds": [LINKED] } })));
    daemon.clock.advance(1);
    daemon.scan();
    assert_eq!(values(&daemon), PRE_GAME);
    ok(alice.send_json(json!({ "type": "set_edit_holds", "payload": { "gpuIds": [] } })));
    daemon.settle();
    assert_eq!(values(&daemon), GAMING);
}

#[test]
fn a_cooling_edit_hold_defers_profile_replacement_but_the_applied_curve_keeps_running() {
    let (daemon, alice) = cooled_dota();
    apply_cooling(&alice, separate_cooling());
    ok(alice.send_json(json!({ "type": "set_edit_holds", "payload": { "gpuIds": [LINKED] } })));
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    assert_eq!(values(&daemon), PRE_GAME);
    assert_eq!(cooling(&alice)["applied"], separate_cooling());
    daemon.set_temperature(LINKED, Some(60));
    daemon.run_curves_for(500);
    assert_eq!(
        daemon.gpus.gpu(LINKED).cooling.fans[0].current_speed_percent,
        Some(65)
    );
    assert_eq!(
        daemon.gpus.gpu(LINKED).cooling.fans[1].current_speed_percent,
        Some(55)
    );
    assert_eq!(cooling(&alice)["applied"], separate_cooling());
    assert_eq!(
        status(&alice)["gpus"][0]["state"]["status"],
        "waitingForEdits"
    );
    drop(alice);
    daemon.scan();
    let reopened = daemon.user(1000, "alice");
    assert_eq!(cooling(&reopened)["applied"], shared_curve());
    assert_eq!(current_profile(&reopened, LINKED)["id"], "fans");
}

#[test]
fn every_session_receives_the_waiting_status_and_other_users_do_not_see_private_profile_names() {
    let (daemon, alice) = linked_dota();
    let second = daemon.user(1000, "alice");
    let bob = daemon.user(1001, "bob");
    ok(second.send_json(json!({ "type": "set_edit_holds", "payload": { "gpuIds": [LINKED] } })));
    set_switching(&daemon, &alice, true);
    for session in [&alice, &second, &bob] {
        session.pushed_events();
    }
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    for session in [&alice, &second] {
        assert!(session
            .pushed_events()
            .iter()
            .any(|event| event["type"] == "automaticSwitchingChanged"
                && event["payload"]["gpus"][0]["state"]["status"] == "waitingForEdits"));
    }
    assert_eq!(
        status(&bob)["gpus"][0]["state"],
        json!({ "status": "anotherUser" })
    );
}

#[test]
fn gpu_absence_is_remembered_during_an_edit_hold_and_reconciled_after_release() {
    let (daemon, alice) = linked_dota();
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    ok(alice.send_json(json!({ "type": "set_edit_holds", "payload": { "gpuIds": [LINKED] } })));
    let mut returning = daemon.gpus.gpu(LINKED);
    daemon.gpus.depart(LINKED);
    daemon.scan();
    returning.power.current_limit_watts = Some(575);
    daemon.gpus.arrive(returning);
    let before = daemon.gpus.writes();
    daemon.scan();
    assert_eq!(daemon.gpus.writes(), before);
    assert_eq!(values(&daemon).0, Some(575));
    ok(alice.send_json(json!({ "type": "set_edit_holds", "payload": { "gpuIds": [] } })));
    daemon.settle();
    assert_eq!(values(&daemon), GAMING);
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(values(&daemon), PRE_GAME);
}

#[test]
fn a_returning_gpu_keeps_manual_and_external_overrides_even_if_the_game_ended_while_absent() {
    for external in [false, true] {
        for ends_while_absent in [false, true] {
            let (daemon, alice) = linked_dota();
            set_switching(&daemon, &alice, true);
            daemon.run_steam_game(123, 1000, 420, DOTA);
            daemon.scan();
            if external {
                daemon
                    .gpus
                    .update(LINKED, |gpu| gpu.power.current_limit_watts = Some(380));
                link(&alice, LINKED, Some("cool"));
                daemon.settle();
            } else {
                ok(alice.send_json(json!({ "type": "set_power_limit",
                    "payload": { "gpuId": LINKED, "limitWatts": 380 } })));
            }
            assert_eq!(
                status(&alice)["gpus"][0]["state"]["status"],
                "manualOverride"
            );
            let mut returning = daemon.gpus.gpu(LINKED);
            daemon.gpus.depart(LINKED);
            daemon.scan();
            if ends_while_absent {
                daemon.end_process(123);
                daemon.scan();
            }
            // The returning GPU may report reset controls; an override still forbids writes.
            returning.power.current_limit_watts = Some(410);
            let before = daemon.gpus.writes();
            daemon.gpus.arrive(returning);
            daemon.scan();
            assert_eq!(daemon.gpus.writes(), before);
            assert_eq!(values(&daemon).0, Some(410));
            if !ends_while_absent {
                assert_eq!(
                    status(&alice)["gpus"][0]["state"]["status"],
                    "manualOverride"
                );
                daemon.end_process(123);
                daemon.scan();
                assert_eq!(daemon.gpus.writes(), before);
            }
        }
    }
}

#[test]
fn applying_manual_tuning_while_a_profile_waits_keeps_the_override_after_edit_holds_release() {
    let (daemon, alice) = linked_dota();
    ok(alice.send_json(json!({ "type": "set_edit_holds", "payload": { "gpuIds": [LINKED] } })));
    set_switching(&daemon, &alice, true);
    daemon.run_steam_game(123, 1000, 420, DOTA);
    daemon.scan();
    assert_eq!(
        status(&alice)["gpus"][0]["state"]["status"],
        "waitingForEdits"
    );

    ok(alice.send_json(json!({ "type": "set_power_limit",
        "payload": { "gpuId": LINKED, "limitWatts": 380 } })));
    daemon.settle();
    assert_eq!(
        status(&alice)["gpus"][0]["state"],
        json!({ "status": "manualOverride", "changedOutsidePrism": false })
    );
    let before = daemon.gpus.writes();
    ok(alice.send_json(json!({ "type": "set_edit_holds", "payload": { "gpuIds": [] } })));
    daemon.settle();
    assert_eq!(values(&daemon).0, Some(380));
    assert_eq!(
        status(&alice)["gpus"][0]["state"]["status"],
        "manualOverride"
    );
    daemon.end_process(123);
    daemon.scan();
    assert_eq!(daemon.gpus.writes(), before);
    assert_eq!(values(&daemon).0, Some(380));
}
