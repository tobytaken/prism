use serde_json::json;

use super::harness::{
    observed_gpu, tunable_gpu, tunable_gpu_with_id, FakeGpuBackend, Harness, Setter,
};
use super::tuning_store::{STAGED_FILE_SUFFIX, TUNING_STORE_FILE_NAME};
use crate::gpu::{
    ControlSupport, GpuCommandErrorCode, GpuTargetRequest, SetGpuPowerLimitRequest,
    TuningStoreHealth, TuningStoreProblem,
};
use crate::protocol::{
    ApplyGpuProfileRequest, DaemonRequest, DaemonResponse, LiveClockOffsetsRequest,
    OffsetInstruction, ProfileSetting,
};

#[test]
fn fake_sessions_keep_their_callers_and_receive_store_health_alongside_one_shot_requests() {
    use super::{SessionPeer, SessionPolicy};
    use crate::protocol::DaemonEvent;
    let daemon = Harness::start(FakeGpuBackend::default());
    let policy = SessionPolicy {
        admin_group: "admins".into(),
        admin_user: Some("alice".into()),
    };
    let alice = SessionPeer {
        uid: 1000,
        username: "alice".into(),
        groups: vec![],
        primary_group: "users".into(),
        home: "/home/alice".into(),
    };
    let bob = SessionPeer {
        uid: 1001,
        username: "bob".into(),
        groups: vec!["admins".into()],
        primary_group: "users".into(),
        home: "/home/bob".into(),
    };
    let core = daemon.core();
    let first = core.open_session(alice, &policy).unwrap();
    let second = core.open_session(bob, &policy).unwrap();
    assert_eq!(first.peer().uid, 1000);
    assert_eq!(second.peer().username, "bob");
    for session in [&first, &second] {
        assert_eq!(
            session
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap(),
            DaemonEvent::StoreHealthChanged(TuningStoreHealth::Healthy {})
        );
        assert!(matches!(
            core.handle_session(session, DaemonRequest::GetDaemonStatus),
            DaemonResponse::DaemonStatus(_)
        ));
    }
    assert!(matches!(
        daemon.send(DaemonRequest::GetDaemonStatus),
        DaemonResponse::DaemonStatus(_)
    ));
}

#[test]
fn unauthorized_sessions_are_rejected_by_the_same_admin_policy() {
    use super::{SessionPeer, SessionPolicy};
    let daemon = Harness::start(FakeGpuBackend::default());
    let policy = SessionPolicy {
        admin_group: "admins".into(),
        admin_user: Some("alice".into()),
    };
    let peer = SessionPeer {
        uid: 1002,
        username: "mallory".into(),
        groups: vec!["users".into()],
        primary_group: "users".into(),
        home: "/home/mallory".into(),
    };
    let expected = policy.authorize(&peer).unwrap_err();
    let Err(actual) = daemon.core().open_session(peer, &policy) else {
        panic!("unauthorized session was admitted")
    };
    assert_eq!(actual, expected);
    assert_eq!(actual.code, GpuCommandErrorCode::NoPermission);
}

#[test]
fn restarting_the_core_closes_old_fake_sessions_and_pushes_the_reopened_stores_health() {
    use super::{SessionPeer, SessionPolicy};
    use crate::protocol::DaemonEvent;
    let mut daemon = Harness::start(FakeGpuBackend::default());
    let policy = SessionPolicy {
        admin_group: "admins".into(),
        admin_user: None,
    };
    let peer = SessionPeer {
        uid: 1000,
        username: "alice".into(),
        groups: vec!["admins".into()],
        primary_group: "users".into(),
        home: "/home/alice".into(),
    };
    let old = daemon.core().open_session(peer.clone(), &policy).unwrap();
    old.recv_timeout(std::time::Duration::from_secs(1)).unwrap();
    std::fs::write(daemon.persistent_path(TUNING_STORE_FILE_NAME), b"{broken").unwrap();
    daemon.restart();
    assert!(matches!(
        old.recv_timeout(std::time::Duration::from_secs(1)),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
    ));
    let resumed = daemon.core().open_session(peer, &policy).unwrap();
    assert!(matches!(
        resumed
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap(),
        DaemonEvent::StoreHealthChanged(TuningStoreHealth::Unhealthy {
            problem: TuningStoreProblem::Unreadable,
            ..
        })
    ));
}

#[test]
fn framed_sessions_share_the_socket_with_one_shot_clients_and_close_on_eof() {
    use crate::framing::{read_frame, write_frame};
    use crate::protocol::{SessionMessage, SessionRequest};
    use std::io::Read;
    use std::os::unix::net::UnixStream;
    let daemon = Harness::start(FakeGpuBackend::default());
    let (mut client, mut server) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .unwrap();
    let core = daemon.core();
    let policy = super::PrismDaemonConfig {
        admin_group: "root".into(),
        admin_user: users::get_current_username().map(|n| n.to_string_lossy().into_owned()),
    };
    let serving = std::thread::spawn(move || super::handle_connection(&core, &policy, &mut server));
    write_frame(
        &mut client,
        &SessionRequest {
            id: 1,
            request: DaemonRequest::OpenSession,
        },
    )
    .unwrap();
    assert!(matches!(
        read_frame::<SessionMessage>(&mut client).unwrap(),
        Some(SessionMessage::Response {
            id: 1,
            response: DaemonResponse::DaemonStatus(_)
        })
    ));
    assert!(matches!(
        read_frame::<SessionMessage>(&mut client).unwrap(),
        Some(SessionMessage::Event { .. })
    ));
    write_frame(
        &mut client,
        &SessionRequest {
            id: 7,
            request: DaemonRequest::GetPerformance,
        },
    )
    .unwrap();
    assert!(matches!(
        read_frame::<SessionMessage>(&mut client).unwrap(),
        Some(SessionMessage::Response {
            id: 7,
            response: DaemonResponse::Performance(_)
        })
    ));
    assert!(matches!(
        daemon.send(DaemonRequest::GetDaemonStatus),
        DaemonResponse::DaemonStatus(_)
    ));
    client.shutdown(std::net::Shutdown::Write).unwrap();
    let mut end = Vec::new();
    client.read_to_end(&mut end).unwrap();
    serving.join().unwrap().unwrap();
}

#[test]
fn a_non_admin_socket_peer_is_rejected_identically_for_sessions_and_one_shot_requests() {
    use crate::framing::{read_frame, write_frame};
    use crate::protocol::{SessionMessage, SessionRequest};
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    // Root is always authorized, including on CI hosts that run the tests as root.
    if users::get_current_uid() == 0 {
        return;
    }
    let daemon = Harness::start(FakeGpuBackend::default());
    let policy = super::SessionPolicy {
        admin_group: "prism-test-no-admin-group".into(),
        admin_user: None,
    };
    let (mut one_shot, mut server) = UnixStream::pair().unwrap();
    let core = daemon.core();
    let one_shot_policy = policy.clone();
    let serving =
        std::thread::spawn(move || super::handle_connection(&core, &one_shot_policy, &mut server));
    one_shot
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .unwrap();
    one_shot
        .write_all(&serde_json::to_vec(&DaemonRequest::GetInventory).unwrap())
        .unwrap();
    one_shot.shutdown(std::net::Shutdown::Write).unwrap();
    let mut response = Vec::new();
    one_shot.read_to_end(&mut response).unwrap();
    let DaemonResponse::Error(expected) = serde_json::from_slice(&response).unwrap() else {
        panic!("unauthorized one-shot client was admitted")
    };
    serving.join().unwrap().unwrap();
    let (mut session, mut server) = UnixStream::pair().unwrap();
    let core = daemon.core();
    let serving = std::thread::spawn(move || super::handle_connection(&core, &policy, &mut server));
    session
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .unwrap();
    write_frame(
        &mut session,
        &SessionRequest {
            id: 1,
            request: DaemonRequest::OpenSession,
        },
    )
    .unwrap();
    let Some(SessionMessage::Response {
        id: 1,
        response: DaemonResponse::Error(actual),
    }) = read_frame(&mut session).unwrap()
    else {
        panic!("unauthorized session was admitted")
    };
    assert_eq!(actual, expected);
    assert_eq!(actual.code, GpuCommandErrorCode::NoPermission);
    assert!(read_frame::<SessionMessage>(&mut session)
        .unwrap()
        .is_none());
    serving.join().unwrap().unwrap();
}

fn apply_profile_request(settings: serde_json::Value) -> serde_json::Value {
    json!({ "type": "apply_profile", "payload": { "gpuId": "GPU-123", "settings": settings } })
}

#[test]
fn profile_validation_blocks_all_writes_when_an_included_setting_is_unsupported() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![observed_gpu()]));
    let result = daemon.send_json(apply_profile_request(json!({
        "powerLimitWatts": { "kind": "value", "value": 450 },
        "memoryOffsetMHz": { "kind": "value", "value": 80 }
    })));
    assert_eq!(result["payload"]["power"]["status"], "unattempted");
    assert_eq!(result["payload"]["memory"]["status"], "unattempted");
    assert!(result["payload"]["memory"]["error"]
        .as_str()
        .unwrap()
        .contains("Memory clock offset is unsupported"));
    assert_eq!(daemon.performance().gpus[0].power.current_limit_watts, None);
    assert!(daemon.gpus.writes().is_empty());
}

#[test]
fn profiles_validate_fresh_support_ranges_and_stable_ownership_before_any_write() {
    for invalid in [
        "power low",
        "power high",
        "graphics low",
        "memory high",
        "unknown support",
        "unsupported",
        "missing range",
        "reversed range",
        "absent owner",
        "unverified identity",
        "ambiguous identity",
    ] {
        let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
        if invalid == "ambiguous identity" {
            daemon.gpus.arrive(tunable_gpu());
        } else {
            daemon.gpus.update("GPU-123", |gpu| match invalid {
                "power low" => gpu.power.min_limit_watts = Some(500),
                "power high" => gpu.power.max_limit_watts = Some(400),
                "graphics low" => gpu.clocks.graphics.min_offset_mhz = Some(180),
                "memory high" => gpu.clocks.memory.max_offset_mhz = Some(50),
                "unknown support" => gpu.clocks.memory.support = ControlSupport::Unknown,
                "unsupported" => gpu.clocks.memory.support = ControlSupport::Unsupported,
                "missing range" => gpu.clocks.memory.min_offset_mhz = None,
                "reversed range" => gpu.clocks.memory.min_offset_mhz = Some(300),
                "absent owner" => gpu.stable_id = Some("GPU-replacement".into()),
                "unverified identity" => gpu.stable_id = None,
                _ => unreachable!(),
            });
        }
        let before = daemon.performance();
        let saved = daemon.saved_state_bytes();
        let result = daemon.send_json(apply_profile_request(json!({
            "powerLimitWatts": { "kind": "value", "value": 450 },
            "graphicsOffsetMHz": { "kind": "value", "value": 150 },
            "memoryOffsetMHz": { "kind": "value", "value": 80 }
        })));
        for setting in ["power", "graphics", "memory"] {
            assert_eq!(
                result["payload"][setting]["status"], "unattempted",
                "{invalid}"
            );
            assert!(
                result["payload"][setting]["error"].as_str().unwrap().len() > 10,
                "{invalid}"
            );
        }
        assert_eq!(daemon.performance(), before, "{invalid}");
        assert!(daemon.gpus.writes().is_empty(), "{invalid}");
        assert_eq!(daemon.saved_state_bytes(), saved, "{invalid}");
    }
}

#[test]
fn profile_application_is_blocked_while_its_gpu_is_absent_and_works_once_it_returns() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let request =
        apply_profile_request(json!({ "powerLimitWatts": { "kind": "value", "value": 450 } }));

    daemon.gpus.depart("GPU-123");
    let result = daemon.send_json(request.clone());
    assert_eq!(result["payload"]["power"]["status"], "unattempted");
    assert!(result["payload"]["power"]["error"]
        .as_str()
        .unwrap()
        .contains("The owning GPU is absent"));

    daemon.gpus.arrive(tunable_gpu());
    let result = daemon.send_json(request);
    assert_eq!(result["payload"]["power"]["status"], "succeeded");
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(450)
    );
}

#[test]
fn profile_application_changes_only_included_values_on_its_owner() {
    let mut owner = tunable_gpu();
    // Omitted controls need no applicability information.
    owner.clocks.memory.support = ControlSupport::Unknown;
    let gpus = FakeGpuBackend::with_gpus(vec![tunable_gpu_with_id("GPU-other"), owner]);
    let daemon = Harness::start(gpus);
    let other = daemon.gpus.gpu("GPU-other");
    let before = daemon.gpus.gpu("GPU-123");

    let result = daemon.send_json(apply_profile_request(json!({
        "powerLimitWatts": { "kind": "value", "value": 450 },
        "graphicsOffsetMHz": { "kind": "value", "value": -150 }
    })));
    assert_eq!(result["payload"]["power"]["status"], "succeeded");
    assert_eq!(result["payload"]["graphics"]["status"], "succeeded");
    assert!(result["payload"]["memory"].is_null());
    let actual = daemon.performance();
    assert_eq!(actual.gpus[0], other);
    assert_eq!(actual.gpus[1].power.current_limit_watts, Some(450));
    assert_eq!(
        actual.gpus[1].clocks.graphics.current_offset_mhz,
        Some(-150)
    );
    assert_eq!(actual.gpus[1].clocks.memory, before.clocks.memory);
    assert_eq!(actual.gpus[1].cooling, before.cooling);
}

#[test]
fn out_of_range_profile_values_propose_every_bound_correction_without_any_write() {
    let mut gpu = tunable_gpu();
    gpu.power.min_limit_watts = Some(100);
    gpu.power.max_limit_watts = Some(280);
    gpu.clocks.graphics.min_offset_mhz = Some(-100);
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![gpu]));
    let before = daemon.performance();
    let result = daemon.send_json(apply_profile_request(json!({
        "powerLimitWatts": { "kind": "value", "value": 300 },
        "graphicsOffsetMHz": { "kind": "value", "value": -150 },
        "memoryOffsetMHz": { "kind": "value", "value": 80 }
    })));
    let contract: serde_json::Value = serde_json::from_str(include_str!(
        "../../../src/test/contracts/profile-corrections.json"
    ))
    .unwrap();
    assert_eq!(result["payload"], contract);
    let DaemonResponse::ProfileApplication(roundtrip) = serde_json::from_value(result).unwrap()
    else {
        panic!("expected profile application")
    };
    assert_eq!(roundtrip.corrections.len(), 2);
    assert_eq!(daemon.performance(), before);

    let result = daemon.send_json(apply_profile_request(json!({
        "powerLimitWatts": { "kind": "value", "value": 280 },
        "graphicsOffsetMHz": { "kind": "value", "value": -100 },
        "memoryOffsetMHz": { "kind": "value", "value": 80 }
    })));
    for setting in ["power", "graphics", "memory"] {
        assert_eq!(result["payload"][setting]["status"], "succeeded");
    }
    assert!(result["payload"].get("corrections").is_none());
    let gpu = daemon.gpus.gpu("GPU-123");
    assert_eq!(gpu.power.current_limit_watts, Some(280));
    assert_eq!(gpu.clocks.graphics.current_offset_mhz, Some(-100));
}

#[test]
fn range_corrections_are_not_offered_when_another_included_setting_blocks_application() {
    for blocker in [
        "unsupported",
        "unknown support",
        "missing range",
        "absent owner",
    ] {
        let mut gpu = tunable_gpu();
        gpu.power.max_limit_watts = Some(400);
        match blocker {
            "unsupported" => gpu.clocks.memory.support = ControlSupport::Unsupported,
            "unknown support" => gpu.clocks.memory.support = ControlSupport::Unknown,
            "missing range" => gpu.clocks.memory.max_offset_mhz = None,
            "absent owner" => gpu.stable_id = Some("GPU-replacement".into()),
            _ => unreachable!(),
        }
        let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![gpu]));
        let before = daemon.performance();
        let result = daemon.send_json(apply_profile_request(json!({
            "powerLimitWatts": { "kind": "value", "value": 450 },
            "memoryOffsetMHz": { "kind": "value", "value": 80 }
        })));
        assert!(result["payload"].get("corrections").is_none(), "{blocker}");
        assert_eq!(result["payload"]["power"]["status"], "unattempted");
        assert_eq!(result["payload"]["memory"]["status"], "unattempted");
        assert_eq!(daemon.performance(), before, "{blocker}");
    }
}

#[test]
fn profile_application_reports_partial_failure_and_validates_retries() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    daemon
        .gpus
        .fail("GPU-123", Setter::GraphicsOffset, "Graphics write denied");
    let result = daemon.send_json(apply_profile_request(json!({
        "powerLimitWatts": { "kind": "value", "value": 450 },
        "graphicsOffsetMHz": { "kind": "value", "value": 150 },
        "memoryOffsetMHz": { "kind": "value", "value": 80 }
    })));
    assert_eq!(result["payload"]["power"]["status"], "succeeded");
    assert_eq!(result["payload"]["graphics"]["status"], "failed");
    assert_eq!(result["payload"]["memory"]["status"], "unattempted");
    let contract: serde_json::Value = serde_json::from_str(include_str!(
        "../../../src/test/contracts/profile-outcomes.json"
    ))
    .unwrap();
    assert_eq!(result["payload"], contract);
    let roundtrip: DaemonResponse = serde_json::from_value(result).unwrap();
    assert!(matches!(roundtrip, DaemonResponse::ProfileApplication(_)));

    daemon.gpus.stop_failing("GPU-123", Setter::GraphicsOffset);
    daemon
        .gpus
        .update("GPU-123", |gpu| gpu.clocks.memory.max_offset_mhz = Some(50));
    let retry = apply_profile_request(json!({
        "graphicsOffsetMHz": { "kind": "value", "value": 150 },
        "memoryOffsetMHz": { "kind": "value", "value": 80 }
    }));
    let rejected = daemon.send_json(retry.clone());
    assert_eq!(rejected["payload"]["graphics"]["status"], "unattempted");
    assert!(rejected["payload"]["memory"]["error"]
        .as_str()
        .unwrap()
        .contains("-200 to 50"));

    daemon.gpus.update("GPU-123", |gpu| {
        gpu.clocks.memory.max_offset_mhz = Some(200)
    });
    let success = daemon.send_json(retry);
    assert_eq!(success["payload"]["graphics"]["status"], "succeeded");
    assert_eq!(success["payload"]["memory"]["status"], "succeeded");
    assert!(success["payload"]["power"].is_null());
    let gpu = daemon.gpus.gpu("GPU-123");
    assert_eq!(gpu.power.current_limit_watts, Some(450));
    assert_eq!(gpu.clocks.graphics.current_offset_mhz, Some(150));
    assert_eq!(gpu.clocks.memory.current_offset_mhz, Some(80));
}

fn apply_profile_settings(daemon: &Harness, settings: serde_json::Value) -> serde_json::Value {
    let result = daemon.send_json(apply_profile_request(settings));
    result["payload"].clone()
}

#[test]
fn explicit_defaults_follow_the_currently_reported_power_default_while_numbers_stay_fixed() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let defaults = json!({
        "powerLimitWatts": { "kind": "default" },
        "graphicsOffsetMHz": { "kind": "default" },
        "memoryOffsetMHz": { "kind": "default" }
    });
    let fixed = json!({ "powerLimitWatts": { "kind": "value", "value": 575 } });
    let result = apply_profile_settings(&daemon, defaults.clone());
    for setting in ["power", "graphics", "memory"] {
        assert_eq!(result[setting]["status"], "succeeded", "{setting}");
    }
    let gpu = daemon.gpus.gpu("GPU-123");
    assert_eq!(gpu.power.current_limit_watts, Some(575));
    assert_eq!(gpu.clocks.graphics.current_offset_mhz, Some(0));
    assert_eq!(gpu.clocks.memory.current_offset_mhz, Some(0));

    // A driver update changes the reported default.
    daemon
        .gpus
        .update("GPU-123", |gpu| gpu.power.default_limit_watts = Some(520));
    apply_profile_settings(&daemon, fixed);
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(575)
    );
    apply_profile_settings(&daemon, defaults);
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(520)
    );
}

#[test]
fn unavailable_or_out_of_range_defaults_block_every_included_instruction_before_writes() {
    for (invalid, expected) in [
        (
            "power default unavailable",
            "Power limit: the default is unavailable",
        ),
        (
            "power default above maximum",
            "Power limit: the default 650 is outside the current allowed range 300 to 600",
        ),
        (
            "offset range excludes zero",
            "Memory clock offset: the default 0 is outside the current allowed range 10 to 200",
        ),
        ("unsupported default", "Memory clock offset is unsupported"),
    ] {
        let mut gpu = tunable_gpu();
        match invalid {
            "power default unavailable" => gpu.power.default_limit_watts = None,
            "power default above maximum" => gpu.power.default_limit_watts = Some(650),
            "offset range excludes zero" => gpu.clocks.memory.min_offset_mhz = Some(10),
            "unsupported default" => gpu.clocks.memory.support = ControlSupport::Unsupported,
            _ => unreachable!(),
        }
        let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![gpu]));
        let before = daemon.performance();
        let result = apply_profile_settings(
            &daemon,
            json!({
                "powerLimitWatts": { "kind": "default" },
                "graphicsOffsetMHz": { "kind": "value", "value": 150 },
                "memoryOffsetMHz": { "kind": "default" }
            }),
        );
        // A default has no saved value to correct, so no range review is offered.
        assert!(result.get("corrections").is_none(), "{invalid}");
        for setting in ["power", "graphics", "memory"] {
            assert_eq!(result[setting]["status"], "unattempted", "{invalid}");
            assert!(
                result[setting]["error"]
                    .as_str()
                    .unwrap()
                    .contains(expected),
                "{invalid}: {}",
                result[setting]["error"]
            );
        }
        assert_eq!(daemon.performance(), before, "{invalid}");
    }
}

#[test]
fn range_review_proposes_numeric_corrections_alongside_applicable_defaults() {
    let mut gpu = tunable_gpu();
    gpu.clocks.graphics.max_offset_mhz = Some(100);
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![gpu]));
    let before = daemon.performance();
    let result = apply_profile_settings(
        &daemon,
        json!({
            "powerLimitWatts": { "kind": "default" },
            "graphicsOffsetMHz": { "kind": "value", "value": 150 }
        }),
    );
    assert_eq!(
        result["corrections"],
        json!([{
            "setting": "graphicsOffsetMHz",
            "savedValue": 150,
            "proposedValue": 100,
            "bound": "maximum"
        }])
    );
    assert_eq!(result["power"]["status"], "unattempted");
    assert_eq!(daemon.performance(), before);
}

#[test]
fn omitted_settings_need_no_default_resolution() {
    let mut gpu = tunable_gpu();
    gpu.power.default_limit_watts = None;
    gpu.power.support = ControlSupport::Unknown;
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![gpu]));
    let result = apply_profile_settings(
        &daemon,
        json!({ "graphicsOffsetMHz": { "kind": "default" } }),
    );
    assert_eq!(result["graphics"]["status"], "succeeded");
    assert!(result["power"].is_null());
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(475)
    );
}

#[test]
fn profile_settings_round_trip_omitted_numeric_and_default_instructions() {
    let request: ApplyGpuProfileRequest = serde_json::from_value(json!({
        "gpuId": "GPU-123",
        "settings": {
            "powerLimitWatts": { "kind": "value", "value": 575 },
            "graphicsOffsetMHz": { "kind": "default" }
        }
    }))
    .unwrap();
    assert_eq!(
        request.settings.power_limit_watts,
        Some(ProfileSetting::Value { value: 575 })
    );
    assert_eq!(
        request.settings.graphics_offset_mhz,
        Some(ProfileSetting::Default {})
    );
    assert_eq!(request.settings.memory_offset_mhz, None);
    assert_eq!(
        serde_json::to_value(&request).unwrap()["settings"],
        json!({
            "powerLimitWatts": { "kind": "value", "value": 575 },
            "graphicsOffsetMHz": { "kind": "default" }
        })
    );
}

#[test]
fn live_offsets_report_partial_success() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![observed_gpu()]));
    daemon
        .gpus
        .fail("GPU-123", Setter::MemoryOffset, "Memory write failed");
    let response = daemon.send(DaemonRequest::SetClockOffsets(LiveClockOffsetsRequest {
        gpu_id: "GPU-123".into(),
        graphics_offset_mhz: Some(OffsetInstruction::Value(150)),
        memory_offset_mhz: Some(OffsetInstruction::Value(80)),
    }));
    let json = serde_json::to_value(response).unwrap();
    assert_eq!(json["payload"]["graphics"]["status"], "succeeded");
    assert_eq!(json["payload"]["memory"]["status"], "failed");
    assert_eq!(json["payload"]["memory"]["error"], "Memory write failed");
    let contract: serde_json::Value = serde_json::from_str(include_str!(
        "../../../src/test/contracts/tuning-outcomes.json"
    ))
    .unwrap();
    assert_eq!(json["payload"], contract);
    let roundtrip: DaemonResponse = serde_json::from_value(json).unwrap();
    assert!(matches!(roundtrip, DaemonResponse::ClockOffsets(_)));
    assert_eq!(
        daemon.performance().gpus[0]
            .clocks
            .graphics
            .current_offset_mhz,
        Some(150)
    );
}

#[test]
fn live_offsets_stop_after_failure_and_leave_later_setting_unattempted() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![observed_gpu()]));
    daemon
        .gpus
        .fail("GPU-123", Setter::GraphicsOffset, "GPU unavailable");
    let response = daemon.send(DaemonRequest::SetClockOffsets(LiveClockOffsetsRequest {
        gpu_id: "GPU-123".into(),
        graphics_offset_mhz: Some(OffsetInstruction::Value(150)),
        memory_offset_mhz: Some(OffsetInstruction::Value(80)),
    }));
    let json = serde_json::to_value(response).unwrap();
    assert_eq!(json["payload"]["graphics"]["status"], "failed");
    assert_eq!(json["payload"]["memory"]["status"], "unattempted");
    let gpu = daemon.gpus.gpu("GPU-123");
    assert_eq!(gpu.clocks.graphics.current_offset_mhz, None);
    assert_eq!(gpu.clocks.memory.current_offset_mhz, None);
}

#[test]
fn live_reset_reports_partial_outcomes_and_memory_only_retry_leaves_graphics_alone() {
    let mut gpu = observed_gpu();
    gpu.clocks.graphics.support = ControlSupport::Supported;
    gpu.clocks.memory.support = ControlSupport::Supported;
    gpu.clocks.graphics.current_offset_mhz = Some(150);
    gpu.clocks.memory.current_offset_mhz = Some(80);
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![gpu]));
    daemon
        .gpus
        .fail("GPU-123", Setter::MemoryOffset, "Memory write failed");
    let response = daemon.send(DaemonRequest::ResetClockOffsets(GpuTargetRequest {
        gpu_id: "GPU-123".into(),
    }));
    let json = serde_json::to_value(response).unwrap();
    assert_eq!(json["payload"]["graphics"]["status"], "succeeded");
    assert_eq!(json["payload"]["memory"]["status"], "failed");

    daemon.gpus.stop_failing("GPU-123", Setter::MemoryOffset);
    let response = daemon.send(DaemonRequest::SetClockOffsets(LiveClockOffsetsRequest {
        gpu_id: "GPU-123".into(),
        graphics_offset_mhz: None,
        memory_offset_mhz: Some(OffsetInstruction::Value(0)),
    }));
    let json = serde_json::to_value(response).unwrap();
    assert!(json["payload"]["graphics"].is_null());
    assert_eq!(json["payload"]["memory"]["status"], "succeeded");
    let gpu = daemon.gpus.gpu("GPU-123");
    assert_eq!(gpu.clocks.graphics.current_offset_mhz, Some(0));
    assert_eq!(gpu.clocks.memory.current_offset_mhz, Some(0));
}

#[test]
fn performance_reads_preserve_tuning_states_and_recover() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../src/test/contracts/tuning-observations.json"
    ))
    .unwrap();
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![observed_gpu()]));
    let response = daemon.send(DaemonRequest::GetPerformance);
    let wire = serde_json::to_value(&response).unwrap();
    assert_eq!(wire["type"], "performance");
    assert_eq!(wire["payload"], fixture);
    assert_eq!(
        serde_json::from_value::<DaemonResponse>(wire).unwrap(),
        response
    );

    daemon.gpus.update("GPU-123", |gpu| {
        gpu.power.current_limit_watts = Some(450);
        gpu.power.current_value_error = None;
        gpu.clocks.graphics.support = ControlSupport::Supported;
        gpu.clocks.graphics.reason = None;
        gpu.clocks.graphics.current_offset_mhz = Some(80);
        gpu.clocks.graphics.min_offset_mhz = Some(-200);
        gpu.clocks.graphics.max_offset_mhz = Some(200);
    });
    let performance = daemon.performance();
    assert_eq!(performance.gpus[0].power.current_limit_watts, Some(450));
    assert_eq!(performance.gpus[0].power.current_value_error, None);
    assert_eq!(
        performance.gpus[0].clocks.graphics.support,
        ControlSupport::Supported
    );
    assert_eq!(
        performance.gpus[0].clocks.memory.support,
        ControlSupport::Unsupported
    );
    assert!(daemon.gpus.writes().is_empty());
}

#[test]
fn get_daemon_status_returns_connected_socket_info() {
    let daemon = Harness::start(FakeGpuBackend::default());

    let DaemonResponse::DaemonStatus(status) = daemon.send(DaemonRequest::GetDaemonStatus) else {
        panic!("expected daemon status response");
    };

    assert!(status.reachable);
    assert!(status.managed_by_daemon);
    assert!(status.socket_path.contains("prismd-test.sock"));
}

#[test]
fn power_limit_commands_return_command_responses_without_mutating_state() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    daemon
        .gpus
        .fail("GPU-123", Setter::PowerLimit, "mock permission denied");

    let response = daemon.send(DaemonRequest::SetPowerLimit(SetGpuPowerLimitRequest {
        gpu_id: "GPU-123".into(),
        limit_watts: 250,
    }));

    let DaemonResponse::Command(response) = response else {
        panic!("expected command response");
    };

    assert!(!response.success);
    assert_eq!(response.error.as_deref(), Some("mock permission denied"));
    assert!(daemon.saved_state_bytes().is_none());
}

#[test]
fn command_success_returns_success_response() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));

    let response = daemon.send(DaemonRequest::SetPowerLimit(SetGpuPowerLimitRequest {
        gpu_id: "GPU-123".into(),
        limit_watts: 250,
    }));

    let DaemonResponse::Command(response) = response else {
        panic!("expected command response");
    };

    assert!(response.success);
    assert!(response.error.is_none());
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(250)
    );
}

fn set_power(gpu_id: &str, limit_watts: u32) -> DaemonRequest {
    DaemonRequest::SetPowerLimit(SetGpuPowerLimitRequest {
        gpu_id: gpu_id.into(),
        limit_watts,
    })
}

#[test]
fn a_write_blocked_on_one_gpu_delays_no_other_gpu_and_no_reads() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![
        tunable_gpu_with_id("GPU-slow"),
        tunable_gpu_with_id("GPU-fast"),
    ]));
    let gate = daemon.gpus.block("GPU-slow", Setter::PowerLimit);
    let mut blocked = daemon.send_in_background(set_power("GPU-slow", 400));
    gate.wait_until_entered();

    // Each of these fails its test by timing out if it waits for the blocked write.
    let DaemonResponse::Command(fast) = daemon.send(set_power("GPU-fast", 410)) else {
        panic!("expected command response")
    };
    assert!(fast.success);
    let profile = daemon.send_json(json!({ "type": "apply_profile", "payload": {
        "gpuId": "GPU-fast", "settings": { "graphicsOffsetMHz": { "kind": "value", "value": 90 } }
    }}));
    assert_eq!(profile["payload"]["graphics"]["status"], "succeeded");
    assert!(matches!(
        daemon.send(DaemonRequest::GetInventory),
        DaemonResponse::Inventory(_)
    ));
    assert!(matches!(
        daemon.send(DaemonRequest::GetDaemonStatus),
        DaemonResponse::DaemonStatus(_)
    ));
    assert!(matches!(
        daemon.send(DaemonRequest::GetDriverJob(
            crate::drivers::DriverJobRequest {
                job_id: "driver-unknown".into(),
            }
        )),
        DaemonResponse::Error(_)
    ));
    let fast = daemon.gpus.gpu("GPU-fast");
    assert_eq!(fast.power.current_limit_watts, Some(410));
    assert_eq!(fast.clocks.graphics.current_offset_mhz, Some(90));
    assert_eq!(fast.clocks.memory.current_offset_mhz, Some(0));
    assert_eq!(
        daemon.performance().gpus[0].power.current_limit_watts,
        Some(475)
    );
    assert!(blocked.is_pending());

    gate.release();
    let DaemonResponse::Command(slow) = blocked.response() else {
        panic!("expected command response")
    };
    assert!(slow.success);
    assert_eq!(
        daemon.gpus.gpu("GPU-slow").power.current_limit_watts,
        Some(400)
    );
}

#[test]
fn writes_to_the_same_gpu_wait_for_the_write_in_progress() {
    let daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));
    let gate = daemon.gpus.block("GPU-123", Setter::PowerLimit);
    let first = daemon.send_in_background(set_power("GPU-123", 400));
    gate.wait_until_entered();
    let mut second =
        daemon.send_in_background(DaemonRequest::SetClockOffsets(LiveClockOffsetsRequest {
            gpu_id: "GPU-123".into(),
            graphics_offset_mhz: Some(OffsetInstruction::Value(60)),
            memory_offset_mhz: None,
        }));

    assert!(second.is_pending());
    assert_eq!(
        daemon.gpus.writes(),
        vec![("GPU-123".to_string(), Setter::PowerLimit)]
    );

    gate.release();
    assert!(matches!(first.response(), DaemonResponse::Command(_)));
    assert!(matches!(second.response(), DaemonResponse::ClockOffsets(_)));
    assert_eq!(
        daemon.gpus.writes(),
        vec![
            ("GPU-123".to_string(), Setter::PowerLimit),
            ("GPU-123".to_string(), Setter::GraphicsOffset),
        ]
    );
}

#[test]
fn a_client_slow_to_send_its_request_does_not_hold_up_other_connections() {
    use std::io::{Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};

    let directory = tempfile::tempdir().unwrap();
    let socket_path = directory.path().join("prismd.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();
    let daemon = Harness::start(FakeGpuBackend::default());
    let core = daemon.core();
    let config = std::sync::Arc::new(super::PrismDaemonConfig {
        admin_group: "root".into(),
        admin_user: None,
    });
    std::thread::spawn(move || super::serve(listener, core, config));

    let _slow = UnixStream::connect(&socket_path).unwrap();
    let mut client = UnixStream::connect(&socket_path).unwrap();
    client
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .unwrap();
    client
        .write_all(&serde_json::to_vec(&DaemonRequest::GetDaemonStatus).unwrap())
        .unwrap();
    client.shutdown(std::net::Shutdown::Write).unwrap();
    let mut response = Vec::new();
    client
        .read_to_end(&mut response)
        .expect("the daemon did not answer while another client was slow");

    assert!(matches!(
        serde_json::from_slice(&response).unwrap(),
        DaemonResponse::DaemonStatus(_)
    ));
}

const TUNING_STORE: &str = TUNING_STORE_FILE_NAME;

#[test]
fn without_a_tuning_store_the_daemon_starts_healthy_at_the_current_schema() {
    let mut daemon = Harness::start(FakeGpuBackend::with_gpus(vec![tunable_gpu()]));

    assert_eq!(
        daemon.status().tuning_store,
        Some(TuningStoreHealth::Healthy {})
    );
    daemon.restart();
    assert_eq!(
        daemon.status().tuning_store,
        Some(TuningStoreHealth::Healthy {})
    );
    assert!(daemon.persistent_file(TUNING_STORE).is_some());
}

fn assert_unhealthy(daemon: &Harness, expected: TuningStoreProblem) {
    let Some(TuningStoreHealth::Unhealthy {
        problem,
        path,
        detail,
    }) = daemon.status().tuning_store
    else {
        panic!("expected an unhealthy tuning store");
    };
    assert_eq!(problem, expected);
    assert_eq!(
        path,
        daemon.persistent_path(TUNING_STORE).display().to_string()
    );
    assert!(!detail.is_empty());
}

// Acceptance scenario 11 from #8, as far as it exists so far: the file is preserved, manual
// tuning works and the problem is reported. Startup and legacy reapplication are suspended.
#[test]
fn a_corrupt_tuning_store_is_preserved_and_reported_while_manual_tuning_keeps_working() {
    let corrupt: &[u8] = b"{\"schemaVersion\": 1, \"profiles\": [tru";
    let mut daemon = Harness::start_with_persistent_files(
        FakeGpuBackend::with_gpus(vec![tunable_gpu()]),
        &[(TUNING_STORE, corrupt)],
    );
    assert_eq!(
        daemon.persistent_file(TUNING_STORE).as_deref(),
        Some(corrupt)
    );
    assert_unhealthy(&daemon, TuningStoreProblem::Unreadable);

    let DaemonResponse::Command(power) = daemon.send(set_power("GPU-123", 450)) else {
        panic!("expected a command response");
    };
    assert!(power.success);
    let offsets = daemon.send(DaemonRequest::SetClockOffsets(LiveClockOffsetsRequest {
        gpu_id: "GPU-123".into(),
        graphics_offset_mhz: Some(OffsetInstruction::Value(150)),
        memory_offset_mhz: None,
    }));
    let offsets = serde_json::to_value(offsets).unwrap();
    assert_eq!(offsets["payload"]["graphics"]["status"], "succeeded");
    let profile = daemon.send_json(apply_profile_request(json!({
        "memoryOffsetMHz": { "kind": "value", "value": 80 }
    })));
    assert_eq!(profile["payload"]["memory"]["status"], "succeeded");
    let gpu = daemon.gpus.gpu("GPU-123");
    assert_eq!(gpu.power.current_limit_watts, Some(450));
    assert_eq!(gpu.clocks.graphics.current_offset_mhz, Some(150));
    assert_eq!(gpu.clocks.memory.current_offset_mhz, Some(80));
    assert_eq!(
        daemon.persistent_file(TUNING_STORE).as_deref(),
        Some(corrupt)
    );

    daemon.restart();
    assert_unhealthy(&daemon, TuningStoreProblem::Unreadable);
    assert_eq!(
        daemon.persistent_file(TUNING_STORE).as_deref(),
        Some(corrupt)
    );
}

#[test]
fn a_tuning_store_from_a_newer_schema_version_is_treated_as_unreadable_and_preserved() {
    let newer: &[u8] = b"{\"schemaVersion\": 99, \"users\": []}";
    let mut daemon = Harness::start_with_persistent_files(
        FakeGpuBackend::with_gpus(vec![tunable_gpu()]),
        &[(TUNING_STORE, newer)],
    );
    assert_unhealthy(&daemon, TuningStoreProblem::NewerVersion);

    let DaemonResponse::Command(power) = daemon.send(set_power("GPU-123", 450)) else {
        panic!("expected a command response");
    };
    assert!(power.success);
    assert_eq!(
        daemon.gpus.gpu("GPU-123").power.current_limit_watts,
        Some(450)
    );
    assert_eq!(daemon.persistent_file(TUNING_STORE).as_deref(), Some(newer));
    daemon.restart();
    assert_unhealthy(&daemon, TuningStoreProblem::NewerVersion);
    assert_eq!(daemon.persistent_file(TUNING_STORE).as_deref(), Some(newer));
}

#[test]
fn a_store_write_interrupted_by_a_crash_leaves_the_previous_document_in_use() {
    let previous: &[u8] = b"{\"schemaVersion\": 6}";
    let daemon = Harness::start_with_persistent_files(
        FakeGpuBackend::with_gpus(vec![tunable_gpu()]),
        &[
            (TUNING_STORE, previous),
            (
                &format!("{TUNING_STORE}{STAGED_FILE_SUFFIX}"),
                b"{\"schemaVersion\": 1, \"pro",
            ),
        ],
    );

    assert_eq!(
        daemon.status().tuning_store,
        Some(TuningStoreHealth::Healthy {})
    );
    assert_eq!(
        daemon.persistent_file(TUNING_STORE).as_deref(),
        Some(previous)
    );
}
