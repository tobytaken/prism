use std::env;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::daemon_client;
use crate::drivers::{
    collect_driver_status, DriverJob, DriverJobRequest, DriverStatusResponse,
    StartDriverActionRequest,
};
use crate::gpu::{
    collect_inventory, collect_performance, GpuCommandError, GpuCommandErrorCode, GpuDaemonStatus,
    GpuInventoryResponse, GpuPerformanceResponse, GpuTargetRequest, SetGpuFanModeAutoRequest,
    SetGpuFanSpeedRequest, SetGpuPowerLimitRequest,
};
use crate::protocol::{
    AddSteamGameRequest, GameRequest, SetAutomaticSwitchingRequest, SetGameProfileRequest,
};
use crate::protocol::{
    CorrectAndApplyProfileRequest, ImportProfilesRequest, LiveClockOffsetsRequest,
    ProfileIdRequest, RenameProfileRequest, SaveProfileAsNewRequest, SaveToCurrentProfileRequest,
    SetStartupBehaviorRequest, SetStartupProfileRequest, StartupGpuRequest,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeRequest {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeResponse {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<BridgeError>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeError {
    pub code: String,
    pub message: String,
}

pub fn handle_request(request: BridgeRequest) -> BridgeResponse {
    let result = match request.method.as_str() {
        "gpu.applyProfile" => {
            parse_and_serialize_result::<crate::protocol::ApplyGpuProfileRequest, _, _>(
                request.params,
                "gpu.applyProfile",
                |request| daemon_client::apply_gpu_profile(request).map_err(BridgeError::from),
            )
        }
        "gpu.getMigrationState" => daemon_client::get_migration_state()
            .map_err(BridgeError::from)
            .and_then(serialize_result),
        "gpu.dismissMigrationNotice" => daemon_client::dismiss_migration_notice()
            .map_err(BridgeError::from)
            .and_then(serialize_result),
        "gpu.adoptMigratedProfile" => parse_and_serialize_result::<ProfileIdRequest, _, _>(
            request.params,
            "gpu.adoptMigratedProfile",
            |request| daemon_client::adopt_migrated_profile(request).map_err(BridgeError::from),
        ),
        "gpu.getStartupState" => daemon_client::get_startup_state()
            .map_err(BridgeError::from)
            .and_then(serialize_result),
        "gpu.setStartupBehavior" => parse_and_serialize_result::<SetStartupBehaviorRequest, _, _>(
            request.params,
            "gpu.setStartupBehavior",
            |request| daemon_client::set_startup_behavior(request).map_err(BridgeError::from),
        ),
        "gpu.setStartupProfile" => parse_and_serialize_result::<SetStartupProfileRequest, _, _>(
            request.params,
            "gpu.setStartupProfile",
            |request| daemon_client::set_startup_profile(request).map_err(BridgeError::from),
        ),
        "gpu.dismissStartupOutcome" => parse_and_serialize_result::<StartupGpuRequest, _, _>(
            request.params,
            "gpu.dismissStartupOutcome",
            |request| daemon_client::dismiss_startup_outcome(request).map_err(BridgeError::from),
        ),
        "gpu.listProfiles" => daemon_client::list_gpu_profiles()
            .map_err(BridgeError::from)
            .and_then(serialize_result),
        "gpu.renameProfile" => parse_and_serialize_result::<RenameProfileRequest, _, _>(
            request.params,
            "gpu.renameProfile",
            |request| daemon_client::rename_gpu_profile(request).map_err(BridgeError::from),
        ),
        "gpu.deleteProfile" => parse_and_serialize_result::<ProfileIdRequest, _, _>(
            request.params,
            "gpu.deleteProfile",
            |request| daemon_client::delete_gpu_profile(request).map_err(BridgeError::from),
        ),
        "gpu.getTuningStates" => daemon_client::get_gpu_tuning_states()
            .map_err(BridgeError::from)
            .and_then(serialize_result),
        "gpu.saveProfileAsNew" => parse_and_serialize_result::<SaveProfileAsNewRequest, _, _>(
            request.params,
            "gpu.saveProfileAsNew",
            |request| daemon_client::save_gpu_profile_as_new(request).map_err(BridgeError::from),
        ),
        "gpu.saveProfileToCurrent" => {
            parse_and_serialize_result::<SaveToCurrentProfileRequest, _, _>(
                request.params,
                "gpu.saveProfileToCurrent",
                |request| {
                    daemon_client::save_gpu_profile_to_current(request).map_err(BridgeError::from)
                },
            )
        }
        "gpu.applyProfileById" => parse_and_serialize_result::<ProfileIdRequest, _, _>(
            request.params,
            "gpu.applyProfileById",
            |request| daemon_client::apply_gpu_profile_by_id(request).map_err(BridgeError::from),
        ),
        "gpu.correctAndApplyProfile" => {
            parse_and_serialize_result::<CorrectAndApplyProfileRequest, _, _>(
                request.params,
                "gpu.correctAndApplyProfile",
                |request| {
                    daemon_client::correct_and_apply_gpu_profile(request).map_err(BridgeError::from)
                },
            )
        }
        "gpu.importProfiles" => parse_and_serialize_result::<ImportProfilesRequest, _, _>(
            request.params,
            "gpu.importProfiles",
            |request| daemon_client::import_gpu_profiles(request).map_err(BridgeError::from),
        ),
        "games.listSteamApps" => daemon_client::list_steam_apps()
            .map_err(BridgeError::from)
            .and_then(serialize_result),
        "games.getRunning" => daemon_client::get_running_games()
            .map_err(BridgeError::from)
            .and_then(serialize_result),
        "games.getSwitchingStatus" => daemon_client::get_automatic_switching_status()
            .map_err(BridgeError::from)
            .and_then(serialize_result),
        "games.get" => daemon_client::get_games()
            .map_err(BridgeError::from)
            .and_then(serialize_result),
        "games.addSteamGame" => parse_and_serialize_result::<AddSteamGameRequest, _, _>(
            request.params,
            "games.addSteamGame",
            |request| daemon_client::add_steam_game(request).map_err(BridgeError::from),
        ),
        "games.setProfile" => parse_and_serialize_result::<SetGameProfileRequest, _, _>(
            request.params,
            "games.setProfile",
            |request| daemon_client::set_game_profile(request).map_err(BridgeError::from),
        ),
        "games.remove" => parse_and_serialize_result::<GameRequest, _, _>(
            request.params,
            "games.remove",
            |request| daemon_client::remove_game(request).map_err(BridgeError::from),
        ),
        "games.setAutomaticSwitching" => {
            parse_and_serialize_result::<SetAutomaticSwitchingRequest, _, _>(
                request.params,
                "games.setAutomaticSwitching",
                |request| {
                    daemon_client::set_automatic_switching(request).map_err(BridgeError::from)
                },
            )
        }
        "gpu.getInventory" => serialize_result(get_gpu_inventory()),
        "gpu.getPerformance" => serialize_result(get_gpu_performance()),
        "gpu.getDaemonStatus" => serialize_result(get_gpu_daemon_status()),
        "gpu.setPowerLimit" => parse_and_serialize_result::<SetGpuPowerLimitRequest, _, _>(
            request.params,
            "gpu.setPowerLimit",
            set_gpu_power_limit,
        ),
        "gpu.resetPowerLimit" => parse_and_serialize_result::<GpuTargetRequest, _, _>(
            request.params,
            "gpu.resetPowerLimit",
            reset_gpu_power_limit,
        ),
        "gpu.setFanSpeed" => parse_and_serialize_result::<SetGpuFanSpeedRequest, _, _>(
            request.params,
            "gpu.setFanSpeed",
            set_gpu_fan_speed,
        ),
        "gpu.setFanModeAuto" => parse_and_serialize_result::<SetGpuFanModeAutoRequest, _, _>(
            request.params,
            "gpu.setFanModeAuto",
            set_gpu_fan_mode_auto,
        ),
        "gpu.setClockOffsets" => parse_and_serialize_result::<LiveClockOffsetsRequest, _, _>(
            request.params,
            "gpu.setClockOffsets",
            set_gpu_clock_offsets,
        ),
        "gpu.resetClockOffsets" => parse_and_serialize_result::<GpuTargetRequest, _, _>(
            request.params,
            "gpu.resetClockOffsets",
            reset_gpu_clock_offsets,
        ),
        "drivers.getStatus" => serialize_result(get_driver_status()),
        "drivers.startAction" => parse_and_serialize_result::<StartDriverActionRequest, _, _>(
            request.params,
            "drivers.startAction",
            start_driver_action,
        ),
        "drivers.getJob" => parse_and_serialize_result::<DriverJobRequest, _, _>(
            request.params,
            "drivers.getJob",
            get_driver_job,
        ),
        "drivers.cancelJob" => parse_and_serialize_result::<DriverJobRequest, _, _>(
            request.params,
            "drivers.cancelJob",
            cancel_driver_job,
        ),
        "drivers.requestReboot" => request_host_reboot(),
        _ => Err(BridgeError {
            code: "not_found".into(),
            message: format!("Unknown Prism backend method {}", request.method),
        }),
    };

    match result {
        Ok(value) => BridgeResponse {
            id: request.id,
            result: Some(value),
            error: None,
        },
        Err(error) => BridgeResponse {
            id: request.id,
            result: None,
            error: Some(error),
        },
    }
}

pub fn invalid_request_response(message: impl Into<String>) -> BridgeResponse {
    BridgeResponse {
        id: 0,
        result: None,
        error: Some(BridgeError {
            code: "invalid_request".into(),
            message: message.into(),
        }),
    }
}

fn parse_and_serialize_result<T, F, R>(
    params: Option<Value>,
    method: &str,
    handler: F,
) -> Result<Value, BridgeError>
where
    T: DeserializeOwned,
    R: Serialize,
    F: FnOnce(T) -> Result<R, BridgeError>,
{
    let payload = parse_params(params, method)?;
    serialize_result(handler(payload)?)
}

fn parse_params<T>(params: Option<Value>, method: &str) -> Result<T, BridgeError>
where
    T: DeserializeOwned,
{
    let value = params.ok_or_else(|| BridgeError {
        code: "invalid_request".into(),
        message: format!("Method {method} requires params"),
    })?;

    serde_json::from_value(value).map_err(|error| BridgeError {
        code: "invalid_request".into(),
        message: error.to_string(),
    })
}

fn serialize_result<T>(value: T) -> Result<Value, BridgeError>
where
    T: Serialize,
{
    serde_json::to_value(value).map_err(|error| BridgeError {
        code: "internal".into(),
        message: error.to_string(),
    })
}

fn get_gpu_inventory() -> GpuInventoryResponse {
    match daemon_client::get_inventory() {
        Ok(inventory) => inventory,
        Err(error) => {
            if backend_requires_daemon() {
                GpuInventoryResponse {
                    gpus: vec![],
                    warnings: vec![format!("Prism daemon is unavailable: {}", error.message)],
                }
            } else {
                let mut inventory = collect_inventory();

                for gpu in &mut inventory.gpus {
                    gpu.control
                        .warnings
                        .push(format!("Prism daemon is unavailable: {}", error.message));
                }

                inventory
            }
        }
    }
}

fn get_gpu_performance() -> GpuPerformanceResponse {
    if backend_requires_daemon() {
        match daemon_client::get_performance() {
            Ok(performance) => performance,
            Err(error) => GpuPerformanceResponse {
                gpus: vec![],
                warnings: vec![format!("Prism daemon is unavailable: {}", error.message)],
            },
        }
    } else {
        collect_performance()
    }
}

fn get_gpu_daemon_status() -> GpuDaemonStatus {
    daemon_client::get_daemon_status()
}

fn set_gpu_power_limit(
    request: SetGpuPowerLimitRequest,
) -> Result<crate::protocol::DaemonCommandResponse, BridgeError> {
    daemon_client::set_gpu_power_limit(request).map_err(BridgeError::from)
}

fn reset_gpu_power_limit(
    request: GpuTargetRequest,
) -> Result<crate::protocol::DaemonCommandResponse, BridgeError> {
    daemon_client::reset_gpu_power_limit(request).map_err(BridgeError::from)
}

fn set_gpu_fan_speed(
    request: SetGpuFanSpeedRequest,
) -> Result<crate::protocol::DaemonCommandResponse, BridgeError> {
    daemon_client::set_gpu_fan_speed(request).map_err(BridgeError::from)
}

fn set_gpu_fan_mode_auto(
    request: SetGpuFanModeAutoRequest,
) -> Result<crate::protocol::DaemonCommandResponse, BridgeError> {
    daemon_client::set_gpu_fan_mode_auto(request).map_err(BridgeError::from)
}

fn set_gpu_clock_offsets(
    request: LiveClockOffsetsRequest,
) -> Result<crate::protocol::ClockOffsetResponse, BridgeError> {
    daemon_client::set_gpu_clock_offsets(request).map_err(|error| BridgeError {
        code: "command_failed".into(),
        message: error.message,
    })
}

fn reset_gpu_clock_offsets(
    request: GpuTargetRequest,
) -> Result<crate::protocol::ClockOffsetResponse, BridgeError> {
    daemon_client::reset_gpu_clock_offsets(request).map_err(|error| BridgeError {
        code: "command_failed".into(),
        message: error.message,
    })
}

fn get_driver_status() -> DriverStatusResponse {
    match daemon_client::get_driver_status() {
        Ok(status) => status,
        Err(error) => {
            let mut status = collect_driver_status(None);
            status.host.supported = false;
            status.host.unsupported_reason =
                Some(format!("Prism daemon is unavailable: {}", error.message));
            status
                .warnings
                .push(format!("Prism daemon is unavailable: {}", error.message));
            status
        }
    }
}

fn start_driver_action(request: StartDriverActionRequest) -> Result<DriverJob, BridgeError> {
    daemon_client::start_driver_action(request).map_err(BridgeError::from)
}

fn get_driver_job(request: DriverJobRequest) -> Result<DriverJob, BridgeError> {
    daemon_client::get_driver_job(request).map_err(BridgeError::from)
}

fn cancel_driver_job(request: DriverJobRequest) -> Result<DriverJob, BridgeError> {
    daemon_client::cancel_driver_job(request).map_err(BridgeError::from)
}

fn request_host_reboot() -> Result<Value, BridgeError> {
    daemon_client::request_host_reboot()?;
    Ok(Value::Null)
}

fn backend_requires_daemon() -> bool {
    env::var_os("PRISM_BACKEND_REQUIRE_DAEMON").is_some() || env::var_os("FLATPAK_ID").is_some()
}

impl From<GpuCommandError> for BridgeError {
    fn from(value: GpuCommandError) -> Self {
        Self {
            code: format_gpu_error_code(value.code).into(),
            message: value.message,
        }
    }
}

impl From<String> for BridgeError {
    fn from(value: String) -> Self {
        Self {
            code: "command_failed".into(),
            message: value,
        }
    }
}

fn format_gpu_error_code(code: GpuCommandErrorCode) -> &'static str {
    match code {
        GpuCommandErrorCode::DaemonUnavailable => "daemon_unavailable",
        GpuCommandErrorCode::NoPermission => "no_permission",
        GpuCommandErrorCode::InvalidArgument => "invalid_argument",
        GpuCommandErrorCode::NotSupported => "not_supported",
        GpuCommandErrorCode::GpuLost => "gpu_lost",
        GpuCommandErrorCode::NvmlUnavailable => "nvml_unavailable",
        GpuCommandErrorCode::Io => "io",
        GpuCommandErrorCode::Internal => "internal",
        GpuCommandErrorCode::NotFound => "not_found",
    }
}

#[cfg(test)]
mod tests {
    use super::{backend_requires_daemon, handle_request, BridgeError, BridgeRequest};
    use crate::gpu::{GpuCommandError, GpuCommandErrorCode};
    use serde_json::json;

    #[test]
    fn unknown_method_returns_error() {
        let response = handle_request(BridgeRequest {
            id: 7,
            method: "gpu.nope".into(),
            params: None,
        });

        assert!(response.result.is_none());
        assert_eq!(response.error.expect("error").code, "not_found");
    }

    #[test]
    fn mutation_methods_require_params() {
        let response = handle_request(BridgeRequest {
            id: 8,
            method: "gpu.setPowerLimit".into(),
            params: None,
        });

        assert!(response.result.is_none());
        assert_eq!(response.error.expect("error").code, "invalid_request");
    }

    #[test]
    fn mutation_methods_reject_malformed_params() {
        let response = handle_request(BridgeRequest {
            id: 9,
            method: "gpu.setPowerLimit".into(),
            params: Some(json!({ "gpuId": "GPU-123", "limitWatts": "450" })),
        });

        assert!(response.result.is_none());
        assert_eq!(response.error.expect("error").code, "invalid_request");
    }

    #[test]
    fn profile_application_rejects_malformed_included_instructions_before_transport() {
        for settings in [
            json!({ "powerLimitWatts": null }),
            json!({ "powerLimitWatts": 450 }),
            json!({ "powerLimitWatts": { "kind": "value", "value": "450" } }),
            json!({ "powerLimitWatts": { "kind": "value" } }),
            json!({ "graphicsOffsetMHz": { "kind": "value", "value": 120.5 } }),
            json!({ "memoryOffsetMHz": { "kind": "default", "value": 0 } }),
            json!({ "memoryOffsetMHz": { "kind": "reset" } }),
            json!({ "powerLimitWatts": { "kind": "default" }, "fanSpeed": 80 }),
            json!({ "gpuClockLock": { "kind": "value", "value": 1800 } }),
        ] {
            let response = handle_request(BridgeRequest {
                id: 14,
                method: "gpu.applyProfile".into(),
                params: Some(json!({ "gpuId": "GPU-123", "settings": settings })),
            });
            assert!(response.result.is_none());
            assert_eq!(
                response.error.expect("error").code,
                "invalid_request",
                "{settings}"
            );
        }
        let response = handle_request(BridgeRequest {
            id: 14,
            method: "gpu.applyProfile".into(),
            params: None,
        });
        assert_eq!(response.error.expect("error").code, "invalid_request");
    }

    #[test]
    fn gpu_error_codes_convert_to_bridge_error_codes() {
        let error: BridgeError =
            GpuCommandError::new(GpuCommandErrorCode::NoPermission, "permission denied").into();

        assert_eq!(error.code, "no_permission");
        assert_eq!(error.message, "permission denied");
    }

    #[test]
    fn flatpak_mode_requires_daemon() {
        std::env::set_var("PRISM_BACKEND_REQUIRE_DAEMON", "1");
        assert!(backend_requires_daemon());
        std::env::remove_var("PRISM_BACKEND_REQUIRE_DAEMON");
    }
}
