//! Seam 1: GPU tuning profiles owned by the daemon's tuning store, driven through fake sessions.

use serde_json::{Value, json};

use super::harness::{
    FakeGpuBackend, FakeSession, Harness, Setter, tunable_gpu, tunable_gpu_with_id,
};
use super::tuning_store::TUNING_STORE_FILE_NAME;
use prism_gpu::ControlSupport;
use prism_protocol::{DaemonRequest, DaemonResponse, TuningStoreProblem};

fn list(response: Value) -> Vec<Value> {
    assert_eq!(response["type"], "profiles", "{response}");
    response["payload"].as_array().unwrap().clone()
}

/// Save as new, carrying the GPU's revision as the session currently sees it.
fn save_as_new(session: &FakeSession, gpu_id: &str, name: &str) -> Value {
    let states = session.send_json(json!({ "type": "get_tuning_states" }));
    let revision = states["payload"]
        .as_array()
        .and_then(|states| states.iter().find(|state| state["gpu_id"] == gpu_id))
        .map_or(json!(""), |state| state["revision"].clone());
    json!({
        "type": "save_profile_as_new",
        "payload": { "gpu_id": gpu_id, "name": name, "revision": revision }
    })
}

fn apply_by_id(profile_id: &str) -> Value {
    json!({ "type": "apply_profile_by_id", "payload": { "profile_id": profile_id } })
}

fn error_message(response: &Value) -> String {
    assert_eq!(response["type"], "error", "{response}");
    response["payload"]["message"].as_str().unwrap().to_string()
}

fn saved(response: Value) -> Value {
    assert_eq!(response["type"], "profile", "{response}");
    response["payload"].clone()
}

const LIST: fn() -> Value = || json!({ "type": "list_profiles" });

#[test]
fn users_see_only_their_own_profiles_and_a_reused_uid_with_another_username_inherits_none() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    let quiet = saved(alice.send_json(save_as_new(&alice, "GPU-123", "Quiet")));

    assert_eq!(list(alice.send_json(LIST())), vec![quiet.clone()]);
    let bob = daemon.user(1001, "bob");
    assert_eq!(list(bob.send_json(LIST())), Vec::<Value>::new());
    let carol = daemon.user(1000, "carol");
    assert_eq!(list(carol.send_json(LIST())), Vec::<Value>::new());

    // Another user can't apply the profile by guessing its id, and nothing is written.
    for other in [&bob, &carol] {
        let response = other.send_json(apply_by_id(quiet["id"].as_str().unwrap()));
        assert_eq!(response["payload"]["code"], "not_found", "{response}");
    }
    assert!(daemon.gpus.writes().is_empty());

    // The new owner of the uid saves under the same name without touching alice's profile.
    saved(carol.send_json(save_as_new(&carol, "GPU-123", "Quiet")));
    assert_eq!(list(alice.send_json(LIST())), vec![quiet]);
}

#[test]
fn save_as_new_captures_applied_known_values_and_keeps_explicit_default_requests() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    // Use default for the graphics offset and the power limit, then a number for memory.
    alice.send_json(json!({ "type": "reset_power_limit", "payload": { "gpu_id": "GPU-123" } }));
    alice.send_json(json!({
        "type": "set_clock_offsets",
        "payload": { "gpu_id": "GPU-123", "graphics_offset_mhz": "default", "memory_offset_mhz": 40 }
    }));

    let profile = saved(alice.send_json(save_as_new(&alice, "GPU-123", "  Defaults  ")));
    assert_eq!(profile["name"], "Defaults");
    assert_eq!(profile["gpu_id"], "GPU-123");
    assert_eq!(
        profile["settings"],
        json!({
            "power_limit_watts": { "kind": "default" },
            "graphics_offset_mhz": { "kind": "default" },
            "memory_offset_mhz": { "kind": "value", "value": 40 }
        })
    );

    // A number equal to today's default stays a number.
    alice.send_json(json!({
        "type": "set_power_limit", "payload": { "gpu_id": "GPU-123", "limit_watts": 575 }
    }));
    let fixed = saved(alice.send_json(save_as_new(&alice, "GPU-123", "Fixed")));
    assert_eq!(
        fixed["settings"]["power_limit_watts"],
        json!({ "kind": "value", "value": 575 })
    );

    // A default requested through a profile is kept too, until a later number replaces it.
    alice.send_json(json!({
        "type": "apply_profile",
        "payload": { "gpu_id": "GPU-123", "settings": { "power_limit_watts": { "kind": "default" } } }
    }));
    let again = saved(alice.send_json(save_as_new(&alice, "GPU-123", "Default again")));
    assert_eq!(
        again["settings"]["power_limit_watts"],
        json!({ "kind": "default" })
    );

    // A default request no longer describes the GPU once its value has changed outside Prism.
    daemon
        .gpus
        .update("GPU-123", |gpu| gpu.power.current_limit_watts = Some(500));
    let outside = saved(alice.send_json(save_as_new(&alice, "GPU-123", "Outside")));
    assert_eq!(
        outside["settings"]["power_limit_watts"],
        json!({ "kind": "value", "value": 500 })
    );
}

#[test]
fn a_failed_manual_write_does_not_replace_the_recorded_request() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    alice.send_json(json!({ "type": "reset_power_limit", "payload": { "gpu_id": "GPU-123" } }));
    daemon
        .gpus
        .fail("GPU-123", Setter::PowerLimit, "Power write denied");
    alice.send_json(json!({
        "type": "set_power_limit", "payload": { "gpu_id": "GPU-123", "limit_watts": 575 }
    }));

    let profile = saved(alice.send_json(save_as_new(&alice, "GPU-123", "Quiet")));
    assert_eq!(
        profile["settings"]["power_limit_watts"],
        json!({ "kind": "default" })
    );
}

#[test]
fn save_as_new_blocks_on_unknown_support_or_unavailable_values_and_saves_nothing() {
    let mut unknown = tunable_gpu();
    unknown.clocks.graphics.support = ControlSupport::Unknown;
    unknown.clocks.graphics.reason = Some("Graphics probe failed".into());
    let mut unreadable = tunable_gpu();
    unreadable.power.current_limit_watts = None;
    unreadable.power.current_value_error = Some("Power read failed".into());
    let mut nothing = tunable_gpu();
    nothing.power.support = ControlSupport::Unsupported;
    nothing.clocks.graphics.support = ControlSupport::Unsupported;
    nothing.clocks.memory.support = ControlSupport::Unsupported;
    for (gpu, reason) in [
        (
            unknown,
            "Graphics clock offset: support is unknown. Graphics probe failed",
        ),
        (
            unreadable,
            "Power limit: current configured value is unavailable. Power read failed",
        ),
        (
            nothing,
            "This GPU has no supported profile settings (power limit, clock offsets or cooling).",
        ),
    ] {
        let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![gpu]));
        let alice = daemon.user(1000, "alice");
        let response = alice.send_json(save_as_new(&alice, "GPU-123", "Quiet"));
        assert_eq!(error_message(&response), reason);
        assert_eq!(list(alice.send_json(LIST())), Vec::<Value>::new());
    }
}

#[test]
fn unsupported_settings_are_omitted_from_a_saved_profile() {
    let mut gpu = tunable_gpu();
    gpu.clocks.memory.support = ControlSupport::Unsupported;
    gpu.clocks.memory.current_offset_mhz = None;
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![gpu]));
    let alice = daemon.user(1000, "alice");
    let profile = saved(alice.send_json(save_as_new(&alice, "GPU-123", "Quiet")));
    assert_eq!(
        profile["settings"],
        json!({
            "power_limit_watts": { "kind": "value", "value": 475 },
            "graphics_offset_mhz": { "kind": "value", "value": 120 }
        })
    );
}

#[test]
fn names_are_trimmed_and_unique_per_gpu_regardless_of_letter_case() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![
        tunable_gpu_with_id("GPU-123"),
        tunable_gpu_with_id("GPU-456"),
    ]));
    let alice = daemon.user(1000, "alice");
    saved(alice.send_json(save_as_new(&alice, "GPU-123", "Quiet")));

    let clash = alice.send_json(save_as_new(&alice, "GPU-123", " QUIET "));
    assert_eq!(
        error_message(&clash),
        "A profile named \"QUIET\" already exists for this GPU. Choose another name."
    );
    assert_eq!(
        error_message(&alice.send_json(save_as_new(&alice, "GPU-123", "   "))),
        "Enter a name for the profile."
    );
    let other_gpu = saved(alice.send_json(save_as_new(&alice, "GPU-456", "Quiet")));
    assert_eq!(other_gpu["gpu_id"], "GPU-456");
    let names: Vec<_> = list(alice.send_json(LIST()))
        .into_iter()
        .map(|profile| (profile["gpu_id"].clone(), profile["name"].clone()))
        .collect();
    assert_eq!(
        names,
        vec![
            (json!("GPU-123"), json!("Quiet")),
            (json!("GPU-456"), json!("Quiet"))
        ]
    );
}

#[test]
fn the_300_watt_profile_against_a_280_watt_maximum_proposes_a_correction_and_writes_nothing_until_requested()
 {
    let mut gpu = tunable_gpu();
    gpu.power.min_limit_watts = Some(100);
    gpu.power.max_limit_watts = Some(280);
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![gpu]));
    let alice = daemon.user(1000, "alice");
    alice.seed_profiles(json!([{
        "id": "quiet",
        "name": "Quiet",
        "gpu_id": "GPU-123",
        "settings": {
            "power_limit_watts": { "kind": "value", "value": 300 },
            "memory_offset_mhz": { "kind": "value", "value": 40 }
        }
    }]));

    let review = alice.send_json(apply_by_id("quiet"));
    assert_eq!(review["type"], "profile_application");
    assert_eq!(
        review["payload"]["corrections"],
        json!([{
            "setting": "power_limit_watts",
            "saved_value": 300,
            "proposed_value": 280,
            "bound": "maximum"
        }])
    );
    assert_eq!(review["payload"]["power"]["status"], "unattempted");
    assert_eq!(review["payload"]["memory"]["status"], "unattempted");
    assert!(daemon.gpus.writes().is_empty());

    // A correction that no longer matches the saved value changes neither the profile nor the GPU.
    let stale = alice.send_json(json!({
        "type": "correct_and_apply_profile",
        "payload": { "profile_id": "quiet", "corrections": [{
            "setting": "power_limit_watts", "saved_value": 320, "proposed_value": 280, "bound": "maximum"
        }] }
    }));
    assert_eq!(
        error_message(&stale),
        "The saved profile changed while its corrections were being reviewed. Load it again to \
         review current corrections. No GPU settings were changed."
    );
    assert!(daemon.gpus.writes().is_empty());

    let corrected = alice.send_json(json!({
        "type": "correct_and_apply_profile",
        "payload": { "profile_id": "quiet", "corrections": review["payload"]["corrections"] }
    }));
    assert_eq!(
        corrected["type"], "corrected_profile_application",
        "{corrected}"
    );
    let settings = json!({
        "power_limit_watts": { "kind": "value", "value": 280 },
        "memory_offset_mhz": { "kind": "value", "value": 40 }
    });
    assert_eq!(corrected["payload"]["profile"]["settings"], settings);
    assert_eq!(
        corrected["payload"]["application"]["power"]["status"],
        "succeeded"
    );
    assert_eq!(
        corrected["payload"]["application"]["memory"]["status"],
        "succeeded"
    );
    let gpu = daemon.gpus.gpu("GPU-123");
    assert_eq!(gpu.power.current_limit_watts, Some(280));
    assert_eq!(gpu.clocks.memory.current_offset_mhz, Some(40));
    assert_eq!(list(alice.send_json(LIST()))[0]["settings"], settings);
}

#[test]
fn a_corrected_profile_stays_saved_when_its_application_fails() {
    let mut gpu = tunable_gpu();
    gpu.power.max_limit_watts = Some(450);
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![gpu]));
    daemon
        .gpus
        .fail("GPU-123", Setter::PowerLimit, "Power write denied");
    let alice = daemon.user(1000, "alice");
    alice.seed_profiles(json!([{
        "id": "loud", "name": "Loud", "gpu_id": "GPU-123",
        "settings": { "power_limit_watts": { "kind": "value", "value": 500 } }
    }]));

    let corrected = alice.send_json(json!({
        "type": "correct_and_apply_profile",
        "payload": { "profile_id": "loud", "corrections": [{
            "setting": "power_limit_watts", "saved_value": 500, "proposed_value": 450, "bound": "maximum"
        }] }
    }));
    assert_eq!(
        corrected["payload"]["application"]["power"],
        json!({ "status": "failed", "error": "Power write denied" })
    );
    assert_eq!(
        list(alice.send_json(LIST()))[0]["settings"]["power_limit_watts"],
        json!({ "kind": "value", "value": 450 })
    );
}

#[test]
fn applying_a_profile_by_id_writes_only_its_settings_on_its_owning_gpu() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![
        tunable_gpu_with_id("GPU-123"),
        tunable_gpu_with_id("GPU-456"),
    ]));
    let alice = daemon.user(1000, "alice");
    alice.seed_profiles(json!([{
        "id": "quiet", "name": "Quiet", "gpu_id": "GPU-456",
        "settings": { "power_limit_watts": { "kind": "value", "value": 400 } }
    }]));

    let applied = alice.send_json(apply_by_id("quiet"));
    assert_eq!(applied["payload"]["power"]["status"], "succeeded");
    assert!(applied["payload"]["graphics"].is_null());
    assert_eq!(
        daemon.gpus.writes(),
        vec![("GPU-456".to_string(), Setter::PowerLimit)]
    );
    assert_eq!(
        daemon.gpus.gpu("GPU-456").power.current_limit_watts,
        Some(400)
    );
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(475)
    );
}

#[test]
fn profiles_and_recorded_default_requests_survive_a_daemon_restart_and_a_reboot() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let alice = daemon.user(1000, "alice");
    alice.send_json(json!({ "type": "reset_power_limit", "payload": { "gpu_id": "GPU-123" } }));
    let quiet = saved(alice.send_json(save_as_new(&alice, "GPU-123", "Quiet")));
    drop(alice);

    daemon.restart();
    let alice = daemon.user(1000, "alice");
    assert_eq!(list(alice.send_json(LIST())), vec![quiet.clone()]);
    drop(alice);

    daemon.reboot();
    let alice = daemon.user(1000, "alice");
    assert_eq!(list(alice.send_json(LIST())), vec![quiet]);
    let after_reboot = saved(alice.send_json(save_as_new(&alice, "GPU-123", "After reboot")));
    assert_eq!(
        after_reboot["settings"]["power_limit_watts"],
        json!({ "kind": "default" })
    );
    assert_eq!(
        list(daemon.user(1001, "bob").send_json(LIST())),
        Vec::<Value>::new()
    );
}

#[test]
fn profile_requests_for_a_gpu_without_a_stable_identity_are_rejected() {
    let mut gpu = tunable_gpu();
    gpu.id = "nvidia-0".into();
    gpu.stable_id = None;
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![gpu]));
    let alice = daemon.user(1000, "alice");

    for gpu_id in ["nvidia-0", ""] {
        let response = alice.send_json(save_as_new(&alice, gpu_id, "Quiet"));
        assert_eq!(
            error_message(&response),
            "Profiles are unavailable because this GPU's stable identity (UUID) could not be \
             verified. Refresh the GPU readings and try again."
        );
    }
    assert_eq!(list(alice.send_json(LIST())), Vec::<Value>::new());
}

#[test]
fn profile_requests_need_a_session_that_identifies_the_caller() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let DaemonResponse::Error(error) = daemon.send(DaemonRequest::ListProfiles) else {
        panic!("expected an error")
    };
    assert_eq!(error.code, prism_gpu::GpuCommandErrorCode::NoPermission);
}

#[test]
fn an_unhealthy_store_refuses_profile_requests_and_is_preserved() {
    let corrupt: &[u8] = b"{\"schema_version\": 2, \"users\": [tru";
    let daemon = Harness::start_with_persistent_files(
        FakeGpuBackend::with_gpus(vec![tunable_gpu()]),
        &[(TUNING_STORE_FILE_NAME, corrupt)],
    );
    let alice = daemon.user(1000, "alice");
    for request in [
        LIST(),
        save_as_new(&alice, "GPU-123", "Quiet"),
        apply_by_id("quiet"),
    ] {
        let response = alice.send_json(request);
        assert_eq!(response["type"], "error", "{response}");
        assert!(
            error_message(&response).contains("tuning store"),
            "{response}"
        );
    }
    let health = daemon.status().tuning_store.unwrap();
    assert!(matches!(
        health,
        prism_protocol::TuningStoreHealth::Unhealthy {
            problem: TuningStoreProblem::Unreadable,
            ..
        }
    ));
    assert_eq!(
        daemon.persistent_file(TUNING_STORE_FILE_NAME).as_deref(),
        Some(corrupt)
    );
    assert!(daemon.gpus.writes().is_empty());
}

#[test]
fn live_clock_offsets_accept_the_app_field_names_and_default_requests() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let response = daemon.user(1000, "alice").send_json(json!({
        "type": "set_clock_offsets",
        "payload": { "gpu_id": "GPU-123", "graphics_offset_mhz": "default", "memory_offset_mhz": 80 }
    }));
    assert_eq!(response["payload"]["graphics"]["status"], "succeeded");
    assert_eq!(response["payload"]["memory"]["status"], "succeeded");
    let gpu = daemon.gpus.gpu("GPU-123");
    assert_eq!(gpu.clocks.graphics.current_offset_mhz, Some(0));
    assert_eq!(gpu.clocks.memory.current_offset_mhz, Some(80));
}

#[test]
fn cooling_profiles_capture_complete_curves_and_ticks_do_not_invalidate_saving() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(
        vec![super::harness::cooled_gpu()],
    ));
    let alice = daemon.user(1000, "alice");
    let cooling = json!({"arrangement":"shared","setting":{"mode":"curve","points":[
        {"temperature_c":45,"speed_percent":20}, {"temperature_c":85,"speed_percent":100}
    ]}});
    alice.send_json(
        json!({"type":"apply_cooling","payload":{"gpu_id":"GPU-123","configuration":cooling}}),
    );
    let request = save_as_new(&alice, "GPU-123", "Curve");
    daemon.set_temperature("GPU-123", Some(80));
    daemon.run_curves_for(500);
    let profile = saved(alice.send_json(request));
    assert_eq!(profile["settings"]["cooling"], cooling);
}

#[test]
fn applying_a_cooling_profile_evaluates_the_current_temperature_and_legacy_omission_leaves_it_alone()
 {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(
        vec![super::harness::cooled_gpu()],
    ));
    let alice = daemon.user(1000, "alice");
    let cooling = json!({"arrangement":"per_fan","fans":[
        {"fan_index":0,"setting":{"mode":"curve","points":[{"temperature_c":45,"speed_percent":20},{"temperature_c":85,"speed_percent":100}]}},
        {"fan_index":1,"setting":{"mode":"fixed","speed_percent":50}}
    ]});
    alice.seed_profiles(
        json!([{"id":"curve","name":"Curve","gpu_id":"GPU-123","settings":{"cooling":cooling}}]),
    );
    daemon.set_temperature("GPU-123", Some(80));
    let result = alice.send_json(apply_by_id("curve"));
    assert_eq!(
        result["payload"]["cooling"]["outcome"]["status"], "succeeded",
        "{result}"
    );
    let performance = alice.send_json(json!({"type":"get_performance"}));
    assert_eq!(
        performance["payload"]["gpus"][0]["cooling"]["control"]["remembered_curve"],
        cooling
    );
    assert_eq!(
        daemon.gpus.gpu("GPU-123").cooling.fans[0].current_speed_percent,
        Some(90)
    );
    assert_eq!(
        daemon.gpus.gpu("GPU-123").cooling.fans[1].current_speed_percent,
        Some(50)
    );
    alice.send_json(json!({"type":"apply_profile","payload":{"gpu_id":"GPU-123","settings":{"power_limit_watts":{"kind":"default"}}}}));
    let profile = saved(alice.send_json(save_as_new(&alice, "GPU-123", "Still curve")));
    assert_eq!(profile["settings"]["cooling"], cooling);
}

#[test]
fn cooling_range_corrections_require_review_and_keep_the_corrected_profile_after_failure() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(
        vec![super::harness::cooled_gpu()],
    ));
    let alice = daemon.user(1000, "alice");
    let cooling = json!({"arrangement":"shared","setting":{"mode":"fixed","speed_percent":10}});
    alice.seed_profiles(
        json!([{"id":"fixed","name":"Fixed","gpu_id":"GPU-123","settings":{"cooling":cooling}}]),
    );
    let result = alice.send_json(apply_by_id("fixed"));
    assert_eq!(
        result["payload"]["cooling"]["outcome"]["status"],
        "unattempted"
    );
    let corrections = result["payload"]["corrections"].clone();
    assert_eq!(corrections[0]["setting"], "cooling", "{result}");
    assert_eq!(
        corrections[0]["proposed_value"]["setting"]["speed_percent"],
        20
    );
    assert!(daemon.gpus.writes().is_empty());
    daemon.gpus.fail("GPU-123", Setter::FanSpeed, "Denied");
    let result = alice.send_json(json!({"type":"correct_and_apply_profile","payload":{"profile_id":"fixed","corrections":corrections}}));
    assert_eq!(
        result["payload"]["profile"]["settings"]["cooling"]["setting"]["speed_percent"], 20,
        "{result}"
    );
    assert_eq!(
        result["payload"]["application"]["cooling"]["outcome"]["status"],
        "failed"
    );
}

#[test]
fn cooling_profile_validation_blocks_all_writes_when_fixed_bounds_are_unknown() {
    let mut gpu = super::harness::cooled_gpu();
    gpu.cooling.fans[0].min_speed_percent = None;
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![gpu]));
    let result = daemon.send_json(
        json!({"type":"apply_profile","payload":{"gpu_id":"GPU-123","settings":{
            "power_limit_watts":{"kind":"value","value":500},
            "cooling":{"arrangement":"shared","setting":{"mode":"fixed","speed_percent":60}}
        }}}),
    );
    assert_eq!(
        result["payload"]["power"]["status"], "unattempted",
        "{result}"
    );
    assert!(daemon.gpus.writes().is_empty());
}

#[test]
fn automatic_cooling_is_saved_as_default_intent_and_survives_restart() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(
        vec![super::harness::cooled_gpu()],
    ));
    let alice = daemon.user(1000, "alice");
    let profile = saved(alice.send_json(save_as_new(&alice, "GPU-123", "Automatic")));
    assert_eq!(
        profile["settings"]["cooling"],
        json!({"arrangement":"shared","setting":{"mode":"automatic"}})
    );
    drop(alice);
    daemon.restart();
    let alice = daemon.user(1000, "alice");
    assert_eq!(list(alice.send_json(LIST())), vec![profile.clone()]);
    let result = alice.send_json(apply_by_id(profile["id"].as_str().unwrap()));
    assert_eq!(
        result["payload"]["cooling"]["outcome"]["status"],
        "succeeded"
    );
}

#[test]
fn a_partial_cooling_profile_reports_each_fan_and_does_not_become_current() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(
        vec![super::harness::cooled_gpu()],
    ));
    let alice = daemon.user(1000, "alice");
    saved(alice.send_json(save_as_new(&alice, "GPU-123", "Before")));
    alice.seed_profiles(
        json!([{"id":"fixed","name":"Fixed","gpu_id":"GPU-123","settings":{
            "cooling":{"arrangement":"shared","setting":{"mode":"fixed","speed_percent":70}}
        }}]),
    );
    daemon
        .gpus
        .fail_fan("GPU-123", Setter::FanSpeed, 1, "Denied");
    let result = alice.send_json(apply_by_id("fixed"));
    let cooling = &result["payload"]["cooling"];
    assert_eq!(cooling["outcome"]["status"], "failed");
    assert_eq!(cooling["fans"][0]["outcome"]["status"], "succeeded");
    assert_eq!(cooling["fans"][1]["outcome"]["status"], "failed");
    let states = alice.send_json(json!({"type":"get_tuning_states"}));
    assert!(states["payload"][0]["current_profile"].is_null());
}

#[test]
fn unknown_cooling_policy_blocks_capture_instead_of_saving_a_sampled_speed() {
    let mut gpu = super::harness::cooled_gpu();
    gpu.cooling.fans[0].mode = Some(prism_gpu::FanControlMode::Manual);
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![gpu]));
    let alice = daemon.user(1000, "alice");
    let response = alice.send_json(save_as_new(&alice, "GPU-123", "Unknown"));
    assert!(
        error_message(&response).contains("Cooling: the current configured policy is unavailable")
    );
    assert!(list(alice.send_json(LIST())).is_empty());
}

#[test]
fn cooling_application_keeps_working_with_an_unhealthy_store_and_reports_unrecorded_manual_state() {
    let preserved = b"{\"schema_version\":999}";
    let daemon = Harness::start_with_persistent_files(
        FakeGpuBackend::with_gpus(vec![super::harness::cooled_gpu()]),
        &[(TUNING_STORE_FILE_NAME, preserved)],
    );
    let result = daemon.send_json(
        json!({"type":"apply_profile","payload":{"gpu_id":"GPU-123","settings":{
            "cooling":{"arrangement":"shared","setting":{"mode":"fixed","speed_percent":70}}
        }}}),
    );
    assert_eq!(
        result["payload"]["cooling"]["outcome"]["status"],
        "succeeded"
    );
    assert!(
        result["payload"]["cooling"]["outcome"]["error"]
            .as_str()
            .unwrap()
            .contains("not recorded")
    );
    assert_eq!(
        daemon.persistent_file(TUNING_STORE_FILE_NAME).as_deref(),
        Some(&preserved[..])
    );
}

#[test]
fn per_fan_profiles_must_cover_the_complete_current_fan_inventory() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(
        vec![super::harness::cooled_gpu()],
    ));
    let result = daemon.send_json(json!({"type":"apply_profile","payload":{"gpu_id":"GPU-123","settings":{
        "power_limit_watts":{"kind":"value","value":500},
        "cooling":{"arrangement":"per_fan","fans":[{"fan_index":0,"setting":{"mode":"fixed","speed_percent":60}}]}
    }}}));
    assert_eq!(
        result["payload"]["power"]["status"], "unattempted",
        "{result}"
    );
    assert!(daemon.gpus.writes().is_empty());
}

#[test]
fn cooling_corrections_do_not_require_speed_bounds_for_automatic_fans() {
    let mut gpu = super::harness::cooled_gpu();
    gpu.cooling.fans[0].min_speed_percent = None;
    gpu.cooling.fans[0].max_speed_percent = None;
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![gpu]));
    let result = daemon.send_json(
        json!({"type":"apply_profile","payload":{"gpu_id":"GPU-123","settings":{
            "cooling":{"arrangement":"per_fan","fans":[
                {"fan_index":0,"setting":{"mode":"automatic"}},
                {"fan_index":1,"setting":{"mode":"fixed","speed_percent":10}}
            ]}
        }}}),
    );
    assert_eq!(
        result["payload"]["corrections"][0]["proposed_value"]["fans"][1]["setting"]["speed_percent"],
        20,
        "{result}"
    );
    assert!(daemon.gpus.writes().is_empty());
}

#[test]
fn unrecorded_manual_cooling_is_reported_as_possibly_stale_until_cooling_is_recorded() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(
        vec![super::harness::cooled_gpu()],
    ));
    let alice = daemon.user(1000, "alice");
    alice.send_json(json!({"type":"reset_power_limit","payload":{"gpu_id":"GPU-123"}}));
    let blocker = daemon.persistent_path(&format!("{TUNING_STORE_FILE_NAME}.tmp"));
    std::fs::create_dir(&blocker).unwrap();
    let result = alice.send_json(
        json!({"type":"apply_cooling","payload":{"gpu_id":"GPU-123","configuration":{
            "arrangement":"shared","setting":{"mode":"fixed","speed_percent":70}
        }}}),
    );
    assert_eq!(result["payload"]["outcome"]["status"], "succeeded");
    drop(alice);
    std::fs::remove_dir(&blocker).unwrap();
    daemon.restart();
    let alice = daemon.user(1000, "alice");
    let startup = || {
        let response = alice.send_json(json!({"type":"get_startup_state"}));
        assert_eq!(response["type"], "startup_state", "{response}");
        response["payload"]["possibly_stale"].clone()
    };
    assert_eq!(startup(), json!(["GPU-123"]));
    alice.send_json(json!({"type":"reset_power_limit","payload":{"gpu_id":"GPU-123"}}));
    assert_eq!(startup(), json!(["GPU-123"]));
    alice.send_json(
        json!({"type":"apply_profile","payload":{"gpu_id":"GPU-123","settings":{
            "cooling":{"arrangement":"shared","setting":{"mode":"automatic"}}
        }}}),
    );
    assert_eq!(startup(), json!([]));
}

#[test]
fn curve_range_review_preserves_valid_full_speed_plateau_points() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(
        vec![super::harness::cooled_gpu()],
    ));
    let result = daemon.send_json(json!({"type":"apply_profile","payload":{"gpu_id":"GPU-123","settings":{
        "cooling":{"arrangement":"shared","setting":{"mode":"curve","points":[
            {"temperature_c":45,"speed_percent":10}, {"temperature_c":85,"speed_percent":100}, {"temperature_c":90,"speed_percent":100}
        ]}}
    }}}));
    assert_eq!(
        result["payload"]["corrections"][0]["proposed_value"]["setting"]["points"],
        json!([
            {"temperature_c":45,"speed_percent":20}, {"temperature_c":85,"speed_percent":100}, {"temperature_c":90,"speed_percent":100}
        ]),
        "{result}"
    );
    assert!(daemon.gpus.writes().is_empty());
}

#[test]
fn version_eight_remembered_curves_survive_adding_profile_cooling() {
    let curve = json!({"arrangement":"shared","setting":{"mode":"curve","points":[
        {"temperature_c":45,"speed_percent":20}, {"temperature_c":85,"speed_percent":100}
    ]}});
    let state = serde_json::to_vec(&json!({"schema_version":8,"users":[{
        "uid":1000,"username":"alice","remembered_curves":{"GPU-123":curve},
        "profiles":[{"id":"legacy","name":"Legacy","gpu_id":"GPU-123","settings":{"power_limit_watts":{"kind":"default"}}}]
    }]})).unwrap();
    let mut daemon = Harness::start_with_persistent_files(
        FakeGpuBackend::with_gpus(vec![super::harness::cooled_gpu()]),
        &[(TUNING_STORE_FILE_NAME, &state)],
    );
    let alice = daemon.user(1000, "alice");
    let profile = saved(alice.send_json(save_as_new(&alice, "GPU-123", "With cooling")));
    assert_eq!(
        profile["settings"]["cooling"],
        json!({"arrangement":"shared","setting":{"mode":"automatic"}})
    );
    drop(alice);
    daemon.restart();
    let alice = daemon.user(1000, "alice");
    let profiles = list(alice.send_json(LIST()));
    assert!(
        profiles.iter().find(|p| p["id"] == "legacy").unwrap()["settings"]
            .get("cooling")
            .is_none()
    );
    let performance = alice.send_json(json!({"type":"get_performance"}));
    assert_eq!(
        performance["payload"]["gpus"][0]["cooling"]["control"]["remembered_curve"],
        curve
    );
    let stored: Value =
        serde_json::from_slice(&daemon.persistent_file(TUNING_STORE_FILE_NAME).unwrap()).unwrap();
    assert_eq!(
        stored["schema_version"],
        super::tuning_store::TUNING_STORE_SCHEMA_VERSION
    );
}
