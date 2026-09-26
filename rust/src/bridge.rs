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
    collect_inventory, collect_performance, ApplyGpuConfigRequest, GpuCommandError,
    GpuCommandErrorCode, GpuDaemonStatus, GpuInventoryResponse, GpuMutationResponse,
    GpuPerformanceResponse, GpuTargetRequest, ResetGpuConfigRequest, SetGpuClockOffsetsRequest,
    SetGpuFanModeAutoRequest, SetGpuFanSpeedRequest, SetGpuPowerLimitRequest,
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
        "gpu.getInventory" => serialize_result(get_gpu_inventory()),
        "gpu.getPerformance" => serialize_result(get_gpu_performance()),
        "gpu.getDaemonStatus" => serialize_result(get_gpu_daemon_status()),
        "gpu.applyConfig" => parse_and_serialize::<ApplyGpuConfigRequest, _, _>(
            request.params,
            "gpu.applyConfig",
            apply_gpu_config,
        ),
        "gpu.resetConfig" => parse_and_serialize::<ResetGpuConfigRequest, _, _>(
            request.params,
            "gpu.resetConfig",
            reset_gpu_config,
        ),
        "gpu.setPowerLimit" => parse_and_execute::<SetGpuPowerLimitRequest, _>(
            request.params,
            "gpu.setPowerLimit",
            set_gpu_power_limit,
        ),
        "gpu.resetPowerLimit" => parse_and_execute::<GpuTargetRequest, _>(
            request.params,
            "gpu.resetPowerLimit",
            reset_gpu_power_limit,
        ),
        "gpu.setFanSpeed" => parse_and_execute::<SetGpuFanSpeedRequest, _>(
            request.params,
            "gpu.setFanSpeed",
            set_gpu_fan_speed,
        ),
        "gpu.setFanModeAuto" => parse_and_execute::<SetGpuFanModeAutoRequest, _>(
            request.params,
            "gpu.setFanModeAuto",
            set_gpu_fan_mode_auto,
        ),
        "gpu.setClockOffsets" => parse_and_execute::<SetGpuClockOffsetsRequest, _>(
            request.params,
            "gpu.setClockOffsets",
            set_gpu_clock_offsets,
        ),
        "gpu.resetClockOffsets" => parse_and_execute::<GpuTargetRequest, _>(
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

fn parse_and_serialize<T, F, R>(
    params: Option<Value>,
    method: &str,
    handler: F,
) -> Result<Value, BridgeError>
where
    T: DeserializeOwned,
    R: Serialize,
    F: FnOnce(T) -> R,
{
    let payload = parse_params(params, method)?;
    serialize_result(handler(payload))
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

fn parse_and_execute<T, F>(
    params: Option<Value>,
    method: &str,
    handler: F,
) -> Result<Value, BridgeError>
where
    T: DeserializeOwned,
    F: FnOnce(T) -> Result<(), String>,
{
    let payload = parse_params(params, method)?;
    handler(payload)?;
    Ok(Value::Null)
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

fn apply_gpu_config(request: ApplyGpuConfigRequest) -> GpuMutationResponse {
    if request.reapply_saved {
        daemon_client::reapply_saved_gpu_config()
    } else {
        daemon_client::apply_gpu_config(request)
    }
}

fn reset_gpu_config(request: ResetGpuConfigRequest) -> GpuMutationResponse {
    daemon_client::reset_gpu_config(request)
}

fn set_gpu_power_limit(request: SetGpuPowerLimitRequest) -> Result<(), String> {
    daemon_client::set_gpu_power_limit(request)
}

fn reset_gpu_power_limit(request: GpuTargetRequest) -> Result<(), String> {
    daemon_client::reset_gpu_power_limit(request)
}

fn set_gpu_fan_speed(request: SetGpuFanSpeedRequest) -> Result<(), String> {
    daemon_client::set_gpu_fan_speed(request)
}

fn set_gpu_fan_mode_auto(request: SetGpuFanModeAutoRequest) -> Result<(), String> {
    daemon_client::set_gpu_fan_mode_auto(request)
}

fn set_gpu_clock_offsets(request: SetGpuClockOffsetsRequest) -> Result<(), String> {
    daemon_client::set_gpu_clock_offsets(request)
}

fn reset_gpu_clock_offsets(request: GpuTargetRequest) -> Result<(), String> {
    daemon_client::reset_gpu_clock_offsets(request)
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
            method: "gpu.applyConfig".into(),
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
    fn reapply_saved_config_returns_mutation_response_when_daemon_is_unavailable() {
        std::env::set_var(
            "PRISM_SOCKET_PATH",
            "/tmp/prism-bridge-test-missing/prismd.sock",
        );

        let response = handle_request(BridgeRequest {
            id: 10,
            method: "gpu.applyConfig".into(),
            params: Some(json!({ "reapplySaved": true })),
        });

        std::env::remove_var("PRISM_SOCKET_PATH");

        let result = response.result.expect("result");
        assert!(response.error.is_none());
        assert_eq!(result["success"], false);
        assert_eq!(result["inventory"], serde_json::Value::Null);
        assert_eq!(result["error"]["code"], "daemonUnavailable");
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
