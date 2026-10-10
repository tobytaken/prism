//! Seam 1: previewing shared fan curves against the capabilities the GPU reports.

use serde_json::{json, Value};

use super::harness::{cooled_gpu, FakeGpuBackend, Harness};

fn points(points: &[(i64, i64)]) -> Value {
    points
        .iter()
        .map(|(temperature, speed)| json!({ "temperatureC": temperature, "speedPercent": speed }))
        .collect()
}

/// The GPU's shared curve support as the daemon reports it in performance readings.
fn curve_support(daemon: &Harness, gpu_id: &str) -> Value {
    let response = daemon.send_json(json!({ "type": "get_performance" }));
    response["payload"]["gpus"]
        .as_array()
        .unwrap()
        .iter()
        .find(|gpu| gpu["id"] == gpu_id)
        .unwrap_or_else(|| panic!("no GPU {gpu_id} in {response}"))["cooling"]["curveSupport"]
        .clone()
}

#[test]
fn performance_offers_a_starter_curve_within_the_gpus_limits() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));

    assert_eq!(
        curve_support(&daemon, "GPU-123"),
        json!({
            "status": "available",
            "maxOperatingTemperatureC": 90,
            "fullSpeedTemperatureC": 85,
            "minSpeedPercent": 20,
            "maxSpeedPercent": 100,
            "starter": points(&[(45, 20), (65, 60), (75, 80), (85, 100)]),
        })
    );
    assert!(daemon.gpus.writes().is_empty());
}

fn unavailable_reason(daemon: &Harness) -> String {
    let support = curve_support(daemon, "GPU-123");
    assert_eq!(support["status"], "unavailable", "{support}");
    support["reason"].as_str().unwrap().to_string()
}

#[test]
fn curves_are_unavailable_with_a_reason_rather_than_guessing_limits() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));

    daemon.gpus.update("GPU-123", |gpu| {
        gpu.cooling.fans[1].min_speed_percent = None;
        gpu.cooling.fans[1].max_speed_percent = None;
    });
    assert_eq!(
        unavailable_reason(&daemon),
        "Fan 2 doesn't report its supported speed range."
    );

    daemon.gpus.update("GPU-123", |gpu| {
        gpu.cooling.fans[1].min_speed_percent = Some(20);
        gpu.cooling.fans[1].max_speed_percent = Some(90);
    });
    assert!(unavailable_reason(&daemon).contains("different maximum speeds"));

    daemon.gpus.update("GPU-123", |gpu| {
        gpu.cooling.fans[1].min_speed_percent = Some(80);
        gpu.cooling.fans[1].max_speed_percent = Some(30);
    });
    assert_eq!(
        unavailable_reason(&daemon),
        "Fan 2 reports an invalid speed range (80-30%)."
    );

    daemon.gpus.update("GPU-123", |gpu| {
        gpu.cooling.fans[1].min_speed_percent = Some(20);
        gpu.cooling.fans[1].max_speed_percent = Some(100);
        gpu.cooling.max_operating_temperature_c = None;
        gpu.cooling.max_operating_temperature_reason = Some(
            "This GPU reports its thermal limit only as a T.Limit margin, not as a temperature."
                .into(),
        );
    });
    assert_eq!(
        unavailable_reason(&daemon),
        "Fan curves need the GPU's maximum operating temperature. This GPU reports its thermal limit only as a T.Limit margin, not as a temperature."
    );

    daemon.gpus.update("GPU-123", |gpu| {
        gpu.cooling.is_supported = false;
        gpu.cooling.unsupported_reason =
            Some("Fan control is not exposed by this GPU or driver.".into());
        gpu.cooling.fans.clear();
        gpu.cooling.fan_count = 0;
    });
    assert_eq!(
        unavailable_reason(&daemon),
        "Fan curves need controllable fans. Fan control is not exposed by this GPU or driver."
    );
    assert!(daemon.gpus.writes().is_empty());
}

#[test]
fn the_starter_curve_tracks_the_reported_thermal_boundary() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    daemon.gpus.update("GPU-123", |gpu| {
        gpu.cooling.max_operating_temperature_c = Some(105);
    });
    let support = curve_support(&daemon, "GPU-123");
    assert_eq!(support["fullSpeedTemperatureC"], 100);
    assert_eq!(
        support["starter"],
        points(&[(60, 20), (80, 60), (90, 80), (100, 100)])
    );
}

fn preview(daemon: &Harness, gpu_id: &str, curve: &[(i64, i64)]) -> Value {
    daemon.send_json(json!({
        "type": "preview_fan_curve",
        "payload": { "gpuId": gpu_id, "points": points(curve) },
    }))
}

/// The preview's problems as (zero-based point, message) pairs.
fn problems(response: Value) -> Vec<(Option<u64>, String)> {
    assert_eq!(response["type"], "fan_curve_preview", "{response}");
    response["payload"]["problems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|problem| {
            (
                problem["point"].as_u64(),
                problem["message"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn previewing_the_starter_curve_finds_no_problems_and_writes_nothing() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    let starter = [(45, 20), (65, 60), (75, 80), (85, 100)];

    assert_eq!(problems(preview(&daemon, "GPU-123", &starter)), vec![]);
    // Reaching the maximum early, or keeping it past the boundary, is fine.
    assert_eq!(
        problems(preview(
            &daemon,
            "GPU-123",
            &[(30, 20), (70, 100), (90, 100)]
        )),
        vec![]
    );
    assert!(daemon.gpus.writes().is_empty());
    assert_eq!(daemon.gpus.gpu("GPU-123"), cooled_gpu());
}

#[test]
fn preview_reports_why_curves_are_unavailable() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));
    daemon.gpus.update("GPU-123", |gpu| {
        gpu.cooling.fans[0].max_speed_percent = None;
    });

    assert_eq!(
        problems(preview(&daemon, "GPU-123", &[(45, 20), (85, 100)])),
        vec![(
            None,
            "Fan 1 doesn't report its supported speed range.".into()
        )]
    );
    let missing = preview(&daemon, "GPU-999", &[(45, 20), (85, 100)]);
    assert_eq!(missing["type"], "error", "{missing}");
    assert_eq!(missing["payload"]["code"], "notFound");
    assert!(daemon.gpus.writes().is_empty());
}

#[test]
fn preview_rejects_fractional_and_unknown_point_fields() {
    let request = |point: Value| {
        serde_json::from_value::<crate::protocol::DaemonRequest>(json!({
            "type": "preview_fan_curve",
            "payload": { "gpuId": "GPU-123", "points": [point, { "temperatureC": 85, "speedPercent": 100 }] },
        }))
    };

    assert!(request(json!({ "temperatureC": 45, "speedPercent": 20 })).is_ok());
    assert!(request(json!({ "temperatureC": 45.5, "speedPercent": 20 })).is_err());
    assert!(request(json!({ "temperatureC": 45, "speedPercent": "20" })).is_err());
    assert!(request(json!({ "temperatureC": 45, "speedPercent": 20, "rpm": 900 })).is_err());
}

/// The same contract holds the fake desktop to the daemon's judgment.
#[test]
fn support_and_previews_follow_the_shared_contract() {
    let contract: Value = serde_json::from_str(include_str!(
        "../../../src/test/contracts/fan-curve-preview.json"
    ))
    .unwrap();
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![cooled_gpu()]));

    let response = daemon.send_json(json!({
        "type": "preview_fan_curve",
        "payload": contract["request"],
    }));
    assert_eq!(response["payload"], contract["preview"]);

    for limits in contract["limits"].as_array().unwrap() {
        let name = limits["name"].as_str().unwrap();
        daemon.gpus.update("GPU-123", |gpu| {
            gpu.cooling.max_operating_temperature_c = limits["maxOperatingTemperatureC"]
                .as_u64()
                .map(|c| c as u32);
            for (fan, range) in gpu
                .cooling
                .fans
                .iter_mut()
                .zip(limits["fanSpeedRanges"].as_array().unwrap())
            {
                fan.min_speed_percent = range[0].as_u64().map(|speed| speed as u32);
                fan.max_speed_percent = range[1].as_u64().map(|speed| speed as u32);
            }
        });
        assert_eq!(
            curve_support(&daemon, "GPU-123"),
            limits["curveSupport"],
            "{name}"
        );

        for case in limits["cases"].as_array().unwrap() {
            let curve: Vec<(i64, i64)> = case["points"]
                .as_array()
                .unwrap()
                .iter()
                .map(|point| (point[0].as_i64().unwrap(), point[1].as_i64().unwrap()))
                .collect();
            let response = preview(&daemon, "GPU-123", &curve);
            assert_eq!(response["type"], "fan_curve_preview", "{response}");
            assert_eq!(
                response["payload"]["problems"], case["problems"],
                "{name}: {}",
                case["name"]
            );
        }
    }
    assert!(daemon.gpus.writes().is_empty());
}
