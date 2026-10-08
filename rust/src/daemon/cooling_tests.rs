//! Seam 1: applying cooling configurations and following shared fan curves in the daemon.

use serde_json::{json, Value};

use super::harness::{cooled_gpu, FakeGpuBackend, Harness, Setter};
use crate::gpu::FanControlMode;

/// The starter curve for a 20-100% range under a 90°C maximum operating temperature.
const STARTER: [(i64, i64); 4] = [(45, 20), (65, 60), (75, 80), (85, 100)];

fn points(points: &[(i64, i64)]) -> Value {
    points
        .iter()
        .map(|(temperature, speed)| json!({ "temperatureC": temperature, "speedPercent": speed }))
        .collect()
}

fn shared_curve(curve: &[(i64, i64)]) -> Value {
    json!({ "arrangement": "shared", "setting": { "mode": "curve", "points": points(curve) } })
}

fn apply(daemon: &Harness, configuration: Value) -> Value {
    let response = daemon.send_json(json!({
        "type": "apply_cooling",
        "payload": { "gpuId": "GPU-123", "configuration": configuration },
    }));
    assert_eq!(response["type"], "cooling_application", "{response}");
    response["payload"].clone()
}

/// Prism's cooling control of the GPU as the daemon reports it in performance readings.
fn control(daemon: &Harness) -> Value {
    let response = daemon.send_json(json!({ "type": "get_performance" }));
    response["payload"]["gpus"][0]["cooling"]["control"].clone()
}

/// Each fan's reported mode and speed on the fake GPU.
fn fans(daemon: &Harness) -> Vec<(Option<FanControlMode>, Option<u32>)> {
    daemon
        .gpus
        .gpu("GPU-123")
        .cooling
        .fans
        .iter()
        .map(|fan| (fan.mode, fan.current_speed_percent))
        .collect()
}

fn manual(speed: u32) -> (Option<FanControlMode>, Option<u32>) {
    (Some(FanControlMode::Manual), Some(speed))
}

#[test]
fn applying_a_shared_curve_requests_its_speed_at_the_current_temperature() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));

    let application = apply(&daemon, shared_curve(&STARTER));

    let succeeded = json!({ "status": "succeeded", "error": null });
    assert_eq!(
        application,
        json!({
            "revision": 1,
            "outcome": succeeded,
            "fans": [
                { "fanIndex": 0, "outcome": succeeded },
                { "fanIndex": 1, "outcome": succeeded },
            ],
        })
    );
    // 71°C lies 6/10 of the way from (65°C, 60%) to (75°C, 80%).
    assert_eq!(fans(&daemon), vec![manual(72), manual(72)]);
    assert_eq!(
        control(&daemon),
        json!({
            "revision": 1,
            "applied": shared_curve(&STARTER),
            "requested": [
                { "fanIndex": 0, "speedPercent": 72 },
                { "fanIndex": 1, "speedPercent": 72 },
            ],
            "curveTemperatureC": 71,
            "suspension": null,
            "recovery": null,
        })
    );
}

/// The speed each fan currently reports.
fn speeds(daemon: &Harness) -> Vec<Option<u32>> {
    fans(daemon).into_iter().map(|(_, speed)| speed).collect()
}

#[test]
fn a_curve_follows_rising_temperature_on_the_next_evaluation() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));

    daemon.set_temperature("GPU-123", Some(80));
    assert_eq!(speeds(&daemon), vec![Some(72), Some(72)]);
    daemon.run_curves_for(500);

    // 80°C lies halfway from (75°C, 80%) to (85°C, 100%).
    assert_eq!(speeds(&daemon), vec![Some(90), Some(90)]);
    assert_eq!(control(&daemon)["curveTemperatureC"], 80);
    assert_eq!(control(&daemon)["requested"][0]["speedPercent"], 90);
}

#[test]
fn beyond_its_end_points_a_curve_requests_the_nearest_end_points_speed() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    daemon.set_temperature("GPU-123", Some(30));
    apply(&daemon, shared_curve(&STARTER));
    assert_eq!(speeds(&daemon), vec![Some(20), Some(20)]);

    daemon.set_temperature("GPU-123", Some(95));
    daemon.run_curves_for(500);
    assert_eq!(speeds(&daemon), vec![Some(100), Some(100)]);
}

#[test]
fn a_lower_demand_waits_five_seconds_and_then_uses_the_demand_of_that_moment() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));

    daemon.set_temperature("GPU-123", Some(65));
    // The next evaluation sees the lower demand and starts the countdown.
    daemon.run_curves_for(500);
    daemon.run_curves_for(2_000);
    // Demand falls further during the countdown, still below the current speed throughout.
    daemon.set_temperature("GPU-123", Some(55));
    daemon.run_curves_for(2_500);
    assert_eq!(speeds(&daemon), vec![Some(72), Some(72)]);

    daemon.run_curves_for(500);
    // Five seconds after the lower demand was seen, the speed drops to the demand at 55°C.
    assert_eq!(speeds(&daemon), vec![Some(40), Some(40)]);
    assert_eq!(control(&daemon)["requested"][1]["speedPercent"], 40);
}

#[test]
fn demand_returning_to_the_current_speed_cancels_the_countdown() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));

    daemon.set_temperature("GPU-123", Some(60));
    daemon.run_curves_for(3_000);
    daemon.set_temperature("GPU-123", Some(71));
    daemon.run_curves_for(500);
    daemon.set_temperature("GPU-123", Some(60));
    daemon.run_curves_for(500);
    daemon.run_curves_for(4_500);
    // Only 4.5 seconds of continuously lower demand since the rebound.
    assert_eq!(speeds(&daemon), vec![Some(72), Some(72)]);

    daemon.run_curves_for(500);
    assert_eq!(speeds(&daemon), vec![Some(50), Some(50)]);
}

#[test]
fn the_curve_keeps_running_after_the_app_session_closes() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let session = daemon.user(1000, "alice");
    let response = session.send_json(json!({
        "type": "apply_cooling",
        "payload": { "gpuId": "GPU-123", "configuration": shared_curve(&STARTER) },
    }));
    assert_eq!(response["payload"]["outcome"]["status"], "succeeded");
    drop(session);

    daemon.set_temperature("GPU-123", Some(80));
    daemon.run_curves_for(500);
    assert_eq!(speeds(&daemon), vec![Some(90), Some(90)]);
}

#[test]
fn a_resumed_curve_follows_the_temperature_while_switching_waits_and_sessions_reconnect() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    daemon.install_steam_app("alice", 570, "Dota 2");
    let alice = daemon.user(1000, "alice");
    for request in [
        json!({ "type": "import_profiles", "payload": { "profiles": [
            { "id": "gaming", "name": "Gaming", "gpuId": "GPU-123",
              "settings": { "powerLimitWatts": { "kind": "value", "value": 350 } } },
        ] } }),
        json!({ "type": "add_steam_game", "payload": { "appId": 570 } }),
        json!({ "type": "set_game_profile", "payload": {
            "game": { "kind": "steam", "appId": 570 }, "gpuId": "GPU-123", "profileId": "gaming",
        } }),
        json!({ "type": "set_automatic_switching", "payload": { "enabled": true } }),
    ] {
        assert_ne!(
            alice.send_json(request.clone())["type"],
            "error",
            "{request}"
        );
    }
    apply(&daemon, shared_curve(&STARTER));
    daemon.run_steam_game(100, 1000, 1_000, 570);
    daemon.scan();
    drop(alice);

    daemon.restart();

    // Switching waits for the restarted daemon's first complete scan; the curve doesn't.
    daemon.set_temperature("GPU-123", Some(80));
    daemon.run_curves_for(500);
    assert_eq!(speeds(&daemon), vec![Some(90), Some(90)]);
    // A reconnecting session reads the controller's current state.
    let alice = daemon.user(1000, "alice");
    let performance = alice.send_json(json!({ "type": "get_performance" }));
    let reconnected = &performance["payload"]["gpus"][0]["cooling"]["control"];
    assert_eq!(reconnected["recovery"], resumed(&[0, 1]));
    assert_eq!(reconnected["requested"][0]["speedPercent"], 90);

    daemon.scan();
    daemon.set_temperature("GPU-123", Some(85));
    daemon.run_curves_for(500);
    assert_eq!(speeds(&daemon), vec![Some(100), Some(100)]);
}

#[test]
fn an_application_validates_fresh_readings_and_the_curve_before_any_write() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let rejected = |configuration: Value| {
        let application = apply(&daemon, configuration);
        assert_eq!(
            application["outcome"]["status"], "unattempted",
            "{application}"
        );
        application["outcome"]["error"]
            .as_str()
            .unwrap()
            .to_string()
    };

    // Short of the full-speed boundary.
    assert!(rejected(shared_curve(&[(45, 20), (85, 90)])).contains("must reach 100% by 85°C"));
    // Below the fans' supported minimum.
    assert!(rejected(shared_curve(&[(45, 10), (85, 100)])).contains("between 20% and 100%"));
    daemon.gpus.update("GPU-123", |gpu| {
        gpu.cooling.max_operating_temperature_c = None;
    });
    assert!(rejected(shared_curve(&STARTER)).contains("maximum operating temperature"));
    daemon.gpus.update("GPU-123", |gpu| {
        gpu.cooling.max_operating_temperature_c = Some(90);
    });
    daemon.set_temperature("GPU-123", None);
    assert!(rejected(shared_curve(&STARTER)).contains("temperature is unavailable"));
    daemon.set_temperature("GPU-123", Some(71));
    assert!(rejected(json!({
        "arrangement": "perFan",
        "fans": [{ "fanIndex": 2, "setting": { "mode": "automatic" } }],
    }))
    .contains("Fan 3 isn't reported"));
    let missing = daemon.send_json(json!({
        "type": "apply_cooling",
        "payload": { "gpuId": "GPU-999", "configuration": shared_curve(&STARTER) },
    }));
    assert_eq!(missing["payload"]["outcome"]["status"], "unattempted");

    assert!(daemon.gpus.writes().is_empty());
    assert_eq!(control(&daemon)["applied"], Value::Null);
}

#[test]
fn malformed_configurations_are_rejected_at_the_protocol_boundary() {
    let request = |configuration: Value| {
        serde_json::from_value::<crate::protocol::DaemonRequest>(json!({
            "type": "apply_cooling",
            "payload": { "gpuId": "GPU-123", "configuration": configuration },
        }))
    };
    assert!(request(shared_curve(&STARTER)).is_ok());
    assert!(request(json!({ "arrangement": "perFan", "fans": [{ "fanIndex": 0, "setting": { "mode": "fixed", "speedPercent": 50 } }] })).is_ok());
    for malformed in [
        json!({ "arrangement": "shared", "setting": { "mode": "curve", "points": points(&STARTER), "speedPercent": 50 } }),
        json!({ "arrangement": "shared", "setting": { "mode": "fixed", "speedPercent": 50.5 } }),
        json!({ "arrangement": "shared", "setting": { "mode": "turbo" } }),
        json!({ "arrangement": "shared", "fans": [] }),
        json!({ "arrangement": "perFan", "fans": [{ "fanIndex": -1, "setting": { "mode": "automatic" } }] }),
        json!({ "arrangement": "shared", "setting": { "mode": "automatic" }, "uid": 0 }),
    ] {
        assert!(request(malformed.clone()).is_err(), "{malformed}");
    }
}

#[test]
fn returning_to_automatic_cooling_stops_the_curve() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));

    let automatic = json!({ "arrangement": "shared", "setting": { "mode": "automatic" } });
    let application = apply(&daemon, automatic.clone());
    assert_eq!(application["outcome"]["status"], "succeeded");
    let writes = daemon.gpus.writes().len();
    daemon.set_temperature("GPU-123", Some(85));
    daemon.run_curves_for(1_000);

    assert_eq!(
        daemon.gpus.writes().len(),
        writes,
        "the curve wrote after it was replaced"
    );
    assert_eq!(fans(&daemon)[0].0, Some(FanControlMode::Auto));
    assert_eq!(
        control(&daemon),
        json!({ "revision": 2, "applied": automatic, "requested": [], "curveTemperatureC": null, "suspension": null, "recovery": null })
    );
}

#[test]
fn a_fixed_speed_replaces_the_curve_and_is_reported_as_fixed() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));

    let fixed =
        json!({ "arrangement": "shared", "setting": { "mode": "fixed", "speedPercent": 55 } });
    apply(&daemon, fixed.clone());
    daemon.set_temperature("GPU-123", Some(85));
    daemon.run_curves_for(1_000);

    assert_eq!(fans(&daemon), vec![manual(55), manual(55)]);
    assert_eq!(control(&daemon)["applied"], fixed);
    assert_eq!(control(&daemon)["requested"], json!([]));
}

#[test]
fn a_single_fan_change_takes_that_fan_off_the_curve_and_leaves_the_other_following_it() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));

    let response = daemon.send_json(json!({
        "type": "set_fan_mode_auto",
        "payload": { "gpuId": "GPU-123", "fanIndex": 0 },
    }));
    assert_eq!(
        response["payload"],
        json!({ "success": true, "error": null })
    );
    daemon.set_temperature("GPU-123", Some(80));
    daemon.run_curves_for(500);

    assert_eq!(
        fans(&daemon),
        vec![(Some(FanControlMode::Auto), Some(72)), manual(90)]
    );
    assert_eq!(
        control(&daemon)["applied"],
        json!({
            "arrangement": "perFan",
            "fans": [
                { "fanIndex": 0, "setting": { "mode": "automatic" } },
                { "fanIndex": 1, "setting": { "mode": "curve", "points": points(&STARTER) } },
            ],
        })
    );
    assert_eq!(
        control(&daemon)["requested"],
        json!([{ "fanIndex": 1, "speedPercent": 90 }])
    );
}

#[test]
fn curve_control_belongs_to_its_own_gpu() {
    let mut other = cooled_gpu();
    other.id = "GPU-456".into();
    other.stable_id = Some("GPU-456".into());
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu(), other]));
    apply(&daemon, shared_curve(&STARTER));

    daemon.set_temperature("GPU-123", Some(80));
    daemon.set_temperature("GPU-456", Some(80));
    daemon.run_curves_for(500);

    assert_eq!(speeds(&daemon), vec![Some(90), Some(90)]);
    assert!(daemon.gpus.writes().iter().all(|(id, _)| id == "GPU-123"));
    let performance = daemon.send_json(json!({ "type": "get_performance" }));
    assert_eq!(
        performance["payload"]["gpus"][1]["cooling"]["control"]["applied"],
        Value::Null
    );
}

fn restoration(fan_index: u32, status: &str, detail: Option<&str>) -> Value {
    json!({ "fanIndex": fan_index, "status": status, "detail": detail })
}

#[test]
fn a_missing_temperature_suspends_the_curve_keeps_it_and_restores_automatic_cooling() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));

    daemon.set_temperature("GPU-123", None);
    daemon.run_curves_for(500);

    assert_eq!(fans(&daemon)[0].0, Some(FanControlMode::Auto));
    assert_eq!(fans(&daemon)[1].0, Some(FanControlMode::Auto));
    assert_eq!(
        control(&daemon),
        json!({
            "revision": 2,
            "applied": shared_curve(&STARTER),
            "requested": [],
            "curveTemperatureC": null,
            "suspension": {
                "kind": "fault",
                "reason": "The GPU's temperature is unavailable, so Prism stopped following its curve.",
                "restoration": [restoration(0, "restored", None), restoration(1, "restored", None)],
            },
            "recovery": null,
        })
    );

    // The curve stays suspended when the temperature returns, until an explicit retry.
    let writes = daemon.gpus.writes().len();
    daemon.set_temperature("GPU-123", Some(80));
    daemon.run_curves_for(2_000);
    assert_eq!(daemon.gpus.writes().len(), writes);
    assert!(control(&daemon)["suspension"].is_object());
}

#[test]
fn a_failed_fan_write_suspends_the_curve_and_reports_each_fans_restoration_honestly() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));

    daemon
        .gpus
        .fail_fan("GPU-123", Setter::FanSpeed, 1, "Unknown error");
    daemon.gpus.fail_fan(
        "GPU-123",
        Setter::FanModeAuto,
        1,
        "Insufficient permissions",
    );
    daemon.gpus.hide_fan_mode("GPU-123", 0);
    daemon.set_temperature("GPU-123", Some(80));
    daemon.run_curves_for(500);

    let control = control(&daemon);
    assert_eq!(control["applied"], shared_curve(&STARTER));
    assert_eq!(control["requested"], json!([]));
    assert_eq!(
        control["suspension"],
        json!({
            "kind": "fault",
            "reason": "Fan 2 couldn't be set to 90%: Unknown error",
            "restoration": [
                restoration(0, "unverified", Some("The driver accepted the request but doesn't report the fan's control mode.")),
                restoration(1, "failed", Some("Insufficient permissions")),
            ],
        })
    );
}

#[test]
fn retrying_reapplies_the_kept_curve_from_the_current_temperature() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));
    daemon.set_temperature("GPU-123", None);
    daemon.run_curves_for(500);

    daemon.set_temperature("GPU-123", Some(80));
    let retried = apply(&daemon, control(&daemon)["applied"].clone());

    assert_eq!(retried["outcome"]["status"], "succeeded");
    assert_eq!(fans(&daemon), vec![manual(90), manual(90)]);
    assert_eq!(control(&daemon)["suspension"], Value::Null);
    assert_eq!(control(&daemon)["requested"][0]["speedPercent"], 90);
}

#[test]
fn a_failed_application_reports_the_fan_and_returns_the_curve_to_automatic_cooling() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    daemon
        .gpus
        .fail_fan("GPU-123", Setter::FanSpeed, 1, "Unknown error");

    let application = apply(&daemon, shared_curve(&STARTER));

    assert_eq!(application["outcome"]["status"], "failed");
    assert_eq!(
        application["fans"],
        json!([
            { "fanIndex": 0, "outcome": { "status": "succeeded", "error": null } },
            { "fanIndex": 1, "outcome": { "status": "failed", "error": "Unknown error" } },
        ])
    );
    assert_eq!(fans(&daemon)[0].0, Some(FanControlMode::Auto));
    let control = control(&daemon);
    assert_eq!(control["applied"], shared_curve(&STARTER));
    assert_eq!(control["suspension"]["reason"], "Fan 2: Unknown error");
    assert_eq!(
        control["suspension"]["restoration"][0],
        restoration(0, "restored", None)
    );
}

#[test]
fn any_failed_fan_write_suspends_the_gpus_curve_control() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));
    daemon
        .gpus
        .fail_fan("GPU-123", Setter::FanSpeed, 0, "Unknown error");

    let application = apply(
        &daemon,
        json!({ "arrangement": "shared", "setting": { "mode": "fixed", "speedPercent": 55 } }),
    );

    assert_eq!(application["fans"][1]["outcome"]["status"], "unattempted");
    // Fan 2 was left following the curve, which the failure suspended.
    assert_eq!(fans(&daemon)[1].0, Some(FanControlMode::Auto));
    let control = control(&daemon);
    assert_eq!(control["applied"]["arrangement"], "perFan");
    assert_eq!(
        control["suspension"]["restoration"],
        json!([
            restoration(0, "restored", None),
            restoration(1, "restored", None)
        ])
    );
}

#[test]
fn a_failed_write_without_a_curve_reports_no_suspension() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    daemon
        .gpus
        .fail("GPU-123", Setter::FanSpeed, "Unknown error");

    let application = apply(
        &daemon,
        json!({ "arrangement": "shared", "setting": { "mode": "fixed", "speedPercent": 55 } }),
    );

    assert_eq!(application["outcome"]["status"], "failed");
    assert_eq!(control(&daemon)["suspension"], Value::Null);
}

#[test]
fn stopping_the_daemon_returns_curve_fans_to_automatic_cooling_until_it_starts_again() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));

    daemon.stop_daemon();

    assert_eq!(fans(&daemon)[0].0, Some(FanControlMode::Auto));
    assert_eq!(fans(&daemon)[1].0, Some(FanControlMode::Auto));
}

fn resumed(fan_indexes: &[u32]) -> Value {
    let succeeded = json!({ "status": "succeeded", "error": null });
    json!({
        "status": "resumed",
        "fans": fan_indexes
            .iter()
            .map(|index| json!({ "fanIndex": index, "outcome": succeeded }))
            .collect::<Vec<_>>(),
    })
}

#[test]
fn a_same_boot_restart_resumes_a_healthy_curve_from_the_fresh_temperature() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));
    assert_eq!(speeds(&daemon), vec![Some(72), Some(72)]);

    daemon.stop_daemon();
    // Cooler now: the first output follows 55°C at once, not the previous 72% sample nor its
    // five-second decrease delay.
    daemon.set_temperature("GPU-123", Some(55));
    daemon.start_daemon();

    assert_eq!(fans(&daemon), vec![manual(40), manual(40)]);
    assert_eq!(
        control(&daemon),
        json!({
            "revision": 3,
            "applied": shared_curve(&STARTER),
            "requested": [
                { "fanIndex": 0, "speedPercent": 40 },
                { "fanIndex": 1, "speedPercent": 40 },
            ],
            "curveTemperatureC": 55,
            "suspension": null,
            "recovery": resumed(&[0, 1]),
        })
    );

    // The resumed curve keeps following the temperature.
    daemon.set_temperature("GPU-123", Some(80));
    daemon.run_curves_for(500);
    assert_eq!(speeds(&daemon), vec![Some(90), Some(90)]);
}

fn not_resumed(fan_indexes: &[u32], error: &str) -> Value {
    json!({
        "status": "retryRequired",
        "fans": fan_indexes
            .iter()
            .map(|index| json!({ "fanIndex": index, "outcome": { "status": "unattempted", "error": error } }))
            .collect::<Vec<_>>(),
    })
}

#[test]
fn a_restart_that_cant_validate_the_curve_returns_to_automatic_cooling_until_a_retry() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));

    daemon.stop_daemon();
    daemon.set_temperature("GPU-123", None);
    let writes = daemon.gpus.writes().len();
    daemon.start_daemon();

    let unavailable = "The GPU's temperature is unavailable, so a curve can't be followed. Refresh the GPU readings and try again.";
    assert!(daemon.gpus.writes()[writes..]
        .iter()
        .all(|(_, setter)| *setter == Setter::FanModeAuto));
    assert_eq!(fans(&daemon)[0].0, Some(FanControlMode::Auto));
    assert_eq!(fans(&daemon)[1].0, Some(FanControlMode::Auto));
    assert_eq!(
        control(&daemon),
        json!({
            "revision": 3,
            "applied": shared_curve(&STARTER),
            "requested": [],
            "curveTemperatureC": null,
            "suspension": {
                "kind": "fault",
                "reason": format!("After the Prism daemon restarted, the curve couldn't resume. {unavailable}"),
                "restoration": [restoration(0, "restored", None), restoration(1, "restored", None)],
            },
            "recovery": not_resumed(&[0, 1], unavailable),
        })
    );

    // A returning temperature doesn't resume it, and neither does another restart.
    daemon.set_temperature("GPU-123", Some(80));
    daemon.run_curves_for(1_000);
    daemon.restart();
    assert_eq!(control(&daemon)["requested"], json!([]));
    assert_eq!(control(&daemon)["recovery"]["status"], "retryRequired");

    // An explicit retry does, from the current temperature.
    let retried = apply(&daemon, control(&daemon)["applied"].clone());
    assert_eq!(retried["outcome"]["status"], "succeeded");
    assert_eq!(fans(&daemon), vec![manual(90), manual(90)]);
    assert_eq!(control(&daemon)["suspension"], Value::Null);
    assert_eq!(control(&daemon)["recovery"], Value::Null);
}

#[test]
fn a_fan_write_failing_on_resumption_reports_each_fan_and_returns_to_automatic_cooling() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));

    daemon.stop_daemon();
    daemon
        .gpus
        .fail_fan("GPU-123", Setter::FanSpeed, 0, "Unknown error");
    daemon.gpus.fail_fan(
        "GPU-123",
        Setter::FanModeAuto,
        0,
        "Insufficient permissions",
    );
    daemon.start_daemon();

    let control = control(&daemon);
    assert_eq!(
        control["recovery"],
        json!({
            "status": "retryRequired",
            "fans": [
                { "fanIndex": 0, "outcome": { "status": "failed", "error": "Unknown error" } },
                { "fanIndex": 1, "outcome": { "status": "unattempted", "error": "An earlier fan failed. Retry the curve." } },
            ],
        })
    );
    assert_eq!(
        control["suspension"],
        json!({
            "kind": "fault",
            "reason": "After the Prism daemon restarted, the curve couldn't resume. Fan 1: Unknown error",
            "restoration": [
                restoration(0, "failed", Some("Insufficient permissions")),
                restoration(1, "restored", None),
            ],
        })
    );
    assert_eq!(control["requested"], json!([]));
}

#[test]
fn a_faulted_curve_stays_suspended_across_a_restart() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));
    daemon.set_temperature("GPU-123", None);
    daemon.run_curves_for(500);
    let suspension = control(&daemon)["suspension"].clone();
    daemon.set_temperature("GPU-123", Some(80));
    let writes = daemon.gpus.writes().len();

    daemon.restart();
    daemon.run_curves_for(1_000);

    assert_eq!(
        daemon.gpus.writes().len(),
        writes,
        "a faulted curve resumed"
    );
    assert_eq!(control(&daemon)["suspension"], suspension);
    assert_eq!(control(&daemon)["recovery"], Value::Null);
}

#[test]
fn a_curve_resumes_only_on_fans_that_were_following_it() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(
        &daemon,
        json!({ "arrangement": "perFan", "fans": [
            { "fanIndex": 0, "setting": { "mode": "curve", "points": points(&STARTER) } },
            { "fanIndex": 1, "setting": { "mode": "curve", "points": points(&STARTER) } },
        ] }),
    );
    daemon.set_temperature("GPU-123", None);
    daemon.run_curves_for(500);
    daemon.set_temperature("GPU-123", Some(80));
    // Fan 1's curve is applied again; fan 2's stays suspended.
    apply(
        &daemon,
        json!({ "arrangement": "perFan", "fans": [
            { "fanIndex": 0, "setting": { "mode": "curve", "points": points(&STARTER) } },
        ] }),
    );
    let suspension = control(&daemon)["suspension"].clone();
    assert!(suspension.is_object());

    daemon.restart();

    assert_eq!(fans(&daemon)[0], manual(90));
    assert_eq!(fans(&daemon)[1].0, Some(FanControlMode::Auto));
    assert_eq!(control(&daemon)["recovery"], resumed(&[0]));
    assert_eq!(control(&daemon)["suspension"], suspension);
}

#[test]
fn a_fan_taken_over_while_the_daemon_was_stopped_is_left_alone() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));

    daemon.stop_daemon();
    daemon.gpus.update("GPU-123", |gpu| {
        gpu.cooling.fans[1].mode = Some(FanControlMode::Manual);
        gpu.cooling.fans[1].current_speed_percent = Some(65);
    });
    let writes = daemon.gpus.writes().len();
    daemon.start_daemon();

    assert_eq!(daemon.gpus.writes().len(), writes);
    let reason = "While the Prism daemon was stopped, fan 2 came under manual control from elsewhere. Prism left those controls alone.";
    assert_eq!(
        control(&daemon)["suspension"],
        json!({ "kind": "externalControl", "reason": reason, "restoration": [] })
    );
    assert_eq!(control(&daemon)["recovery"], not_resumed(&[0, 1], reason));
}

#[test]
fn a_restart_doesnt_resume_a_curve_on_changed_fans() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));

    daemon.stop_daemon();
    daemon.gpus.update("GPU-123", |gpu| {
        gpu.cooling.fans.pop();
        gpu.cooling.fan_count = 1;
    });
    daemon.start_daemon();

    let changed = "The GPU's fans changed since the curve was applied.";
    assert_eq!(control(&daemon)["recovery"], not_resumed(&[0, 1], changed));
    assert_eq!(control(&daemon)["requested"], json!([]));
    assert_eq!(fans(&daemon)[0].0, Some(FanControlMode::Auto));
}

#[test]
fn a_restart_doesnt_resume_a_curve_the_gpus_limits_no_longer_allow() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));

    daemon.stop_daemon();
    daemon.gpus.update("GPU-123", |gpu| {
        gpu.cooling.max_operating_temperature_c = None
    });
    daemon.start_daemon();

    assert_eq!(control(&daemon)["recovery"]["status"], "retryRequired");
    assert_eq!(control(&daemon)["requested"], json!([]));
    assert_eq!(fans(&daemon)[0].0, Some(FanControlMode::Auto));
}

#[test]
fn a_new_boot_id_never_resumes_the_previous_boots_curve() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));
    daemon.stop_daemon();
    let writes = daemon.gpus.writes().len();

    daemon.reboot_with_retained_boot_state();

    assert_eq!(daemon.gpus.writes().len(), writes);
    assert_eq!(control(&daemon)["applied"], Value::Null);
    assert_eq!(control(&daemon)["recovery"], Value::Null);
}

#[test]
fn an_unreadable_record_resumes_nothing() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));
    daemon.stop_daemon();
    let path = daemon.boot_path("cooling-control.json");
    let mut record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record["gpus"]["GPU-123"]["interrupted"]["unexpected"] = json!(true);
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let writes = daemon.gpus.writes().len();

    daemon.start_daemon();

    assert_eq!(daemon.gpus.writes().len(), writes);
    assert_eq!(control(&daemon)["applied"], Value::Null);
}

#[test]
fn a_recorded_fan_without_a_curve_is_not_resumed_as_one() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));
    daemon.stop_daemon();
    let path = daemon.boot_path("cooling-control.json");
    let mut record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record["gpus"]["GPU-123"]["configuration"] =
        json!({ "arrangement": "shared", "setting": { "mode": "fixed", "speedPercent": 55 } });
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();

    daemon.start_daemon();

    assert_eq!(control(&daemon)["recovery"]["status"], "retryRequired");
    assert_eq!(control(&daemon)["requested"], json!([]));
    assert_eq!(fans(&daemon)[0].0, Some(FanControlMode::Auto));
}

#[test]
fn after_an_abnormal_termination_the_next_daemon_restores_abandoned_fans_before_resuming() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));
    daemon.gpus.fail_fan(
        "GPU-123",
        Setter::FanModeAuto,
        1,
        "Insufficient permissions",
    );
    let writes = daemon.gpus.writes().len();

    daemon.set_temperature("GPU-123", Some(80));
    daemon.crash_daemon();

    let fan_writes: Vec<_> = daemon.gpus.writes()[writes..]
        .iter()
        .map(|(_, setter)| *setter)
        .collect();
    assert_eq!(
        fan_writes,
        vec![
            Setter::FanModeAuto,
            Setter::FanModeAuto,
            Setter::FanSpeed,
            Setter::FanSpeed
        ],
        "the abandoned fans weren't returned to automatic cooling first"
    );
    assert_eq!(fans(&daemon), vec![manual(90), manual(90)]);
    assert_eq!(control(&daemon)["recovery"], resumed(&[0, 1]));
    assert_eq!(control(&daemon)["suspension"], Value::Null);
}

#[test]
fn recovering_abandoned_curves_needs_neither_the_daemon_nor_its_curve_loop() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));

    // What the service manager runs once the daemon process has gone.
    super::cooling::recover_abandoned_curves(daemon.gpus.as_ref(), &daemon.boot_path(""), "boot-1");
    assert_eq!(fans(&daemon)[0].0, Some(FanControlMode::Auto));
    assert_eq!(fans(&daemon)[1].0, Some(FanControlMode::Auto));
    let writes = daemon.gpus.writes().len();

    daemon.crash_daemon();
    assert!(
        daemon.gpus.writes()[writes..]
            .iter()
            .all(|(_, setter)| *setter == Setter::FanSpeed),
        "the fans were restored twice"
    );
    assert_eq!(control(&daemon)["recovery"], resumed(&[0, 1]));
}

#[test]
fn a_reboot_forgets_the_previous_boots_curve() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));

    daemon.crash_and_reboot();

    assert_eq!(control(&daemon)["applied"], Value::Null);
}

#[test]
fn a_stalled_curve_is_reported_to_the_supervisor_outside_the_loop() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let core = daemon.core();
    assert!(core.cooling_is_live(1_000), "no curve, nothing to stall");
    apply(&daemon, shared_curve(&STARTER));
    daemon.run_curves_for(1_000);
    assert!(core.cooling_is_live(1_000));

    let gate = daemon.gpus.block("GPU-123", Setter::FanSpeed);
    daemon.set_temperature("GPU-123", Some(80));
    daemon.clock.advance(500);
    core.follow_curves();
    gate.wait_until_entered();
    for _ in 0..4 {
        daemon.clock.advance(500);
        core.follow_curves();
    }
    assert!(!core.cooling_is_live(1_000), "the stalled GPU looked live");
    gate.release();
    daemon.settle();

    // A curve loop that stops running is a stall too.
    daemon.clock.advance(1_500);
    assert!(!core.cooling_is_live(1_000));
    core.follow_curves();
    daemon.settle();
    assert!(core.cooling_is_live(1_000));
}

#[test]
fn curve_output_changes_are_not_tuning_changes() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let session = daemon.user(1000, "alice");
    apply(&daemon, shared_curve(&STARTER));
    let revision = |session: &super::harness::FakeSession| {
        session.send_json(json!({ "type": "get_tuning_states" }))["payload"][0]["revision"].clone()
    };
    let before = revision(&session);
    session.pushed_events();

    daemon.set_temperature("GPU-123", Some(80));
    daemon.run_curves_for(1_000);

    assert_eq!(speeds(&daemon), vec![Some(90), Some(90)]);
    assert_eq!(revision(&session), before);
    assert!(session
        .pushed_events()
        .iter()
        .all(|event| event["type"] != "tuningStateChanged"));
    let startup = session.send_json(json!({ "type": "get_startup_state" }));
    assert_eq!(startup["payload"]["possiblyStale"], json!([]));
    assert!(daemon
        .persistent_file("tuning-state.json")
        .is_none_or(|state| !String::from_utf8_lossy(&state).contains("speed")));
}

/// The same payloads hold the desktop boundary and the fake desktop to the daemon.
#[test]
fn cooling_payloads_round_trip_the_shared_contract() {
    use crate::cooling::{ApplyCoolingRequest, CoolingApplication, CoolingControl};
    let contract: Value = serde_json::from_str(include_str!(
        "../../../src/test/contracts/cooling-application.json"
    ))
    .unwrap();
    let request: ApplyCoolingRequest = serde_json::from_value(contract["request"].clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), contract["request"]);
    let application: CoolingApplication =
        serde_json::from_value(contract["application"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(application).unwrap(),
        contract["application"]
    );
    for control in contract["controls"].as_array().unwrap() {
        let typed: CoolingControl = serde_json::from_value(control.clone()).unwrap();
        assert_eq!(&serde_json::to_value(typed).unwrap(), control);
    }
}

/// The fake desktop computes requested speeds from the same contract.
#[test]
fn applied_curves_request_the_contracts_speeds() {
    let contract: Value = serde_json::from_str(include_str!(
        "../../../src/test/contracts/cooling-application.json"
    ))
    .unwrap();
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    for pair in contract["requestedAtTemperature"].as_array().unwrap() {
        daemon.set_temperature("GPU-123", Some(pair[0].as_u64().unwrap() as u32));
        apply(&daemon, contract["request"]["configuration"].clone());
        assert_eq!(
            control(&daemon)["requested"][0]["speedPercent"],
            pair[1],
            "{pair}"
        );
    }
}

#[test]
fn a_stop_that_cant_wait_for_a_stuck_curve_write_leaves_its_fans_to_recovery() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));
    let gate = daemon.gpus.block("GPU-123", Setter::FanSpeed);
    daemon.set_temperature("GPU-123", Some(80));
    daemon.clock.advance(500);
    let stuck = daemon.core();
    stuck.follow_curves();
    gate.wait_until_entered();

    stuck.stop();
    assert_eq!(fans(&daemon)[1].0, Some(FanControlMode::Auto));

    // The hung write lands after the stop restored the fans.
    gate.release();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while fans(&daemon)[0] != manual(90) {
        assert!(
            std::time::Instant::now() < deadline,
            "the hung write never landed"
        );
        std::thread::yield_now();
    }
    assert_eq!(
        control(&daemon)["requested"],
        json!([]),
        "the stopped curve came back"
    );

    // Whoever runs next returns the fan the late write took, and doesn't resume a stalled curve.
    daemon.crash_daemon();
    assert_eq!(fans(&daemon)[0].0, Some(FanControlMode::Auto));
    assert_eq!(
        control(&daemon)["suspension"],
        json!({
            "kind": "fault",
            "reason": STALLED,
            "restoration": [restoration(0, "restored", None), restoration(1, "restored", None)],
        })
    );
    assert_eq!(control(&daemon)["requested"], json!([]));
    assert_eq!(control(&daemon)["recovery"], Value::Null);
}

const STALLED: &str =
    "Prism's fan curve control stopped responding, so the daemon stopped following this curve.";

#[test]
fn a_curve_whose_control_stalled_is_not_resumed_after_the_watchdog_stops_the_daemon() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));

    // The supervisor outside the curve loop saw no progress, then the watchdog killed the daemon.
    daemon.core().record_cooling_stall(true);
    daemon.crash_daemon();

    assert_eq!(fans(&daemon)[0].0, Some(FanControlMode::Auto));
    assert_eq!(control(&daemon)["suspension"]["reason"], STALLED);
    assert_eq!(control(&daemon)["requested"], json!([]));
}

#[test]
fn a_stall_that_cleared_before_the_daemon_stopped_doesnt_block_resumption() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));
    daemon.core().record_cooling_stall(true);
    daemon.core().record_cooling_stall(false);

    daemon.crash_daemon();

    assert_eq!(control(&daemon)["recovery"], resumed(&[0, 1]));
}

#[test]
fn a_write_hanging_mid_application_is_a_stall_with_its_fans_already_recorded() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let core = daemon.core();
    let gate = daemon.gpus.block("GPU-123", Setter::FanSpeed);
    let application = daemon.send_in_background(
        serde_json::from_value(json!({
            "type": "apply_cooling",
            "payload": { "gpuId": "GPU-123", "configuration": shared_curve(&STARTER) },
        }))
        .unwrap(),
    );
    gate.wait_until_entered();

    let record: Value =
        serde_json::from_slice(&std::fs::read(daemon.boot_path("cooling-control.json")).unwrap())
            .unwrap();
    assert_eq!(record["gpus"]["GPU-123"]["curveFans"], json!([0, 1]));
    daemon.clock.advance(2_000);
    assert!(!core.cooling_is_live(1_000));

    gate.release();
    application.response();
    core.follow_curves();
    daemon.settle();
    assert!(core.cooling_is_live(1_000));
}

#[test]
fn a_failed_fixed_command_keeps_the_previous_applied_configuration() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(
        &daemon,
        json!({ "arrangement": "shared", "setting": { "mode": "fixed", "speedPercent": 40 } }),
    );
    daemon
        .gpus
        .fail_fan("GPU-123", Setter::FanSpeed, 1, "Rejected");
    let result = apply(
        &daemon,
        json!({ "arrangement": "perFan", "fans": [
        { "fanIndex": 0, "setting": { "mode": "automatic" } },
        { "fanIndex": 1, "setting": { "mode": "fixed", "speedPercent": 60 } }
    ] }),
    );
    assert_eq!(result["fans"][0]["outcome"]["status"], "succeeded");
    assert_eq!(result["fans"][1]["outcome"]["status"], "failed");
    assert_eq!(
        control(&daemon)["applied"],
        json!({ "arrangement": "perFan", "fans": [
        { "fanIndex": 0, "setting": { "mode": "automatic" } },
        { "fanIndex": 1, "setting": { "mode": "fixed", "speedPercent": 40 } }
    ] })
    );
}

#[test]
fn separate_fans_preview_and_apply_against_their_own_limits_and_identities() {
    let mut gpu = cooled_gpu();
    gpu.cooling.fans[0].index = 7;
    gpu.cooling.fans[0].max_speed_percent = Some(80);
    gpu.cooling.fans.reverse();
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![gpu]));
    let curve = points(&[(45, 20), (85, 80)]);
    let preview = daemon.send_json(json!({ "type": "preview_fan_curve", "payload": {
        "gpuId": "GPU-123", "fanIndex": 7, "points": curve
    }}));
    assert_eq!(preview["payload"]["problems"], json!([]), "{preview}");
    assert_eq!(preview["payload"]["support"]["maxSpeedPercent"], 80);
    let result = apply(
        &daemon,
        json!({ "arrangement": "perFan", "fans": [
        { "fanIndex": 7, "setting": { "mode": "curve", "points": curve } },
        { "fanIndex": 1, "setting": { "mode": "fixed", "speedPercent": 55 } }
    ] }),
    );
    assert_eq!(result["outcome"]["status"], "succeeded", "{result}");
    assert_eq!(fans(&daemon), vec![manual(55), manual(59)]);
    daemon.set_temperature("GPU-123", Some(85));
    daemon.run_curves_for(500);
    assert_eq!(fans(&daemon), vec![manual(55), manual(80)]);
}

#[test]
fn shared_applications_require_the_reviewed_fan_inventory() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let request = |indexes: Value| {
        daemon.send_json(json!({ "type": "apply_cooling", "payload": {
        "gpuId": "GPU-123", "fanIndexes": indexes,
        "configuration": { "arrangement": "shared", "setting": { "mode": "fixed", "speedPercent": 55 } }
    }}))
    };
    for indexes in [json!([0]), json!([0, 7]), json!([0, 0, 1])] {
        let result = request(indexes);
        assert_eq!(
            result["payload"]["outcome"]["status"], "unattempted",
            "{result}"
        );
        assert!(control(&daemon)["applied"].is_null());
    }
    let result = request(json!([1, 0]));
    assert_eq!(result["payload"]["outcome"]["status"], "succeeded");
    assert_eq!(fans(&daemon), vec![manual(55), manual(55)]);
}

#[test]
fn mixed_modes_keep_success_failed_and_unattempted_fans_distinct() {
    let mut gpu = cooled_gpu();
    let mut third = gpu.cooling.fans[0].clone();
    third.index = 2;
    gpu.cooling.fans.push(third);
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![gpu]));
    apply(
        &daemon,
        json!({ "arrangement": "shared", "setting": { "mode": "fixed", "speedPercent": 40 } }),
    );
    daemon
        .gpus
        .fail_fan("GPU-123", Setter::FanSpeed, 1, "Rejected");
    let result = apply(
        &daemon,
        json!({ "arrangement": "perFan", "fans": [
        { "fanIndex": 0, "setting": { "mode": "curve", "points": points(&STARTER) } },
        { "fanIndex": 1, "setting": { "mode": "fixed", "speedPercent": 55 } },
        { "fanIndex": 2, "setting": { "mode": "automatic" } }
    ] }),
    );
    assert_eq!(result["fans"][0]["outcome"]["status"], "succeeded");
    assert_eq!(result["fans"][1]["outcome"]["status"], "failed");
    assert_eq!(result["fans"][2]["outcome"]["status"], "unattempted");
    let state = control(&daemon);
    assert_eq!(
        state["applied"]["fans"][1]["setting"],
        json!({ "mode": "fixed", "speedPercent": 40 })
    );
    assert_eq!(
        state["applied"]["fans"][2]["setting"],
        json!({ "mode": "fixed", "speedPercent": 40 })
    );
    assert_eq!(
        state["suspension"]["restoration"],
        json!([restoration(0, "restored", None)])
    );
    assert_eq!(fans(&daemon)[0].0, Some(FanControlMode::Auto));
    daemon.set_temperature("GPU-123", Some(85));
    daemon.run_curves_for(500);
    assert_eq!(control(&daemon)["requested"], json!([]));
}

#[test]
fn another_sessions_cooling_change_rejects_a_stale_draft_without_writing() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let first = daemon.user(1000, "alice");
    let second = daemon.user(1000, "alice");
    let revision = control(&daemon)["revision"].clone();
    let request = json!({ "type": "apply_cooling", "payload": {
        "gpuId": "GPU-123", "configuration": shared_curve(&STARTER),
        "expectedRevision": revision,
    }});
    assert_eq!(
        first.send_json(request.clone())["payload"]["outcome"]["status"],
        "succeeded"
    );
    let writes = daemon.gpus.writes();
    let stale = second.send_json(request);
    assert_eq!(stale["payload"]["outcome"]["status"], "unattempted");
    assert!(stale["payload"]["outcome"]["error"]
        .as_str()
        .unwrap()
        .contains("Review"));
    assert_eq!(daemon.gpus.writes(), writes);
    let applied_revision = control(&daemon)["revision"].clone();
    daemon.set_temperature("GPU-123", Some(80));
    daemon.run_curves_for(500);
    assert_eq!(control(&daemon)["revision"], applied_revision);
}

#[test]
fn an_observable_external_mode_change_yields_without_restoring_or_resuming() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    apply(&daemon, shared_curve(&STARTER));
    let revision = control(&daemon)["revision"].as_u64().unwrap();
    daemon.gpus.update("GPU-123", |gpu| {
        gpu.cooling.fans[0].mode = Some(FanControlMode::Auto);
        gpu.cooling.fans[0].current_speed_percent = Some(33);
    });
    let writes = daemon.gpus.writes();
    // External ownership takes priority even when the sensor is also unavailable.
    daemon.set_temperature("GPU-123", None);
    daemon.run_curves_for(500);
    assert_eq!(daemon.gpus.writes(), writes);
    assert_eq!(control(&daemon)["suspension"]["kind"], "externalControl");
    assert_eq!(control(&daemon)["suspension"]["restoration"], json!([]));
    assert_eq!(control(&daemon)["applied"], shared_curve(&STARTER));
    assert_eq!(control(&daemon)["requested"], json!([]));
    assert_eq!(control(&daemon)["revision"], revision + 1);
    daemon.set_temperature("GPU-123", Some(80));
    daemon.run_curves_for(10_000);
    daemon.restart();
    assert_eq!(daemon.gpus.writes(), writes);
    assert_eq!(control(&daemon)["suspension"]["kind"], "externalControl");
    assert_eq!(
        apply(&daemon, shared_curve(&STARTER))["outcome"]["status"],
        "succeeded"
    );
    assert_eq!(speeds(&daemon), vec![Some(90), Some(90)]);
}

#[test]
fn queued_applications_check_the_revision_after_the_preceding_write_finishes() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let first = daemon.user(1000, "alice");
    let second = daemon.user(1001, "bob");
    let request = json!({ "type": "apply_cooling", "payload": {
        "gpuId": "GPU-123", "configuration": shared_curve(&STARTER), "expectedRevision": 0,
    }});
    let gate = daemon.gpus.block("GPU-123", Setter::FanSpeed);
    let first_request = request.clone();
    let first = std::thread::spawn(move || first.send_json(first_request));
    gate.wait_until_entered();
    assert_eq!(control(&daemon)["revision"], 0);
    let second = std::thread::spawn(move || second.send_json(request));
    gate.release();
    assert_eq!(
        first.join().unwrap()["payload"]["outcome"]["status"],
        "succeeded"
    );
    assert_eq!(
        second.join().unwrap()["payload"]["outcome"]["status"],
        "unattempted"
    );
    assert_eq!(daemon.gpus.writes().len(), 2);
    assert_eq!(control(&daemon)["revision"], 1);
}

#[test]
fn driver_observations_do_not_claim_ownership_or_report_false_external_changes() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let _session = daemon.user(1000, "alice");
    daemon.set_temperature("GPU-123", Some(60));
    daemon.run_curves_for(500);
    assert_eq!(control(&daemon)["revision"], 0);
    assert!(daemon.gpus.writes().is_empty());
    apply(&daemon, shared_curve(&STARTER));
    daemon.gpus.update("GPU-123", |gpu| {
        gpu.cooling.fans[0].current_speed_percent = Some(25);
        gpu.cooling.fans[1].mode = None;
    });
    daemon.set_temperature("GPU-123", Some(80));
    daemon.run_curves_for(500);
    assert_eq!(control(&daemon)["revision"], 1);
    assert_eq!(control(&daemon)["suspension"], Value::Null);
    assert_eq!(speeds(&daemon), vec![Some(90), Some(90)]);
}

#[test]
fn changing_a_fixed_fans_policy_yields_only_its_gpus_curve() {
    let mut other = cooled_gpu();
    other.id = "GPU-456".into();
    other.stable_id = Some("GPU-456".into());
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu(), other]));
    apply(
        &daemon,
        json!({ "arrangement": "perFan", "fans": [
        { "fanIndex": 0, "setting": { "mode": "fixed", "speedPercent": 55 } },
        { "fanIndex": 1, "setting": { "mode": "curve", "points": points(&STARTER) } },
    ] }),
    );
    daemon.send_json(json!({ "type": "apply_cooling", "payload": { "gpuId": "GPU-456", "configuration": shared_curve(&STARTER) } }));
    daemon.gpus.update("GPU-123", |gpu| {
        gpu.cooling.fans[0].mode = Some(FanControlMode::Auto)
    });
    let writes = daemon.gpus.writes().len();
    daemon.set_temperature("GPU-123", Some(80));
    daemon.set_temperature("GPU-456", Some(80));
    daemon.run_curves_for(500);
    assert_eq!(control(&daemon)["suspension"]["kind"], "externalControl");
    assert!(daemon.gpus.writes()[writes..]
        .iter()
        .all(|(uuid, _)| uuid == "GPU-456"));
    assert_eq!(
        daemon.gpus.gpu("GPU-456").cooling.fans[0].current_speed_percent,
        Some(90)
    );
}
