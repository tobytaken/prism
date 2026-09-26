use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use crate::daemon::DEFAULT_SOCKET_PATH;
use crate::drivers::{DriverJob, DriverJobRequest, DriverStatusResponse, StartDriverActionRequest};
use crate::gpu::{
    ApplyGpuConfigRequest, GpuCommandError, GpuCommandErrorCode, GpuDaemonStatus,
    GpuInventoryResponse, GpuMutationResponse, GpuPerformanceResponse, GpuTargetRequest,
    ResetGpuConfigRequest, SetGpuClockOffsetsRequest, SetGpuFanModeAutoRequest,
    SetGpuFanSpeedRequest, SetGpuPowerLimitRequest,
};
use crate::protocol::{ClockOffsetResponse, DaemonCommandResponse, DaemonRequest, DaemonResponse};

pub fn get_inventory() -> Result<GpuInventoryResponse, GpuCommandError> {
    match send_request(DaemonRequest::GetInventory)? {
        DaemonResponse::Inventory(inventory) => Ok(inventory),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("inventory")),
    }
}

pub fn get_performance() -> Result<GpuPerformanceResponse, GpuCommandError> {
    match send_request(DaemonRequest::GetPerformance)? {
        DaemonResponse::Performance(performance) => Ok(performance),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("performance")),
    }
}

pub fn get_daemon_status() -> GpuDaemonStatus {
    match send_request(DaemonRequest::GetDaemonStatus) {
        Ok(DaemonResponse::DaemonStatus(status)) => status,
        Ok(DaemonResponse::Error(error)) => {
            GpuDaemonStatus::disconnected(socket_path(), Some(error.message))
        }
        Ok(_) => GpuDaemonStatus::disconnected(
            socket_path(),
            Some("Prism daemon returned an unexpected response".into()),
        ),
        Err(error) => GpuDaemonStatus::disconnected(socket_path(), Some(error.message)),
    }
}

pub fn apply_gpu_config(request: ApplyGpuConfigRequest) -> GpuMutationResponse {
    match send_request(DaemonRequest::ApplyConfig(request)) {
        Ok(DaemonResponse::Mutation(response)) => response,
        Ok(DaemonResponse::Error(error)) => GpuMutationResponse::failure(error),
        Ok(_) => GpuMutationResponse::failure(unexpected_response_error("mutation")),
        Err(error) => GpuMutationResponse::failure(error),
    }
}

pub fn reset_gpu_config(request: ResetGpuConfigRequest) -> GpuMutationResponse {
    match send_request(DaemonRequest::ResetConfig(request)) {
        Ok(DaemonResponse::Mutation(response)) => response,
        Ok(DaemonResponse::Error(error)) => GpuMutationResponse::failure(error),
        Ok(_) => GpuMutationResponse::failure(unexpected_response_error("mutation")),
        Err(error) => GpuMutationResponse::failure(error),
    }
}

pub fn reapply_saved_gpu_config() -> GpuMutationResponse {
    match send_request(DaemonRequest::ReapplySavedConfig) {
        Ok(DaemonResponse::Mutation(response)) => response,
        Ok(DaemonResponse::Error(error)) => GpuMutationResponse::failure(error),
        Ok(_) => GpuMutationResponse::failure(unexpected_response_error("mutation")),
        Err(error) => GpuMutationResponse::failure(error),
    }
}

pub fn set_gpu_power_limit(
    request: SetGpuPowerLimitRequest,
) -> Result<DaemonCommandResponse, GpuCommandError> {
    send_gpu_command(DaemonRequest::SetPowerLimit(request))
}

pub fn reset_gpu_power_limit(
    request: GpuTargetRequest,
) -> Result<DaemonCommandResponse, GpuCommandError> {
    send_gpu_command(DaemonRequest::ResetPowerLimit(request))
}

pub fn set_gpu_fan_speed(
    request: SetGpuFanSpeedRequest,
) -> Result<DaemonCommandResponse, GpuCommandError> {
    send_gpu_command(DaemonRequest::SetFanSpeed(request))
}

pub fn set_gpu_fan_mode_auto(
    request: SetGpuFanModeAutoRequest,
) -> Result<DaemonCommandResponse, GpuCommandError> {
    send_gpu_command(DaemonRequest::SetFanModeAuto(request))
}

pub fn set_gpu_clock_offsets(
    request: SetGpuClockOffsetsRequest,
) -> Result<ClockOffsetResponse, GpuCommandError> {
    send_offset_request(DaemonRequest::SetClockOffsets(request))
}

pub fn reset_gpu_clock_offsets(
    request: GpuTargetRequest,
) -> Result<ClockOffsetResponse, GpuCommandError> {
    send_offset_request(DaemonRequest::ResetClockOffsets(request))
}

pub fn get_driver_status() -> Result<DriverStatusResponse, GpuCommandError> {
    match send_request(DaemonRequest::GetDriverStatus)? {
        DaemonResponse::DriverStatus(status) => Ok(status),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("driver status")),
    }
}

pub fn start_driver_action(
    request: StartDriverActionRequest,
) -> Result<DriverJob, GpuCommandError> {
    match send_request(DaemonRequest::StartDriverAction(request))? {
        DaemonResponse::DriverJob(job) => Ok(job),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("driver job")),
    }
}

pub fn get_driver_job(request: DriverJobRequest) -> Result<DriverJob, GpuCommandError> {
    match send_request(DaemonRequest::GetDriverJob(request))? {
        DaemonResponse::DriverJob(job) => Ok(job),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("driver job")),
    }
}

pub fn cancel_driver_job(request: DriverJobRequest) -> Result<DriverJob, GpuCommandError> {
    match send_request(DaemonRequest::CancelDriverJob(request))? {
        DaemonResponse::DriverJob(job) => Ok(job),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("driver job")),
    }
}

pub fn request_host_reboot() -> Result<(), String> {
    send_command_request(DaemonRequest::RequestHostReboot).map_err(|error| error.message)
}

fn send_request(request: DaemonRequest) -> Result<DaemonResponse, GpuCommandError> {
    let mut stream = UnixStream::connect(socket_path()).map_err(map_io_error)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(map_io_error)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(map_io_error)?;

    let payload = serde_json::to_vec(&request)
        .map_err(|error| GpuCommandError::new(GpuCommandErrorCode::Internal, error.to_string()))?;

    stream.write_all(&payload).map_err(map_io_error)?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(map_io_error)?;

    let mut response = Vec::new();
    stream.read_to_end(&mut response).map_err(map_io_error)?;

    serde_json::from_slice(&response)
        .map_err(|error| GpuCommandError::new(GpuCommandErrorCode::Internal, error.to_string()))
}

fn send_command_request(request: DaemonRequest) -> Result<(), GpuCommandError> {
    match send_request(request)? {
        DaemonResponse::Command(response) => {
            if response.success {
                Ok(())
            } else {
                Err(GpuCommandError::new(
                    GpuCommandErrorCode::Internal,
                    response.error.unwrap_or_else(|| {
                        "Prism daemon failed to apply the requested change".into()
                    }),
                ))
            }
        }
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("command")),
    }
}

fn socket_path() -> String {
    env::var("PRISM_SOCKET_PATH").unwrap_or_else(|_| DEFAULT_SOCKET_PATH.to_string())
}

fn map_io_error(error: std::io::Error) -> GpuCommandError {
    let code = match error.kind() {
        std::io::ErrorKind::NotFound
        | std::io::ErrorKind::ConnectionRefused
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::ConnectionAborted
        | std::io::ErrorKind::TimedOut => GpuCommandErrorCode::DaemonUnavailable,
        std::io::ErrorKind::PermissionDenied => GpuCommandErrorCode::NoPermission,
        _ => GpuCommandErrorCode::Io,
    };

    GpuCommandError::new(code, error.to_string())
}

fn unexpected_response_error(expected: &str) -> GpuCommandError {
    GpuCommandError::new(
        GpuCommandErrorCode::Internal,
        format!("Prism daemon returned an unexpected response for {expected}"),
    )
}

fn send_offset_request(request: DaemonRequest) -> Result<ClockOffsetResponse, GpuCommandError> {
    match send_request(request)? {
        DaemonResponse::ClockOffsets(response) => Ok(response),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("clock offsets")),
    }
}

fn send_gpu_command(request: DaemonRequest) -> Result<DaemonCommandResponse, GpuCommandError> {
    match send_request(request)? {
        DaemonResponse::Command(response) => Ok(response),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("GPU command")),
    }
}
