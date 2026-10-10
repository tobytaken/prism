use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;

use serde::{Deserialize, Serialize};

use crate::host::{PackageManager, SecureBootState};
use crate::nvidia::gpu_supports_open_modules;
use crate::{collect_driver_status, DriverAction, DriverJob, DriverJobStatus};
use prism_gpu::{GpuCommandError, GpuCommandErrorCode};

const LOG_TAIL_LIMIT: usize = 80;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct DriverJobFile {
    jobs: Vec<DriverJob>,
}

#[derive(Clone)]
pub struct DriverJobStore {
    state_path: PathBuf,
    state: Arc<Mutex<DriverJobFile>>,
    canceled_jobs: Arc<Mutex<BTreeSet<String>>>,
}

impl DriverJobStore {
    pub fn load(state_path: PathBuf) -> Self {
        let state = std::fs::read_to_string(&state_path)
            .ok()
            .and_then(|contents| serde_json::from_str::<DriverJobFile>(&contents).ok())
            .unwrap_or_default();

        Self {
            state_path,
            state: Arc::new(Mutex::new(state)),
            canceled_jobs: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    pub fn active_job(&self) -> Option<DriverJob> {
        self.state
            .lock()
            .ok()?
            .jobs
            .iter()
            .find(|job| job.status.is_active())
            .cloned()
    }

    pub fn job(&self, job_id: &str) -> Option<DriverJob> {
        self.state
            .lock()
            .ok()?
            .jobs
            .iter()
            .find(|job| job.id == job_id)
            .cloned()
    }

    pub fn start(&self, action: DriverAction) -> Result<DriverJob, GpuCommandError> {
        let job = DriverJob {
            id: format!("driver-{}", unix_timestamp_ms()),
            status: DriverJobStatus::Queued,
            action,
            started_at_unix_ms: unix_timestamp_ms(),
            finished_at_unix_ms: None,
            current_step: Some("Queued".into()),
            log_tail: vec!["Driver job queued.".into()],
            error: None,
            requires_reboot: true,
        };

        {
            let mut state = self.state.lock().map_err(|_| {
                GpuCommandError::new(
                    GpuCommandErrorCode::Internal,
                    "Driver job state is poisoned",
                )
            })?;
            // Checked again under the lock: concurrent requests can both pass the caller's check.
            if state.jobs.iter().any(|job| job.status.is_active()) {
                return Err(GpuCommandError::new(
                    GpuCommandErrorCode::InvalidArgument,
                    "A driver install or update is already running",
                ));
            }
            state.jobs.push(job.clone());
        }
        self.persist();

        let store = self.clone();
        let job_id = job.id.clone();
        thread::spawn(move || store.run_job(job_id));

        Ok(job)
    }

    pub fn cancel(&self, job_id: &str) -> Option<DriverJob> {
        {
            let mut canceled = self.canceled_jobs.lock().ok()?;
            canceled.insert(job_id.to_string());
        }
        self.update_job(job_id, |job| {
            if job.status == DriverJobStatus::Queued {
                job.status = DriverJobStatus::Canceled;
                job.finished_at_unix_ms = Some(unix_timestamp_ms());
                job.current_step = None;
                job.log_tail.push("Driver job canceled before it started.".into());
            } else {
                push_log(job, "Cancellation requested. The current package-manager command may need to finish first.");
            }
        });
        self.job(job_id)
    }

    fn run_job(&self, job_id: String) {
        self.update_job(&job_id, |job| {
            job.status = DriverJobStatus::Running;
            job.current_step = Some("Preparing driver action".into());
            push_log(job, "Driver job started.");
        });

        let result = self.run_job_inner(&job_id);
        match result {
            Ok(completion) => self.update_job(&job_id, |job| {
                job.status = completion.status;
                job.finished_at_unix_ms = Some(unix_timestamp_ms());
                job.current_step = completion.current_step;
                job.error = None;
                job.requires_reboot = completion.requires_reboot;
                push_log(job, &completion.message);
            }),
            Err(error) => self.update_job(&job_id, |job| {
                job.status = if self.is_canceled(&job_id) {
                    DriverJobStatus::Canceled
                } else {
                    DriverJobStatus::Failed
                };
                job.finished_at_unix_ms = Some(unix_timestamp_ms());
                job.current_step = None;
                job.error = Some(error.clone());
                push_log(job, &error);
            }),
        }
    }

    fn run_job_inner(&self, job_id: &str) -> Result<JobCompletion, String> {
        let status = collect_driver_status(None);
        let plan = status
            .nvidia
            .plans
            .iter()
            .find(|plan| {
                self.job(job_id)
                    .map(|job| plan.action == job.action)
                    .unwrap_or(false)
            })
            .ok_or_else(|| "No supported driver action is available on this system.".to_string())?;

        if !plan.blockers.is_empty() {
            return Err(plan.blockers.join("; "));
        }

        let steps = build_steps(&status.host, &status.nvidia, plan.action);
        for step in steps {
            if self.is_canceled(job_id) {
                return Err("Driver job was canceled.".into());
            }

            match step {
                JobStep::Command(command) => {
                    self.update_job(job_id, |job| {
                        job.current_step = Some(command.label.clone());
                        push_log(job, &format!("Running {}", command.display()));
                    });
                    run_driver_command(&command, |line| {
                        self.update_job(job_id, |job| push_log(job, &line));
                    })?;
                }
                JobStep::UbuntuUpgradeInstalled => {
                    self.update_job(job_id, |job| {
                        job.current_step = Some("Finding installed NVIDIA packages".into());
                        push_log(job, "Finding installed NVIDIA packages.");
                    });
                    let packages = installed_deb_nvidia_packages()?;
                    if packages.is_empty() {
                        return Err("No installed NVIDIA packages were found to upgrade.".into());
                    }
                    let mut args = vec!["-y".into(), "install".into(), "--only-upgrade".into()];
                    args.extend(packages);
                    let command = DriverCommand {
                        label: "Upgrading installed NVIDIA packages".into(),
                        program: "apt-get".into(),
                        args,
                        env: vec![("DEBIAN_FRONTEND".into(), "noninteractive".into())],
                    };
                    run_driver_command(&command, |line| {
                        self.update_job(job_id, |job| push_log(job, &line));
                    })?;
                }
            }
        }

        let secure_boot_manual = status.host.package_manager == PackageManager::Dnf
            && status.host.secure_boot == SecureBootState::Enabled;

        Ok(JobCompletion {
            status: if secure_boot_manual {
                DriverJobStatus::ManualStepRequired
            } else {
                DriverJobStatus::RebootRequired
            },
            current_step: if secure_boot_manual {
                Some("Machine Owner Key enrollment may be required".into())
            } else {
                Some("Reboot required".into())
            },
            message: if secure_boot_manual {
                "Package operation finished. Complete firmware key enrollment if prompted, then reboot.".into()
            } else {
                "Package operation finished. Reboot to load the NVIDIA driver.".into()
            },
            requires_reboot: true,
        })
    }

    fn update_job(&self, job_id: &str, update: impl FnOnce(&mut DriverJob)) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(job) = state.jobs.iter_mut().find(|job| job.id == job_id) {
                update(job);
            }
        }
        self.persist();
    }

    fn persist(&self) {
        let Ok(state) = self.state.lock() else {
            return;
        };
        if let Some(parent) = self.state_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(payload) = serde_json::to_vec_pretty(&*state) {
            let _ = std::fs::write(&self.state_path, payload);
        }
    }

    fn is_canceled(&self, job_id: &str) -> bool {
        self.canceled_jobs
            .lock()
            .map(|jobs| jobs.contains(job_id))
            .unwrap_or(false)
    }
}

struct JobCompletion {
    status: DriverJobStatus,
    current_step: Option<String>,
    message: String,
    requires_reboot: bool,
}

enum JobStep {
    Command(DriverCommand),
    UbuntuUpgradeInstalled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DriverCommand {
    label: String,
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
}

impl DriverCommand {
    fn display(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn build_steps(
    host: &crate::host::DriverHostInfo,
    nvidia: &crate::nvidia::NvidiaDriverStatus,
    action: DriverAction,
) -> Vec<JobStep> {
    match (host.package_manager, action) {
        (PackageManager::Apt, DriverAction::InstallRecommended) => vec![
            JobStep::Command(apt_update()),
            JobStep::Command(DriverCommand {
                label: "Installing Ubuntu's recommended NVIDIA driver".into(),
                program: "ubuntu-drivers".into(),
                args: vec!["install".into()],
                env: vec![("DEBIAN_FRONTEND".into(), "noninteractive".into())],
            }),
        ],
        (PackageManager::Apt, DriverAction::UpdateInstalled) => {
            vec![
                JobStep::Command(apt_update()),
                JobStep::UbuntuUpgradeInstalled,
            ]
        }
        (PackageManager::Dnf, DriverAction::InstallRecommended) => {
            let module_package = if gpu_supports_open_modules(&nvidia.gpus) {
                "kmod-nvidia-open-dkms"
            } else {
                "kmod-nvidia-latest-dkms"
            };
            vec![
                JobStep::Command(DriverCommand {
                    label: "Installing Fedora kernel build packages".into(),
                    program: "dnf".into(),
                    args: vec![
                        "-y".into(),
                        "install".into(),
                        "kernel-devel-matched".into(),
                        "kernel-headers".into(),
                    ],
                    env: vec![],
                }),
                JobStep::Command(DriverCommand {
                    label: "Adding NVIDIA Fedora repository".into(),
                    program: "dnf".into(),
                    args: vec![
                        "config-manager".into(),
                        "addrepo".into(),
                        format!(
                            "--from-repofile=https://developer.download.nvidia.com/compute/cuda/repos/fedora{}/x86_64/cuda-fedora{}.repo",
                            host.distro_version, host.distro_version
                        ),
                    ],
                    env: vec![],
                }),
                JobStep::Command(DriverCommand {
                    label: "Installing NVIDIA driver packages".into(),
                    program: "dnf".into(),
                    args: vec![
                        "-y".into(),
                        "install".into(),
                        "nvidia-driver".into(),
                        module_package.into(),
                    ],
                    env: vec![],
                }),
            ]
        }
        (PackageManager::Dnf, DriverAction::UpdateInstalled) => {
            vec![JobStep::Command(DriverCommand {
                label: "Updating NVIDIA driver packages".into(),
                program: "dnf".into(),
                args: vec![
                    "-y".into(),
                    "upgrade".into(),
                    "--refresh".into(),
                    "*nvidia*".into(),
                    "cuda-drivers".into(),
                    "nvidia-open".into(),
                ],
                env: vec![],
            })]
        }
        _ => vec![],
    }
}

fn apt_update() -> DriverCommand {
    DriverCommand {
        label: "Refreshing APT package metadata".into(),
        program: "apt-get".into(),
        args: vec!["update".into()],
        env: vec![("DEBIAN_FRONTEND".into(), "noninteractive".into())],
    }
}

fn run_driver_command(command: &DriverCommand, mut log: impl FnMut(String)) -> Result<(), String> {
    validate_driver_command(command)?;

    let mut process = Command::new(&command.program);
    process.args(&command.args);
    for (key, value) in &command.env {
        process.env(key, value);
    }

    let output = process
        .output()
        .map_err(|error| format!("Failed to run {}: {error}", command.program))?;

    log_command_output(&output.stdout, &mut log);
    log_command_output(&output.stderr, &mut log);

    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} exited with status {}",
            command.display(),
            output.status
        ))
    }
}

fn installed_deb_nvidia_packages() -> Result<Vec<String>, String> {
    let output = Command::new("dpkg-query")
        .args(["-W", "-f=${binary:Package}\n"])
        .output()
        .map_err(|error| format!("Failed to query installed packages: {error}"))?;

    if !output.status.success() {
        return Err(format!("dpkg-query exited with status {}", output.status));
    }

    let packages = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|package| is_deb_nvidia_package(package))
        .map(str::to_string)
        .collect();

    Ok(packages)
}

fn is_deb_nvidia_package(package: &str) -> bool {
    let expected_prefix = package.starts_with("nvidia-")
        || package.starts_with("libnvidia-")
        || package.starts_with("linux-modules-nvidia-");
    expected_prefix
        && package
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || matches!(char, '-' | '+' | '.' | ':'))
}

fn validate_driver_command(command: &DriverCommand) -> Result<(), String> {
    match command.program.as_str() {
        "apt-get" => validate_apt_get(&command.args),
        "ubuntu-drivers" if command.args == ["install"] => Ok(()),
        "dnf" => validate_dnf(&command.args),
        other => Err(format!("Driver command {other} is not allowlisted")),
    }
}

fn validate_apt_get(args: &[String]) -> Result<(), String> {
    if args == ["update"] {
        return Ok(());
    }

    if args.len() >= 4
        && args[0..3] == ["-y", "install", "--only-upgrade"]
        && args[3..]
            .iter()
            .all(|package| is_deb_nvidia_package(package))
    {
        return Ok(());
    }

    Err(format!("apt-get arguments are not allowlisted: {args:?}"))
}

fn validate_dnf(args: &[String]) -> Result<(), String> {
    let exact_allowlist: &[&[&str]] = &[
        &["-y", "install", "kernel-devel-matched", "kernel-headers"],
        &["-y", "install", "nvidia-driver", "kmod-nvidia-open-dkms"],
        &["-y", "install", "nvidia-driver", "kmod-nvidia-latest-dkms"],
        &[
            "-y",
            "upgrade",
            "--refresh",
            "*nvidia*",
            "cuda-drivers",
            "nvidia-open",
        ],
    ];

    if exact_allowlist
        .iter()
        .any(|allowed| args.iter().map(String::as_str).eq(allowed.iter().copied()))
    {
        return Ok(());
    }

    if args.len() == 3
        && args[0] == "config-manager"
        && args[1] == "addrepo"
        && args[2].starts_with(
            "--from-repofile=https://developer.download.nvidia.com/compute/cuda/repos/fedora",
        )
        && args[2].ends_with(".repo")
        && !args[2].contains(' ')
    {
        return Ok(());
    }

    Err(format!("dnf arguments are not allowlisted: {args:?}"))
}

fn log_command_output(output: &[u8], log: &mut impl FnMut(String)) {
    for line in String::from_utf8_lossy(output).lines() {
        let line = line.trim();
        if !line.is_empty() {
            log(line.to_string());
        }
    }
}

fn push_log(job: &mut DriverJob, line: &str) {
    job.log_tail.push(line.to_string());
    if job.log_tail.len() > LOG_TAIL_LIMIT {
        let trim_count = job.log_tail.len() - LOG_TAIL_LIMIT;
        job.log_tail.drain(0..trim_count);
    }
}

fn unix_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{is_deb_nvidia_package, validate_driver_command, DriverCommand, DriverJobStore};
    use crate::{DriverAction, DriverJobStatus};

    #[test]
    fn command_allowlist_accepts_known_driver_commands() {
        validate_driver_command(&DriverCommand {
            label: "test".into(),
            program: "apt-get".into(),
            args: vec!["update".into()],
            env: vec![],
        })
        .expect("apt-get update");

        validate_driver_command(&DriverCommand {
            label: "test".into(),
            program: "dnf".into(),
            args: vec![
                "-y".into(),
                "install".into(),
                "nvidia-driver".into(),
                "kmod-nvidia-open-dkms".into(),
            ],
            env: vec![],
        })
        .expect("dnf install");
    }

    #[test]
    fn command_allowlist_rejects_unsafe_commands() {
        let error = validate_driver_command(&DriverCommand {
            label: "bad".into(),
            program: "sh".into(),
            args: vec!["-c".into(), "rm -rf /".into()],
            env: vec![],
        })
        .expect_err("unsafe command");

        assert!(error.contains("not allowlisted"));
    }

    #[test]
    fn deb_package_filter_accepts_only_nvidia_package_names() {
        assert!(is_deb_nvidia_package("nvidia-driver-595"));
        assert!(is_deb_nvidia_package("libnvidia-gl-595:amd64"));
        assert!(!is_deb_nvidia_package("bash"));
        assert!(!is_deb_nvidia_package("nvidia-driver-595;touch"));
    }

    #[test]
    fn a_second_job_is_refused_while_one_is_active() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("jobs.json");
        std::fs::write(
            &path,
            serde_json::json!({ "jobs": [{
                "id": "driver-1",
                "status": "running",
                "action": "installRecommended",
                "startedAtUnixMs": 1,
                "finishedAtUnixMs": null,
                "currentStep": "Installing",
                "logTail": [],
                "error": null,
                "requiresReboot": true
            }] })
            .to_string(),
        )
        .expect("write jobs");
        let store = DriverJobStore::load(path);

        let error = store
            .start(DriverAction::UpdateInstalled)
            .expect_err("refused");

        assert!(error.message.contains("already running"));
        assert_eq!(store.active_job().expect("active").id, "driver-1");
    }

    #[test]
    fn queued_job_can_be_canceled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = DriverJobStore::load(dir.path().join("jobs.json"));
        let job = store
            .start(DriverAction::InstallRecommended)
            .expect("start");
        let canceled = store.cancel(&job.id).expect("canceled");

        assert!(matches!(
            canceled.status,
            DriverJobStatus::Queued | DriverJobStatus::Running | DriverJobStatus::Canceled
        ));
    }
}
