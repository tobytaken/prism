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
    ok(session.send_json(json!({ "type": "get_performance" })))["gpus"][0]["cooling"]["control"]
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
