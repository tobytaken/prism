//! Seam 1: automatic switching while one linked game runs on a GPU, driven by fixture processes
//! and complete scans, through daemon session requests and pushed events.

use serde_json::{json, Value};

use super::harness::{tunable_gpu_with_id, FakeGpuBackend, FakeSession, Harness, Setter};
use super::tuning_store::TUNING_STORE_FILE_NAME;

const LINKED: &str = "GPU-A";
const UNLINKED: &str = "GPU-B";
const DOTA: u32 = 570;

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

    // Cool omits the graphics offset, so Gaming's stays until the game ends.
    link(&alice, LINKED, Some("cool"));
    daemon.settle();
    assert_eq!(values(&daemon), (Some(320), Some(150), Some(0)));
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
    assert_eq!(values(&daemon), PRE_GAME);
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

    daemon.restart();
    let before = writes_to(&daemon, LINKED);
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

    daemon.reboot_with_retained_boot_state();
    daemon.end_process(123);
    let before = writes_to(&daemon, LINKED);
    daemon.scan();
    assert_eq!(writes_to(&daemon, LINKED), before);
}
