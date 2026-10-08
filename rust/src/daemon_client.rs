use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use crate::cooling::{ApplyCoolingRequest, CoolingApplication};
use crate::daemon::DEFAULT_SOCKET_PATH;
use crate::drivers::{DriverJob, DriverJobRequest, DriverStatusResponse, StartDriverActionRequest};
use crate::fan_curve::{FanCurvePreview, PreviewFanCurveRequest};
use crate::gpu::{
    GpuCommandError, GpuCommandErrorCode, GpuDaemonStatus, GpuInventoryResponse,
    GpuPerformanceResponse, GpuTargetRequest, SetGpuFanModeAutoRequest, SetGpuFanSpeedRequest,
    SetGpuPowerLimitRequest,
};
use crate::protocol::{
    AddSteamGameRequest, GameRequest, GamesState, SetAutomaticSwitchingRequest,
    SetGameProfileRequest, SteamApp,
};
use crate::protocol::{
    ApplyGpuProfileRequest, ClockOffsetResponse, CorrectAndApplyProfileRequest,
    CorrectedProfileApplication, DaemonCommandResponse, DaemonRequest, DaemonResponse,
    GpuTuningProfile, GpuTuningState, ImportProfilesRequest, LiveClockOffsetsRequest,
    ProfileApplicationResponse, ProfileIdRequest, ProfileImportResponse, RenameProfileRequest,
    SaveProfileAsNewRequest, SaveToCurrentProfileRequest, SetStartupBehaviorRequest,
    SetStartupProfileRequest, StartupGpuRequest, StartupState,
};

mod session;
pub use session::{start_session, SessionNotification};

pub fn apply_gpu_profile(
    request: ApplyGpuProfileRequest,
) -> Result<ProfileApplicationResponse, GpuCommandError> {
    match send_request(DaemonRequest::ApplyProfile(request))? {
        DaemonResponse::ProfileApplication(response) => Ok(response),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("profile application")),
    }
}

pub fn list_gpu_profiles() -> Result<Vec<GpuTuningProfile>, GpuCommandError> {
    match send_request(DaemonRequest::ListProfiles)? {
        DaemonResponse::Profiles(profiles) => Ok(profiles),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("profiles")),
    }
}

pub fn rename_gpu_profile(
    request: RenameProfileRequest,
) -> Result<GpuTuningProfile, GpuCommandError> {
    match send_request(DaemonRequest::RenameProfile(request))? {
        DaemonResponse::Profile(profile) => Ok(profile),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("renamed profile")),
    }
}

pub fn delete_gpu_profile(
    request: ProfileIdRequest,
) -> Result<DaemonCommandResponse, GpuCommandError> {
    match send_request(DaemonRequest::DeleteProfile(request))? {
        DaemonResponse::Command(response) => Ok(response),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("profile deletion")),
    }
}

pub fn list_steam_apps() -> Result<Vec<SteamApp>, GpuCommandError> {
    match send_request(DaemonRequest::ListSteamApps)? {
        DaemonResponse::SteamApps(apps) => Ok(apps),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("Steam apps")),
    }
}

pub fn get_running_games() -> Result<crate::protocol::RunningGamesState, GpuCommandError> {
    match send_request(DaemonRequest::GetRunningGames)? {
        DaemonResponse::RunningGames(state) => Ok(state),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("running games")),
    }
}

pub fn get_automatic_switching_status(
) -> Result<crate::protocol::AutomaticSwitchingStatus, GpuCommandError> {
    match send_request(DaemonRequest::GetAutomaticSwitchingStatus)? {
        DaemonResponse::AutomaticSwitchingStatus(status) => Ok(status),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("automatic switching status")),
    }
}

pub fn get_games() -> Result<GamesState, GpuCommandError> {
    send_games_request(DaemonRequest::GetGames)
}

pub fn add_steam_game(request: AddSteamGameRequest) -> Result<GamesState, GpuCommandError> {
    send_games_request(DaemonRequest::AddSteamGame(request))
}

pub fn set_game_profile(request: SetGameProfileRequest) -> Result<GamesState, GpuCommandError> {
    send_games_request(DaemonRequest::SetGameProfile(request))
}

pub fn remove_game(request: GameRequest) -> Result<GamesState, GpuCommandError> {
    send_games_request(DaemonRequest::RemoveGame(request))
}

pub fn set_automatic_switching(
    request: SetAutomaticSwitchingRequest,
) -> Result<GamesState, GpuCommandError> {
    send_games_request(DaemonRequest::SetAutomaticSwitching(request))
}

fn send_games_request(request: DaemonRequest) -> Result<GamesState, GpuCommandError> {
    match send_request(request)? {
        DaemonResponse::Games(state) => Ok(state),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("games")),
    }
}

pub fn get_gpu_tuning_states() -> Result<Vec<GpuTuningState>, GpuCommandError> {
    match send_request(DaemonRequest::GetTuningStates)? {
        DaemonResponse::TuningStates(states) => Ok(states),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("tuning states")),
    }
}

pub fn save_gpu_profile_to_current(
    request: SaveToCurrentProfileRequest,
) -> Result<GpuTuningProfile, GpuCommandError> {
    match send_request(DaemonRequest::SaveToCurrentProfile(request))? {
        DaemonResponse::Profile(profile) => Ok(profile),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("saved profile")),
    }
}

pub fn save_gpu_profile_as_new(
    request: SaveProfileAsNewRequest,
) -> Result<GpuTuningProfile, GpuCommandError> {
    match send_request(DaemonRequest::SaveProfileAsNew(request))? {
        DaemonResponse::Profile(profile) => Ok(profile),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("saved profile")),
    }
}

pub fn apply_gpu_profile_by_id(
    request: ProfileIdRequest,
) -> Result<ProfileApplicationResponse, GpuCommandError> {
    match send_request(DaemonRequest::ApplyProfileById(request))? {
        DaemonResponse::ProfileApplication(response) => Ok(response),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("profile application")),
    }
}

pub fn correct_and_apply_gpu_profile(
    request: CorrectAndApplyProfileRequest,
) -> Result<CorrectedProfileApplication, GpuCommandError> {
    match send_request(DaemonRequest::CorrectAndApplyProfile(request))? {
        DaemonResponse::CorrectedProfileApplication(response) => Ok(response),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("corrected profile application")),
    }
}

pub fn import_gpu_profiles(
    request: ImportProfilesRequest,
) -> Result<ProfileImportResponse, GpuCommandError> {
    match send_request(DaemonRequest::ImportProfiles(request))? {
        DaemonResponse::ProfileImport(response) => Ok(response),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("profile import")),
    }
}

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

pub fn preview_fan_curve(
    request: PreviewFanCurveRequest,
) -> Result<FanCurvePreview, GpuCommandError> {
    match send_request(DaemonRequest::PreviewFanCurve(request))? {
        DaemonResponse::FanCurvePreview(preview) => Ok(preview),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("fan curve preview")),
    }
}

pub fn apply_cooling(request: ApplyCoolingRequest) -> Result<CoolingApplication, GpuCommandError> {
    match send_request(DaemonRequest::ApplyCooling(request))? {
        DaemonResponse::CoolingApplication(application) => Ok(application),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("cooling application")),
    }
}

pub fn set_gpu_clock_offsets(
    request: LiveClockOffsetsRequest,
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
    ) {
        Duration::from_secs(35)
    } else if request.is_game_request() {
        // Reading Steam libraries can wait on slow or removable disks.
        Duration::from_secs(10)
    } else {
        Duration::from_secs(2)
    }
}

fn send_request(request: DaemonRequest) -> Result<DaemonResponse, GpuCommandError> {
    if !matches!(
        request,
        DaemonRequest::GetDaemonStatus
            | DaemonRequest::GetDriverStatus
            | DaemonRequest::StartDriverAction(_)
            | DaemonRequest::GetDriverJob(_)
            | DaemonRequest::CancelDriverJob(_)
            | DaemonRequest::RequestHostReboot
    ) {
        if let Some(session) = session::SESSION.get() {
            return session.request(request);
        }
    }
    let mut stream = UnixStream::connect(socket_path()).map_err(map_io_error)?;
    stream
        .set_read_timeout(Some(request_timeout(&request)))
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

pub fn get_startup_state() -> Result<StartupState, GpuCommandError> {
    match send_request(DaemonRequest::GetStartupState)? {
        DaemonResponse::StartupState(state) => Ok(state),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("startup state")),
    }
}

pub fn set_startup_behavior(
    request: SetStartupBehaviorRequest,
) -> Result<DaemonCommandResponse, GpuCommandError> {
    send_gpu_command(DaemonRequest::SetStartupBehavior(request))
}

pub fn set_startup_profile(
    request: SetStartupProfileRequest,
) -> Result<DaemonCommandResponse, GpuCommandError> {
    send_gpu_command(DaemonRequest::SetStartupProfile(request))
}

pub fn dismiss_startup_outcome(
    request: StartupGpuRequest,
) -> Result<DaemonCommandResponse, GpuCommandError> {
    send_gpu_command(DaemonRequest::DismissStartupOutcome(request))
}

pub fn get_migration_state() -> Result<crate::protocol::MigrationState, GpuCommandError> {
    match send_request(DaemonRequest::GetMigrationState)? {
        DaemonResponse::MigrationState(state) => Ok(state),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("migration state")),
    }
}

pub fn adopt_migrated_profile(
    request: ProfileIdRequest,
) -> Result<GpuTuningProfile, GpuCommandError> {
    match send_request(DaemonRequest::AdoptMigratedProfile(request))? {
        DaemonResponse::Profile(profile) => Ok(profile),
        DaemonResponse::Error(error) => Err(error),
        _ => Err(unexpected_response_error("adopted profile")),
    }
}

pub fn dismiss_migration_notice() -> Result<DaemonCommandResponse, GpuCommandError> {
    send_gpu_command(DaemonRequest::DismissMigrationNotice)
}
