pub mod fedora;
pub mod host;
pub mod job;
pub mod nvidia;
pub mod ubuntu;

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::fedora::fedora_plans;
use crate::host::{DriverHostInfo, PackageManager, collect_host_info};
use crate::job::{DriverJobStore, JobCompletion, JobProgress};
use crate::nvidia::{NvidiaDriverStatus, collect_nvidia_status};
use crate::ubuntu::ubuntu_plans;
use prism_gpu::{GpuCommandError, GpuCommandErrorCode};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DriverAction {
    InstallRecommended,
    UpdateInstalled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DriverJobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    #[serde(alias = "cancelled")]
    Canceled,
    RebootRequired,
    ManualStepRequired,
}

impl DriverJobStatus {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriverPlan {
    pub id: String,
    pub action: DriverAction,
    pub label: String,
    pub recommended: bool,
    pub requires_reboot: bool,
    pub manual_steps: Vec<String>,
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriverJob {
    pub id: String,
    pub status: DriverJobStatus,
    pub action: DriverAction,
    pub started_at_unix_ms: u64,
    pub finished_at_unix_ms: Option<u64>,
    pub current_step: Option<String>,
    pub log_tail: Vec<String>,
    pub error: Option<String>,
    pub requires_reboot: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DriverStatusResponse {
    pub host: DriverHostInfo,
    pub nvidia: NvidiaDriverStatus,
    pub active_job: Option<DriverJob>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartDriverActionRequest {
    pub action: DriverAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriverJobRequest {
    pub job_id: String,
}

/// The host's side of driver management, which tests stand in for.
pub trait DriverHost: Send + Sync {
    /// The host's driver status with every driver plan, and no job.
    fn status(&self) -> DriverStatusResponse;
    /// Runs the driver action's package operations, reporting each step and its output to `job`
    /// and stopping early once it's canceled, and says how they finished, or why they didn't.
    fn run(&self, action: DriverAction, job: &JobProgress) -> Result<JobCompletion, String>;
    /// Asks the host to restart.
    fn reboot(&self) -> Result<(), GpuCommandError>;
}

/// The real host: its package manager, its NVIDIA driver and systemd.
pub struct SystemDriverHost;

#[derive(Clone)]
pub struct DriverService {
    host: Arc<dyn DriverHost>,
    jobs: DriverJobStore,
}

impl DriverService {
    /// Driver management on `host`, with its jobs kept at `state_path`, for a daemon running in the
    /// host boot `boot_id`.
    pub fn new(state_path: PathBuf, boot_id: impl Into<String>, host: Arc<dyn DriverHost>) -> Self {
        Self {
            jobs: DriverJobStore::load(state_path, boot_id.into(), Arc::clone(&host)),
            host,
        }
    }

    pub fn status(&self) -> DriverStatusResponse {
        DriverStatusResponse {
            active_job: self.jobs.active_job(),
            ..self.host.status()
        }
    }

    /// The driver job started last in this host boot, whether or not it has finished.
    pub fn latest_job(&self) -> Option<DriverJob> {
        self.jobs.latest_job()
    }

    /// Has `listener` hear about every later change to a driver job, in order.
    pub fn on_job_changed(&self, listener: impl Fn(DriverJob) + Send + Sync + 'static) {
        self.jobs.on_job_changed(listener);
    }

    pub fn start_action(
        &self,
        request: StartDriverActionRequest,
    ) -> Result<DriverJob, GpuCommandError> {
        if self.jobs.active_job().is_some() {
            return Err(GpuCommandError::new(
                GpuCommandErrorCode::InvalidArgument,
                "A driver install or update is already running",
            ));
        }

        let status = self.host.status();
        let plan = status
            .nvidia
            .plans
            .iter()
            .find(|plan| plan.action == request.action)
            .ok_or_else(|| {
                GpuCommandError::new(
                    GpuCommandErrorCode::NotSupported,
                    "No supported driver action is available on this system",
                )
            })?;

        if !plan.blockers.is_empty() {
            return Err(GpuCommandError::new(
                GpuCommandErrorCode::NotSupported,
                plan.blockers.join("; "),
            ));
        }

        self.jobs.start(request.action)
    }

    pub fn job(&self, request: DriverJobRequest) -> Result<DriverJob, GpuCommandError> {
        self.jobs.job(&request.job_id).ok_or_else(|| {
            GpuCommandError::new(
                GpuCommandErrorCode::NotFound,
                format!("Driver job {} was not found", request.job_id),
            )
        })
    }

    pub fn cancel_job(&self, request: DriverJobRequest) -> Result<DriverJob, GpuCommandError> {
        self.jobs.cancel(&request.job_id).ok_or_else(|| {
            GpuCommandError::new(
                GpuCommandErrorCode::NotFound,
                format!("Driver job {} was not found", request.job_id),
            )
        })
    }

    pub fn reboot(&self) -> Result<(), GpuCommandError> {
        self.host.reboot()
    }
}

/// The host's driver status with every driver plan, and no job.
pub fn collect_driver_status() -> DriverStatusResponse {
    let mut warnings = Vec::new();
    let mut host = collect_host_info();
    let mut nvidia = collect_nvidia_status();
    nvidia.plans = match host.package_manager {
        PackageManager::Apt if host.distro_id == "ubuntu" => ubuntu_plans(&host, &nvidia),
        PackageManager::Dnf if host.distro_id == "fedora" => fedora_plans(&host, &nvidia),
        _ => unsupported_plans(&host, &nvidia),
    };

    if nvidia.runfile_install_detected {
        warnings.push(
            "An NVIDIA runfile installation appears to be present. Prism will not mix it with distro packages.".into(),
        );
    }
    if nvidia.rpm_fusion_detected {
        warnings.push(
            "RPM Fusion NVIDIA packages appear to be installed. Prism's first driver automation path uses NVIDIA's official Fedora repository and will not mix sources.".into(),
        );
    }

    host.supported = nvidia
        .plans
        .iter()
        .any(|plan| plan.blockers.is_empty() && plan.recommended);
    if !host.supported && host.unsupported_reason.is_none() {
        host.unsupported_reason = nvidia
            .plans
            .iter()
            .flat_map(|plan| plan.blockers.iter().cloned())
            .next();
    }

    DriverStatusResponse {
        host,
        nvidia,
        active_job: None,
        warnings,
    }
}

pub fn request_host_reboot() -> Result<(), GpuCommandError> {
    let status = std::process::Command::new("systemctl")
        .arg("reboot")
        .status()
        .map_err(|error| GpuCommandError::new(GpuCommandErrorCode::Io, error.to_string()))?;

    if status.success() {
        Ok(())
    } else {
        Err(GpuCommandError::new(
            GpuCommandErrorCode::Internal,
            format!("systemctl reboot exited with status {status}"),
        ))
    }
}

fn unsupported_plans(host: &DriverHostInfo, nvidia: &NvidiaDriverStatus) -> Vec<DriverPlan> {
    let reason = if nvidia.gpus.is_empty() {
        "No NVIDIA GPU was detected.".to_string()
    } else if host.distro_id == "ubuntu" || host.distro_id == "fedora" {
        "Required package-manager tooling was not found.".to_string()
    } else {
        format!(
            "Automated driver installs are currently supported only on Ubuntu and Fedora, not {}.",
            host.distro_id
        )
    };

    vec![DriverPlan {
        id: "unsupported-install".into(),
        action: DriverAction::InstallRecommended,
        label: "Install recommended driver".into(),
        recommended: false,
        requires_reboot: true,
        manual_steps: unsupported_manual_steps(host),
        blockers: vec![reason],
        warnings: vec![],
    }]
}

fn unsupported_manual_steps(host: &DriverHostInfo) -> Vec<String> {
    match host.distro_id.as_str() {
        "debian" => vec![
            "Use Debian's NvidiaGraphicsDrivers documentation and install the distro nvidia-driver package.".into(),
        ],
        "arch" | "endeavouros" | "manjaro" => vec![
            "Use the ArchWiki NVIDIA page and install the package matching your kernel and GPU generation.".into(),
        ],
        "opensuse-tumbleweed" | "opensuse-leap" | "opensuse" => vec![
            "Use the openSUSE NVIDIA driver guide and install through zypper repositories.".into(),
        ],
        _ => vec![
            "Use your distribution's NVIDIA driver packages. Prism does not support the generic NVIDIA .run installer.".into(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::{DriverJobStatus, collect_driver_status};

    #[test]
    fn active_statuses_are_queued_or_running_only() {
        assert!(DriverJobStatus::Queued.is_active());
        assert!(DriverJobStatus::Running.is_active());
        assert!(!DriverJobStatus::Succeeded.is_active());
        assert!(!DriverJobStatus::Failed.is_active());
    }

    #[test]
    fn job_files_written_with_the_british_spelling_still_load_as_canceled() {
        let status: DriverJobStatus = serde_json::from_str("\"cancelled\"").expect("status");

        assert_eq!(status, DriverJobStatus::Canceled);
        assert_eq!(
            serde_json::to_string(&status).expect("json"),
            "\"canceled\""
        );
    }

    #[test]
    fn status_collection_never_panics_on_test_host() {
        let status = collect_driver_status();
        assert!(!status.host.kernel_release.is_empty());
    }
}
