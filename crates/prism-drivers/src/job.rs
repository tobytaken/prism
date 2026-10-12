//! Driver jobs: each run of a driver action, with its progress, log and outcome, kept across
//! daemon restarts. A job belongs to the host boot it started in, and one a daemon restart cut
//! off reads as failed, since nothing runs it any more.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use serde::{Deserialize, Serialize};

use crate::host::{PackageManager, SecureBootState};
use crate::nvidia::gpu_supports_open_modules;
use crate::{
    DriverAction, DriverHost, DriverJob, DriverJobStatus, SystemDriverHost, collect_driver_status,
    request_host_reboot,
};
use prism_gpu::{GpuCommandError, GpuCommandErrorCode};

const LOG_TAIL_LIMIT: usize = 80;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DriverJobFile {
    jobs: Vec<StoredJob>,
}

/// A job as the file keeps it, with the boot it started in; jobs recorded before boots were have
/// none.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredJob {
    #[serde(flatten)]
    job: DriverJob,
    #[serde(default)]
    boot_id: String,
}

/// Hears about each change to a job, in the order they happen.
type JobListener = Arc<dyn Fn(DriverJob) + Send + Sync>;

#[derive(Clone)]
pub struct DriverJobStore {
    state_path: PathBuf,
    /// The host boot this daemon runs in.
    boot_id: String,
    host: Arc<dyn DriverHost>,
    state: Arc<Mutex<DriverJobFile>>,
    canceled_jobs: Arc<Mutex<BTreeSet<String>>>,
    /// The canceled jobs, by id, whose host stopped for it, rather than failing on its own.
    stopped_jobs: Arc<Mutex<BTreeSet<String>>>,
    listener: Arc<Mutex<Option<JobListener>>>,
}

impl DriverJobStore {
    /// The jobs in the file at `state_path`. A job still queued or running there was cut off by a
    /// daemon restart, so it's recorded as failed.
    pub fn load(state_path: PathBuf, boot_id: String, host: Arc<dyn DriverHost>) -> Self {
        let mut state = std::fs::read_to_string(&state_path)
            .ok()
            .and_then(|contents| serde_json::from_str::<DriverJobFile>(&contents).ok())
            .unwrap_or_default();
        let mut interrupted = false;
        for stored in &mut state.jobs {
            let job = &mut stored.job;
            if job.status.is_active() {
                interrupted = true;
                job.status = DriverJobStatus::Failed;
                job.finished_at_unix_ms = Some(unix_timestamp_ms());
                job.current_step = None;
                let error = "prismd stopped before the driver job finished.";
                job.error = Some(error.into());
                push_log(job, error);
            }
        }

        let store = Self {
            state_path,
            boot_id,
            host,
            state: Arc::new(Mutex::new(state)),
            canceled_jobs: Arc::new(Mutex::new(BTreeSet::new())),
            stopped_jobs: Arc::new(Mutex::new(BTreeSet::new())),
            listener: Arc::new(Mutex::new(None)),
        };
        if interrupted && let Ok(state) = store.state.lock() {
            store.persist(&state);
        }
        store
    }

    /// Has `listener` hear about every later change to a job, in order, with the job as it is
    /// then.
    pub fn on_job_changed(&self, listener: impl Fn(DriverJob) + Send + Sync + 'static) {
        if let Ok(mut current) = self.listener.lock() {
            *current = Some(Arc::new(listener));
        }
    }

    pub fn active_job(&self) -> Option<DriverJob> {
        self.state
            .lock()
            .ok()?
            .jobs
            .iter()
            .map(|stored| &stored.job)
            .find(|job| job.status.is_active())
            .cloned()
    }

    /// The job started last in this host boot, whether or not it has finished.
    pub fn latest_job(&self) -> Option<DriverJob> {
        self.state
            .lock()
            .ok()?
            .jobs
            .iter()
            .rev()
            .find(|stored| stored.boot_id == self.boot_id)
            .map(|stored| stored.job.clone())
    }

    pub fn job(&self, job_id: &str) -> Option<DriverJob> {
        self.state
            .lock()
            .ok()?
            .jobs
            .iter()
            .map(|stored| &stored.job)
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
            if state
                .jobs
                .iter()
                .any(|stored| stored.job.status.is_active())
            {
                return Err(GpuCommandError::new(
                    GpuCommandErrorCode::InvalidArgument,
                    "A driver install or update is already running",
                ));
            }
            state.jobs.push(StoredJob {
                job: job.clone(),
                boot_id: self.boot_id.clone(),
            });
            self.persist(&state);
            self.changed(&job);
        }

        let store = self.clone();
        let job_id = job.id.clone();
        thread::spawn(move || store.run_job(job_id));

        Ok(job)
    }

    /// Cancels an active job: a queued one at once, and a running one once its current step
    /// finishes. A job that already ended stays as it is.
    pub fn cancel(&self, job_id: &str) -> Option<DriverJob> {
        let job = self.job(job_id)?;
        if !job.status.is_active() {
            return Some(job);
        }
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
            } else if job.status.is_active() {
                push_log(job, "Cancellation requested. The current package-manager command may need to finish first.");
            }
        });
        self.job(job_id)
    }

    fn run_job(&self, job_id: String) {
        let mut action = None;
        self.update_job(&job_id, |job| {
            // A job canceled before it started stays canceled.
            if job.status == DriverJobStatus::Queued {
                action = Some(job.action);
                job.status = DriverJobStatus::Running;
                job.current_step = Some("Preparing driver action".into());
                push_log(job, "Driver job started.");
            }
        });
        let Some(action) = action else {
            return;
        };

        let progress = JobProgress {
            store: self,
            job_id: &job_id,
        };
        match self.host.run(action, &progress) {
            Ok(completion) => self.update_job(&job_id, |job| {
                job.status = completion.status;
                job.finished_at_unix_ms = Some(unix_timestamp_ms());
                job.current_step = completion.current_step;
                job.error = None;
                job.requires_reboot = completion.requires_reboot;
                push_log(job, &completion.message);
            }),
            Err(error) => self.update_job(&job_id, |job| {
                job.status = if self.contains(&self.stopped_jobs, &job_id) {
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

    /// Changes the job, then saves and announces it, unless that changed nothing. Announced under
    /// the lock, so listeners hear changes in the order they happened.
    fn update_job(&self, job_id: &str, update: impl FnOnce(&mut DriverJob)) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let Some(stored) = state.jobs.iter_mut().find(|stored| stored.job.id == job_id) else {
            return;
        };
        let before = stored.job.clone();
        update(&mut stored.job);
        if stored.job == before {
            return;
        }
        let job = stored.job.clone();
        self.persist(&state);
        self.changed(&job);
    }

    fn changed(&self, job: &DriverJob) {
        let listener = self
            .listener
            .lock()
            .ok()
            .and_then(|listener| listener.clone());
        if let Some(listener) = listener {
            listener(job.clone());
        }
    }

    fn persist(&self, state: &MutexGuard<'_, DriverJobFile>) {
        if let Some(parent) = self.state_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(payload) = serde_json::to_vec_pretty(&**state) {
            let _ = std::fs::write(&self.state_path, payload);
        }
    }

    fn contains(&self, jobs: &Mutex<BTreeSet<String>>, job_id: &str) -> bool {
        jobs.lock()
            .map(|jobs| jobs.contains(job_id))
            .unwrap_or(false)
    }
}

/// A running job, as its host reports progress on it.
pub struct JobProgress<'a> {
    store: &'a DriverJobStore,
    job_id: &'a str,
}

impl JobProgress<'_> {
    /// Starts a step, such as "Installing NVIDIA driver packages", logging `line`.
    pub fn step(&self, label: &str, line: &str) {
        self.store.update_job(self.job_id, |job| {
            job.current_step = Some(label.into());
            push_log(job, line);
        });
    }

    /// Adds these lines to the log at once, such as a command's output.
    pub fn log(&self, lines: Vec<String>) {
        if lines.is_empty() {
            return;
        }
        self.store.update_job(self.job_id, |job| {
            for line in &lines {
                push_log(job, line);
            }
        });
    }

    /// Stops the job, which then reads as canceled, once cancellation was requested: the host
    /// asks before each step.
    pub fn stop_if_canceled(&self) -> Result<(), String> {
        if !self.store.contains(&self.store.canceled_jobs, self.job_id) {
            return Ok(());
        }
        if let Ok(mut stopped) = self.store.stopped_jobs.lock() {
            stopped.insert(self.job_id.to_string());
        }
        Err("Driver job was canceled.".into())
    }
}

impl DriverHost for SystemDriverHost {
    fn status(&self) -> crate::DriverStatusResponse {
        collect_driver_status()
    }

    fn run(&self, action: DriverAction, job: &JobProgress) -> Result<JobCompletion, String> {
        run_system_action(action, job)
    }

    fn reboot(&self) -> Result<(), GpuCommandError> {
        request_host_reboot()
    }
}

/// Runs the driver action's package operations on this host, after checking its plan again.
fn run_system_action(action: DriverAction, job: &JobProgress) -> Result<JobCompletion, String> {
    let status = collect_driver_status();
    let plan = status
        .nvidia
        .plans
        .iter()
        .find(|plan| plan.action == action)
        .ok_or_else(|| "No supported driver action is available on this system.".to_string())?;

    if !plan.blockers.is_empty() {
        return Err(plan.blockers.join("; "));
    }

    let steps = build_steps(&status.host, &status.nvidia, plan.action);
    for step in steps {
        job.stop_if_canceled()?;

        match step {
            JobStep::Command(command) => {
                job.step(&command.label, &format!("Running {}", command.display()));
                job.log(run_driver_command(&command)?);
            }
            JobStep::UbuntuUpgradeInstalled => {
                job.step(
                    "Finding installed NVIDIA packages",
                    "Finding installed NVIDIA packages.",
                );
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
                job.log(run_driver_command(&command)?);
            }
        }
    }

    let secure_boot_manual = status.host.package_manager == PackageManager::Dnf
        && status.host.secure_boot == SecureBootState::Enabled;

    if !secure_boot_manual {
        return Ok(JobCompletion::reboot_required());
    }
    Ok(JobCompletion {
        status: DriverJobStatus::ManualStepRequired,
        current_step: Some("Machine Owner Key enrollment may be required".into()),
        message:
            "Package operation finished. Complete firmware key enrollment if prompted, then reboot."
                .into(),
        requires_reboot: true,
    })
}

/// How a driver action's package operations finished.
pub struct JobCompletion {
    pub status: DriverJobStatus,
    /// What's left, such as "Reboot required".
    pub current_step: Option<String>,
    /// The log's last line.
    pub message: String,
    pub requires_reboot: bool,
}

impl JobCompletion {
    /// Package operations finished, and the host needs a restart to load the driver.
    pub fn reboot_required() -> Self {
        Self {
            status: DriverJobStatus::RebootRequired,
            current_step: Some("Reboot required".into()),
            message: "Package operation finished. Reboot to load the NVIDIA driver.".into(),
            requires_reboot: true,
        }
    }
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

/// Runs the allowlisted command, returning its output lines, or why it failed.
fn run_driver_command(command: &DriverCommand) -> Result<Vec<String>, String> {
    validate_driver_command(command)?;

    let mut process = Command::new(&command.program);
    process.args(&command.args);
    for (key, value) in &command.env {
        process.env(key, value);
    }

    let output = process
        .output()
        .map_err(|error| format!("Failed to run {}: {error}", command.program))?;

    let mut lines = output_lines(&output.stdout);
    lines.extend(output_lines(&output.stderr));

    if output.status.success() {
        Ok(lines)
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

fn output_lines(output: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(output)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
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
    use std::path::Path;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::{
        DriverCommand, DriverJobStore, JobCompletion, JobProgress, is_deb_nvidia_package,
        validate_driver_command,
    };
    use crate::{DriverAction, DriverHost, DriverJob, DriverJobStatus, DriverStatusResponse};
    use prism_gpu::GpuCommandError;

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

    /// A host whose driver actions run until the test finishes them, each with its outcome.
    struct GatedHost(Mutex<Receiver<Result<JobCompletion, String>>>);

    impl DriverHost for GatedHost {
        fn status(&self) -> DriverStatusResponse {
            unreachable!("the store doesn't read the host's status")
        }

        fn run(&self, _: DriverAction, job: &JobProgress) -> Result<JobCompletion, String> {
            job.step("Installing NVIDIA driver packages", "Running dnf install");
            let outcome = self
                .0
                .lock()
                .unwrap()
                .recv()
                .expect("the test finishes the job");
            job.log(vec!["Complete!".into()]);
            outcome
        }

        fn reboot(&self) -> Result<(), GpuCommandError> {
            unreachable!("the store doesn't reboot")
        }
    }

    /// A store on a gated host in boot "boot-1", with every job change it announces.
    fn gated_store(
        path: &Path,
    ) -> (
        DriverJobStore,
        Sender<Result<JobCompletion, String>>,
        Receiver<DriverJob>,
    ) {
        let (finish, outcomes) = mpsc::channel();
        let store = DriverJobStore::load(
            path.into(),
            "boot-1".into(),
            Arc::new(GatedHost(Mutex::new(outcomes))),
        );
        let (changed, changes) = mpsc::channel();
        store.on_job_changed(move |job| {
            let _ = changed.send(job);
        });
        (store, finish, changes)
    }

    /// The announced changes up to and including the one where the job ended.
    fn until_ended(changes: &Receiver<DriverJob>) -> Vec<DriverJob> {
        let mut seen = Vec::new();
        loop {
            let job = changes
                .recv_timeout(Duration::from_secs(5))
                .expect("the job ends");
            let ended = !job.status.is_active();
            seen.push(job);
            if ended {
                return seen;
            }
        }
    }

    #[test]
    fn listeners_hear_each_change_to_a_job_in_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (store, finish, changes) = gated_store(&dir.path().join("jobs.json"));

        store
            .start(DriverAction::InstallRecommended)
            .expect("start");
        finish.send(Ok(JobCompletion::reboot_required())).unwrap();

        let seen = until_ended(&changes);
        let steps: Vec<_> = seen
            .iter()
            .map(|job| (job.status, job.current_step.as_deref()))
            .collect();
        assert_eq!(
            steps,
            [
                (DriverJobStatus::Queued, Some("Queued")),
                (DriverJobStatus::Running, Some("Preparing driver action")),
                (
                    DriverJobStatus::Running,
                    Some("Installing NVIDIA driver packages")
                ),
                (
                    DriverJobStatus::Running,
                    Some("Installing NVIDIA driver packages")
                ),
                (DriverJobStatus::RebootRequired, Some("Reboot required")),
            ]
        );
        let ended = seen.last().unwrap();
        assert_eq!(
            ended.log_tail.last().map(String::as_str),
            Some("Package operation finished. Reboot to load the NVIDIA driver.")
        );
        assert!(ended.log_tail.contains(&"Complete!".to_string()));
        assert_eq!(store.latest_job().as_ref(), Some(ended));
    }

    #[test]
    fn a_second_job_is_refused_while_one_is_active() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (store, finish, changes) = gated_store(&dir.path().join("jobs.json"));
        let first = store
            .start(DriverAction::InstallRecommended)
            .expect("start");

        let error = store
            .start(DriverAction::UpdateInstalled)
            .expect_err("refused");

        assert!(error.message.contains("already running"));
        assert_eq!(store.active_job().expect("active").id, first.id);
        finish.send(Err("dnf exited with status 1".into())).unwrap();
        assert_eq!(
            until_ended(&changes).last().unwrap().status,
            DriverJobStatus::Failed
        );
        assert_eq!(store.active_job(), None);
    }

    #[test]
    fn a_job_that_fails_on_its_own_after_a_cancel_was_requested_reads_as_failed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (store, finish, changes) = gated_store(&dir.path().join("jobs.json"));
        let job = store
            .start(DriverAction::InstallRecommended)
            .expect("start");
        while changes.recv_timeout(Duration::from_secs(5)).unwrap().status
            != DriverJobStatus::Running
        {}

        store.cancel(&job.id);
        // The package manager fails before the host got to stop.
        finish.send(Err("dnf exited with status 1".into())).unwrap();

        let ended = until_ended(&changes).pop().unwrap();
        assert_eq!(ended.status, DriverJobStatus::Failed);
        assert_eq!(ended.error.as_deref(), Some("dnf exited with status 1"));
    }

    #[test]
    fn a_job_still_running_when_the_daemon_stopped_reads_as_failed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("jobs.json");
        std::fs::write(
            &path,
            serde_json::json!({ "jobs": [{
                "id": "driver-1",
                "status": "running",
                "action": "install_recommended",
                "started_at_unix_ms": 1,
                "finished_at_unix_ms": null,
                "current_step": "Installing",
                "log_tail": [],
                "error": null,
                "requires_reboot": true,
                "boot_id": "boot-1"
            }] })
            .to_string(),
        )
        .expect("write jobs");

        let (store, _, _) = gated_store(&path);

        let job = store.latest_job().expect("this boot's job");
        assert_eq!(job.status, DriverJobStatus::Failed);
        assert_eq!(
            job.error.as_deref(),
            Some("prismd stopped before the driver job finished.")
        );
        assert_eq!(store.active_job(), None);
        // Kept that way.
        let (reloaded, _, _) = gated_store(&path);
        assert_eq!(reloaded.latest_job(), Some(job));
    }

    #[test]
    fn the_latest_job_is_one_started_in_this_boot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("jobs.json");
        let (store, finish, changes) = gated_store(&path);
        store
            .start(DriverAction::InstallRecommended)
            .expect("start");
        finish.send(Ok(JobCompletion::reboot_required())).unwrap();
        until_ended(&changes);

        let next_boot = DriverJobStore::load(
            path,
            "boot-2".into(),
            Arc::new(GatedHost(Mutex::new(mpsc::channel().1))),
        );

        assert!(store.latest_job().is_some());
        assert_eq!(next_boot.latest_job(), None);
    }
}
