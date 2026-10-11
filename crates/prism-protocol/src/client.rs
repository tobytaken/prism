use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use crate::{
    AddSteamGameRequest, GameRequest, GamesState, SetAutomaticSwitchingRequest,
    SetGameProfileRequest, SteamApp,
};
use crate::{
    ApplyGpuProfileRequest, ClockOffsetResponse, CorrectAndApplyProfileRequest,
    CorrectedProfileApplication, DaemonCommandResponse, DaemonRequest, DaemonResponse,
    GpuTuningProfile, GpuTuningState, LiveClockOffsetsRequest, ProfileApplicationResponse,
    ProfileIdRequest, RenameProfileRequest, SaveProfileAsNewRequest, SaveToCurrentProfileRequest,
    SetStartupBehaviorRequest, SetStartupProfileRequest, StartupGpuRequest, StartupState,
};
use crate::{DEFAULT_SOCKET_PATH, GpuDaemonStatus};
use prism_drivers::{DriverJob, DriverJobRequest, DriverStatusResponse, StartDriverActionRequest};
use prism_gpu::cooling::{ApplyCoolingRequest, CoolingApplication};
use prism_gpu::fan_curve::{FanCurvePreview, PreviewFanCurveRequest};
use prism_gpu::{
    GpuCommandError, GpuCommandErrorCode, GpuInventoryResponse, GpuTargetRequest,
    SetGpuFanModeAutoRequest, SetGpuFanSpeedRequest, SetGpuPowerLimitRequest,
};

mod session;
pub use session::{ConnectError, Connected, Session, SessionSender, connect};

/// Why a request to prismd failed: prismd didn't answer, or the request failed with one of the
/// codes prismd itself reports, whether prismd sent it or the client hit it locally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestError {
    pub code: RequestErrorCode,
    pub message: String,
}

impl RequestError {
    pub fn new(code: impl Into<RequestErrorCode>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl From<GpuCommandError> for RequestError {
    fn from(error: GpuCommandError) -> Self {
        Self::new(error.code, error.message)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestErrorCode {
    /// prismd isn't listening on its socket, or stopped answering.
    DaemonUnavailable,
    Command(GpuCommandErrorCode),
}

impl From<GpuCommandErrorCode> for RequestErrorCode {
    fn from(code: GpuCommandErrorCode) -> Self {
        Self::Command(code)
    }
}

pub fn apply_gpu_profile(
    request: ApplyGpuProfileRequest,
) -> Result<ProfileApplicationResponse, RequestError> {
    match send_request(DaemonRequest::ApplyProfile(request))? {
        DaemonResponse::ProfileApplication(response) => Ok(response),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("profile application")),
    }
}

pub fn list_gpu_profiles() -> Result<Vec<GpuTuningProfile>, RequestError> {
    match send_request(DaemonRequest::ListProfiles)? {
        DaemonResponse::Profiles(profiles) => Ok(profiles),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("profiles")),
    }
}

pub fn rename_gpu_profile(request: RenameProfileRequest) -> Result<GpuTuningProfile, RequestError> {
    match send_request(DaemonRequest::RenameProfile(request))? {
        DaemonResponse::Profile(profile) => Ok(profile),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("renamed profile")),
    }
}

pub fn delete_gpu_profile(
    request: ProfileIdRequest,
) -> Result<DaemonCommandResponse, RequestError> {
    match send_request(DaemonRequest::DeleteProfile(request))? {
        DaemonResponse::Command(response) => Ok(response),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("profile deletion")),
    }
}

pub fn list_running_programs() -> Result<Vec<crate::RunningProgram>, RequestError> {
    match send_request(DaemonRequest::ListRunningPrograms)? {
        DaemonResponse::RunningPrograms(programs) => Ok(programs),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("running programs")),
    }
}

pub fn add_program(request: crate::AddProgramRequest) -> Result<GamesState, RequestError> {
    match send_request(DaemonRequest::AddProgram(request))? {
        DaemonResponse::Games(state) => Ok(state),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("games")),
    }
}

pub fn list_steam_apps() -> Result<Vec<SteamApp>, RequestError> {
    match send_request(DaemonRequest::ListSteamApps)? {
        DaemonResponse::SteamApps(apps) => Ok(apps),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("Steam apps")),
    }
}

pub fn get_running_games() -> Result<crate::RunningGamesState, RequestError> {
    match send_request(DaemonRequest::GetRunningGames)? {
        DaemonResponse::RunningGames(state) => Ok(state),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("running games")),
    }
}

pub fn set_edit_holds(
    request: crate::SetEditHoldsRequest,
) -> Result<DaemonCommandResponse, RequestError> {
    send_gpu_command(DaemonRequest::SetEditHolds(request))
}

pub fn get_automatic_switching_status() -> Result<crate::AutomaticSwitchingStatus, RequestError> {
    match send_request(DaemonRequest::GetAutomaticSwitchingStatus)? {
        DaemonResponse::AutomaticSwitchingStatus(status) => Ok(status),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("automatic switching status")),
    }
}

pub fn retry_automatic_switching(
    request: crate::StartupGpuRequest,
) -> Result<crate::AutomaticSwitchingStatus, RequestError> {
    match send_request(DaemonRequest::RetryAutomaticSwitching(request))? {
        DaemonResponse::AutomaticSwitchingStatus(status) => Ok(status),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("automatic switching retry")),
    }
}

pub fn set_gpu_switching_paused(
    request: crate::SetGpuSwitchingPausedRequest,
) -> Result<DaemonCommandResponse, RequestError> {
    send_gpu_command(DaemonRequest::SetGpuSwitchingPaused(request))
}

pub fn get_games() -> Result<GamesState, RequestError> {
    send_games_request(DaemonRequest::GetGames)
}

pub fn add_steam_game(request: AddSteamGameRequest) -> Result<GamesState, RequestError> {
    send_games_request(DaemonRequest::AddSteamGame(request))
}

pub fn set_game_profile(request: SetGameProfileRequest) -> Result<GamesState, RequestError> {
    send_games_request(DaemonRequest::SetGameProfile(request))
}

pub fn remove_game(request: GameRequest) -> Result<GamesState, RequestError> {
    send_games_request(DaemonRequest::RemoveGame(request))
}

pub fn set_automatic_switching(
    request: SetAutomaticSwitchingRequest,
) -> Result<GamesState, RequestError> {
    send_games_request(DaemonRequest::SetAutomaticSwitching(request))
}

fn send_games_request(request: DaemonRequest) -> Result<GamesState, RequestError> {
    match send_request(request)? {
        DaemonResponse::Games(state) => Ok(state),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("games")),
    }
}

pub fn get_gpu_tuning_states() -> Result<Vec<GpuTuningState>, RequestError> {
    match send_request(DaemonRequest::GetTuningStates)? {
        DaemonResponse::TuningStates(states) => Ok(states),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("tuning states")),
    }
}

pub fn save_gpu_profile_to_current(
    request: SaveToCurrentProfileRequest,
) -> Result<GpuTuningProfile, RequestError> {
    match send_request(DaemonRequest::SaveToCurrentProfile(request))? {
        DaemonResponse::Profile(profile) => Ok(profile),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("saved profile")),
    }
}

pub fn save_gpu_profile_as_new(
    request: SaveProfileAsNewRequest,
) -> Result<GpuTuningProfile, RequestError> {
    match send_request(DaemonRequest::SaveProfileAsNew(request))? {
        DaemonResponse::Profile(profile) => Ok(profile),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("saved profile")),
    }
}

pub fn apply_gpu_profile_by_id(
    request: ProfileIdRequest,
) -> Result<ProfileApplicationResponse, RequestError> {
    match send_request(DaemonRequest::ApplyProfileById(request))? {
        DaemonResponse::ProfileApplication(response) => Ok(response),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("profile application")),
    }
}

pub fn correct_and_apply_gpu_profile(
    request: CorrectAndApplyProfileRequest,
) -> Result<CorrectedProfileApplication, RequestError> {
    match send_request(DaemonRequest::CorrectAndApplyProfile(request))? {
        DaemonResponse::CorrectedProfileApplication(response) => Ok(response),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("corrected profile application")),
    }
}

pub fn get_inventory() -> Result<GpuInventoryResponse, RequestError> {
    match send_request(DaemonRequest::GetInventory)? {
        DaemonResponse::Inventory(inventory) => Ok(inventory),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("inventory")),
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

pub fn set_gpu_power_limit(
    request: SetGpuPowerLimitRequest,
) -> Result<DaemonCommandResponse, RequestError> {
    send_gpu_command(DaemonRequest::SetPowerLimit(request))
}

pub fn reset_gpu_power_limit(
    request: GpuTargetRequest,
) -> Result<DaemonCommandResponse, RequestError> {
    send_gpu_command(DaemonRequest::ResetPowerLimit(request))
}

pub fn set_gpu_fan_speed(
    request: SetGpuFanSpeedRequest,
) -> Result<DaemonCommandResponse, RequestError> {
    send_gpu_command(DaemonRequest::SetFanSpeed(request))
}

pub fn set_gpu_fan_mode_auto(
    request: SetGpuFanModeAutoRequest,
) -> Result<DaemonCommandResponse, RequestError> {
    send_gpu_command(DaemonRequest::SetFanModeAuto(request))
}

pub fn preview_fan_curve(request: PreviewFanCurveRequest) -> Result<FanCurvePreview, RequestError> {
    match send_request(DaemonRequest::PreviewFanCurve(request))? {
        DaemonResponse::FanCurvePreview(preview) => Ok(preview),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("fan curve preview")),
    }
}

pub fn apply_cooling(request: ApplyCoolingRequest) -> Result<CoolingApplication, RequestError> {
    match send_request(DaemonRequest::ApplyCooling(request))? {
        DaemonResponse::CoolingApplication(application) => Ok(application),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("cooling application")),
    }
}

pub fn set_gpu_clock_offsets(
    request: LiveClockOffsetsRequest,
) -> Result<ClockOffsetResponse, RequestError> {
    send_offset_request(DaemonRequest::SetClockOffsets(request))
}

pub fn reset_gpu_clock_offsets(
    request: GpuTargetRequest,
) -> Result<ClockOffsetResponse, RequestError> {
    send_offset_request(DaemonRequest::ResetClockOffsets(request))
}

pub fn get_driver_status() -> Result<DriverStatusResponse, RequestError> {
    match send_request(DaemonRequest::GetDriverStatus)? {
        DaemonResponse::DriverStatus(status) => Ok(status),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("driver status")),
    }
}

pub fn start_driver_action(request: StartDriverActionRequest) -> Result<DriverJob, RequestError> {
    match send_request(DaemonRequest::StartDriverAction(request))? {
        DaemonResponse::DriverJob(job) => Ok(job),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("driver job")),
    }
}

pub fn get_driver_job(request: DriverJobRequest) -> Result<DriverJob, RequestError> {
    match send_request(DaemonRequest::GetDriverJob(request))? {
        DaemonResponse::DriverJob(job) => Ok(job),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("driver job")),
    }
}

pub fn cancel_driver_job(request: DriverJobRequest) -> Result<DriverJob, RequestError> {
    match send_request(DaemonRequest::CancelDriverJob(request))? {
        DaemonResponse::DriverJob(job) => Ok(job),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("driver job")),
    }
}

pub fn request_host_reboot() -> Result<(), String> {
    send_command_request(DaemonRequest::RequestHostReboot).map_err(|error| error.message)
}

/// GPU operations can be queued behind startup's roughly 30-second arrival wait.
/// Game requests read the caller's Steam libraries.
fn request_timeout(request: &DaemonRequest) -> Duration {
    if matches!(
        request,
        DaemonRequest::ApplyProfile(_)
            | DaemonRequest::SetPowerLimit(_)
            | DaemonRequest::ResetPowerLimit(_)
            | DaemonRequest::SetFanSpeed(_)
            | DaemonRequest::SetFanModeAuto(_)
            | DaemonRequest::SetClockOffsets(_)
            | DaemonRequest::ResetClockOffsets(_)
            | DaemonRequest::SaveProfileAsNew(_)
            | DaemonRequest::SaveToCurrentProfile(_)
            | DaemonRequest::RenameProfile(_)
            | DaemonRequest::DeleteProfile(_)
            | DaemonRequest::ApplyProfileById(_)
            | DaemonRequest::CorrectAndApplyProfile(_)
            | DaemonRequest::SetStartupProfile(_)
            | DaemonRequest::SetGpuSwitchingPaused(_)
    ) {
        Duration::from_secs(35)
    } else if request.is_game_request() {
        // Reading Steam libraries can wait on slow or removable disks.
        Duration::from_secs(10)
    } else {
        Duration::from_secs(2)
    }
}

fn send_request(request: DaemonRequest) -> Result<DaemonResponse, RequestError> {
    let mut stream = UnixStream::connect(socket_path()).map_err(map_io_error)?;
    stream
        .set_read_timeout(Some(request_timeout(&request)))
        .map_err(map_io_error)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(map_io_error)?;

    let payload = serde_json::to_vec(&request)
        .map_err(|error| RequestError::new(GpuCommandErrorCode::Internal, error.to_string()))?;

    stream.write_all(&payload).map_err(map_io_error)?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(map_io_error)?;

    let mut response = Vec::new();
    stream.read_to_end(&mut response).map_err(map_io_error)?;

    serde_json::from_slice(&response)
        .map_err(|error| RequestError::new(GpuCommandErrorCode::Internal, error.to_string()))
}

fn send_command_request(request: DaemonRequest) -> Result<(), RequestError> {
    match send_request(request)? {
        DaemonResponse::Command(response) => {
            if response.success {
                Ok(())
            } else {
                Err(RequestError::new(
                    GpuCommandErrorCode::Internal,
                    response.error.unwrap_or_else(|| {
                        "Prism daemon failed to apply the requested change".into()
                    }),
                ))
            }
        }
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("command")),
    }
}

/// Where prismd listens: `PRISM_SOCKET_PATH`, or the installed service's socket.
pub fn socket_path() -> String {
    env::var("PRISM_SOCKET_PATH").unwrap_or_else(|_| DEFAULT_SOCKET_PATH.to_string())
}

fn map_io_error(error: std::io::Error) -> RequestError {
    let code = match error.kind() {
        std::io::ErrorKind::NotFound
        | std::io::ErrorKind::ConnectionRefused
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::ConnectionAborted
        | std::io::ErrorKind::TimedOut => RequestErrorCode::DaemonUnavailable,
        std::io::ErrorKind::PermissionDenied => GpuCommandErrorCode::NoPermission.into(),
        _ => GpuCommandErrorCode::Io.into(),
    };

    RequestError::new(code, error.to_string())
}

fn unexpected_response_error(expected: &str) -> RequestError {
    RequestError::new(
        GpuCommandErrorCode::Internal,
        format!("Prism daemon returned an unexpected response for {expected}"),
    )
}

fn send_offset_request(request: DaemonRequest) -> Result<ClockOffsetResponse, RequestError> {
    match send_request(request)? {
        DaemonResponse::ClockOffsets(response) => Ok(response),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("clock offsets")),
    }
}

fn send_gpu_command(request: DaemonRequest) -> Result<DaemonCommandResponse, RequestError> {
    match send_request(request)? {
        DaemonResponse::Command(response) => Ok(response),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("GPU command")),
    }
}

pub fn get_startup_state() -> Result<StartupState, RequestError> {
    match send_request(DaemonRequest::GetStartupState)? {
        DaemonResponse::StartupState(state) => Ok(state),
        DaemonResponse::Error(error) => Err(error.into()),
        _ => Err(unexpected_response_error("startup state")),
    }
}

pub fn set_startup_behavior(
    request: SetStartupBehaviorRequest,
) -> Result<DaemonCommandResponse, RequestError> {
    send_gpu_command(DaemonRequest::SetStartupBehavior(request))
}

pub fn set_startup_profile(
    request: SetStartupProfileRequest,
) -> Result<DaemonCommandResponse, RequestError> {
    send_gpu_command(DaemonRequest::SetStartupProfile(request))
}

pub fn dismiss_startup_outcome(
    request: StartupGpuRequest,
) -> Result<DaemonCommandResponse, RequestError> {
    send_gpu_command(DaemonRequest::DismissStartupOutcome(request))
}

#[cfg(test)]
mod tests {
    use std::io;

    use prism_gpu::{GpuCommandError, GpuCommandErrorCode};

    use super::{RequestError, RequestErrorCode, map_io_error};

    #[test]
    fn a_socket_nobody_answers_means_the_daemon_is_unavailable() {
        for kind in [
            io::ErrorKind::NotFound,
            io::ErrorKind::ConnectionRefused,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::TimedOut,
        ] {
            assert_eq!(
                map_io_error(io::Error::from(kind)).code,
                RequestErrorCode::DaemonUnavailable,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_socket_the_user_may_not_open_is_a_permission_error() {
        assert_eq!(
            map_io_error(io::Error::from(io::ErrorKind::PermissionDenied)).code,
            RequestErrorCode::Command(GpuCommandErrorCode::NoPermission)
        );
    }

    #[test]
    fn an_error_the_daemon_reports_keeps_its_code_and_message() {
        let error = RequestError::from(GpuCommandError::new(
            GpuCommandErrorCode::NotFound,
            "No GPU tuning profile has that id",
        ));
        assert_eq!(
            error,
            RequestError {
                code: RequestErrorCode::Command(GpuCommandErrorCode::NotFound),
                message: "No GPU tuning profile has that id".into(),
            }
        );
    }
}
