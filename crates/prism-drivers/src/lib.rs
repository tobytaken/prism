pub mod fedora;
pub mod host;
pub mod job;
pub mod nvidia;
pub mod ubuntu;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::fedora::fedora_plans;
use crate::host::{DriverHostInfo, PackageManager, collect_host_info};
use crate::job::DriverJobStore;
use crate::nvidia::{NvidiaDriverStatus, collect_nvidia_status};
use crate::ubuntu::ubuntu_plans;
use prism_gpu::{GpuCommandError, GpuCommandErrorCode};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DriverAction {
    InstallRecommended,
    UpdateInstalled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct DriverStatusResponse {
    pub host: DriverHostInfo,
    pub nvidia: NvidiaDriverStatus,
    pub active_job: Option<DriverJob>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StartDriverActionRequest {
    pub action: DriverAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DriverJobRequest {
    pub job_id: String,
}

#[derive(Clone)]
pub struct DriverService {
    jobs: DriverJobStore,
}

impl DriverService {
    pub fn new(state_path: PathBuf) -> Self {
        Self {
            jobs: DriverJobStore::load(state_path),
        }
    }

    pub fn status(&self) -> DriverStatusResponse {
        collect_driver_status(self.jobs.active_job())
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

        let status = collect_driver_status(None);
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
}

pub fn collect_driver_status(active_job: Option<DriverJob>) -> DriverStatusResponse {
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
        active_job,
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
        let status = collect_driver_status(None);
        assert!(!status.host.kernel_release.is_empty());
    }
}
