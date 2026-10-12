//! Drivers: the host's recommended driver plan with its blockers and warnings, Run after a
//! confirmation, the driver job's progress and log, Cancel while it's active, and its outcome,
//! with Reboot after a confirmation when the host needs a restart. prismd pushes the latest driver
//! job of this host boot, so what's shown changes only with its pushes; Prism reads the driver
//! plan when Drivers opens and again once a job ends. A job ending notifies, and a reboot or
//! manual step it leaves outstanding is a problem in Activity.

use std::collections::BTreeMap;

use prism_drivers::host::{PackageManager, SecureBootState};
use prism_drivers::{
    DriverAction, DriverJob, DriverJobRequest, DriverJobStatus, DriverPlan, DriverStatusResponse,
    StartDriverActionRequest,
};
use prism_protocol::{DaemonCommandResponse, DaemonRequest, DaemonResponse};

use crate::activity::{ActivityAction, sentence};
use crate::core::{CommandId, Core, Effect, Section, Selection};
use crate::daemon::Fact;
use crate::drafts::{DialogState, not_applied};
use crate::notifications::Subject;
use crate::palette::Command;
use crate::view::{
    ActivityItem, DriverConfirmation, DriverJobView, DriverPlanView, DriversSection,
};

/// What prismd last said about the host's drivers, and the requests it hasn't answered.
#[derive(Debug, Default)]
pub(crate) struct Drivers {
    /// The host's driver status, with its driver plans, as prismd last read it this session, or
    /// why it couldn't. It stays shown while prismd reads it again.
    status: Option<Result<Box<DriverStatusResponse>, String>>,
    /// The read of the host's driver status prismd hasn't answered.
    reading: Option<CommandId>,
    /// The latest driver job of this host boot, as prismd last pushed it, while there is a
    /// session.
    job: Option<DriverJob>,
    /// Each Run, Cancel or Reboot prismd hasn't answered.
    sent: BTreeMap<CommandId, DriverRequest>,
    /// The job, by id, prismd accepted a Cancel for, until it ends.
    canceling: Option<String>,
    /// Why prismd refused the latest request the Drivers section shows, until the next one.
    outcome: Option<String>,
}

/// A request about the host's drivers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DriverRequest {
    Run,
    Cancel,
    Reboot,
}

impl DriverRequest {
    /// What prismd's refusal says.
    fn refused(self) -> &'static str {
        match self {
            Self::Run => "Driver job not started",
            Self::Cancel => "Driver job not canceled",
            Self::Reboot => "Host not restarted",
        }
    }

    /// What it asks prismd to do, such as "starting the driver job".
    fn doing(self) -> &'static str {
        match self {
            Self::Run => "starting the driver job",
            Self::Cancel => "canceling the driver job",
            Self::Reboot => "restarting the host",
        }
    }
}

/// What the driver action does, as its job's title.
fn action_title(action: DriverAction) -> &'static str {
    match action {
        DriverAction::InstallRecommended => "Install recommended NVIDIA driver",
        DriverAction::UpdateInstalled => "Update installed NVIDIA driver packages",
    }
}

/// Whether the job's package operations finished with the host still needing a restart.
fn needs_reboot(job: &DriverJob) -> bool {
    matches!(
        job.status,
        DriverJobStatus::RebootRequired | DriverJobStatus::ManualStepRequired
    )
}

/// The job's state in short, such as "Running" or "Reboot required".
fn state_name(status: DriverJobStatus) -> &'static str {
    match status {
        DriverJobStatus::Queued => "Queued",
        DriverJobStatus::Running => "Running",
        DriverJobStatus::Succeeded => "Done",
        DriverJobStatus::Failed => "Failed",
        DriverJobStatus::Canceled => "Canceled",
        DriverJobStatus::RebootRequired => "Reboot required",
        DriverJobStatus::ManualStepRequired => "Manual step required",
    }
}

/// What the job's state means for the user.
fn explained(job: &DriverJob) -> Vec<String> {
    match job.status {
        DriverJobStatus::Queued => vec!["prismd starts the job shortly.".into()],
        DriverJobStatus::Running => {
            vec!["prismd runs the package manager as root. This can take several minutes.".into()]
        }
        DriverJobStatus::Succeeded => {
            vec!["Package operations finished, and nothing else is needed.".into()]
        }
        DriverJobStatus::RebootRequired => vec![
            "Package operations finished.".into(),
            "Restart the host to load the NVIDIA driver.".into(),
        ],
        DriverJobStatus::ManualStepRequired => vec![
            "Package operations finished, with a step left for you.".into(),
            sentence(
                job.current_step
                    .as_deref()
                    .unwrap_or("A manual step is required"),
            ),
            "Restart the host, and complete the step when the firmware asks for it. The NVIDIA \
             driver loads after that."
                .into(),
        ],
        DriverJobStatus::Failed => vec![
            sentence(job.error.as_deref().unwrap_or("The driver job failed")),
            "Check the log, then run the driver plan again.".into(),
        ],
        DriverJobStatus::Canceled => vec![
            "The driver job stopped before its next step. Steps that finished keep their changes."
                .into(),
        ],
    }
}

/// The recommended driver plan: the one prismd recommends, or else installing the recommended
/// driver, which explains why it can't run.
fn recommended(status: &DriverStatusResponse) -> Option<&DriverPlan> {
    let plans = &status.nvidia.plans;
    plans
        .iter()
        .find(|plan| plan.recommended)
        .or_else(|| {
            plans
                .iter()
                .find(|plan| plan.action == DriverAction::InstallRecommended)
        })
        .or(plans.first())
}

/// The host's facts the section lists.
fn host_facts(status: &DriverStatusResponse) -> Vec<Fact> {
    let host = &status.host;
    let nvidia = &status.nvidia;
    let fact = |label: &str, value: String| Fact {
        label: label.into(),
        value,
    };
    vec![
        fact(
            "Distribution",
            format!("{} {}", host.distro_id, host.distro_version),
        ),
        fact("Kernel", host.kernel_release.clone()),
        fact(
            "Package manager",
            match host.package_manager {
                PackageManager::Apt => "APT",
                PackageManager::Dnf => "DNF",
                PackageManager::Unsupported => "Not supported",
            }
            .into(),
        ),
        fact(
            "Secure Boot",
            match host.secure_boot {
                SecureBootState::Enabled => "On",
                SecureBootState::Disabled => "Off",
                SecureBootState::Unknown => "Unknown",
            }
            .into(),
        ),
        fact(
            "NVIDIA driver",
            nvidia
                .driver_version
                .clone()
                .unwrap_or_else(|| "Not loaded".into()),
        ),
        fact(
            "NVIDIA GPUs",
            if nvidia.gpus.is_empty() {
                "None detected".into()
            } else {
                nvidia
                    .gpus
                    .iter()
                    .map(|gpu| gpu.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            },
        ),
    ]
}

/// prismd's refusal in `response`, if it refused.
fn refusal(response: &DaemonResponse) -> Option<String> {
    match response {
        DaemonResponse::Error(error) => Some(error.message.clone()),
        DaemonResponse::Command(DaemonCommandResponse {
            success: false,
            error,
        }) => Some(error.clone().unwrap_or_else(|| "prismd refused it.".into())),
        _ => None,
    }
}

impl Core {
    /// Whether Drivers is showing.
    pub(crate) fn drivers_open(&self) -> bool {
        self.selection == Some(Selection::Section(Section::Drivers))
    }

    /// Asks prismd for the host's driver status, unless it's being read or there's no session.
    pub(crate) fn read_driver_status(&mut self) -> Vec<Effect> {
        if !self.connected() || self.drivers.reading.is_some() {
            return Vec::new();
        }
        let id = self.drafts.next_command();
        self.drivers.reading = Some(id);
        vec![Effect::CommandPrismd {
            id,
            request: DaemonRequest::GetDriverStatus,
        }]
    }

    /// The host's driver status, once prismd has read it this session.
    fn driver_status(&self) -> Option<&DriverStatusResponse> {
        self.drivers.status.as_ref()?.as_deref().ok()
    }

    /// The recommended driver plan, once the host's driver status has been read.
    fn driver_plan(&self) -> Option<&DriverPlan> {
        recommended(self.driver_status()?)
    }

    /// Whether a driver job is queued or running.
    fn driver_job_active(&self) -> bool {
        self.drivers
            .job
            .as_ref()
            .is_some_and(|job| job.status.is_active())
    }

    /// Whether the latest driver job left the host needing a restart.
    fn reboot_required(&self) -> bool {
        self.drivers.job.as_ref().is_some_and(needs_reboot)
    }

    fn driver_request_sent(&self, request: DriverRequest) -> bool {
        self.drivers.sent.values().any(|sent| *sent == request)
    }

    /// Why the recommended driver plan can't run now, when it can't: also while it's being read,
    /// unless `unread_runs`, as from the palette, which waits for it in the confirmation.
    fn run_unavailable(&self, unread_runs: bool) -> Option<String> {
        if let Some(reason) = self.needs_tuning_access() {
            return Some(reason);
        }
        if self.driver_request_sent(DriverRequest::Run) {
            return Some("Starting…".into());
        }
        if self.driver_job_active() {
            return Some("A driver job is running".into());
        }
        // The plan can't tell what the restart will load until it happens.
        if self.reboot_required() {
            return Some("Restart the host first".into());
        }
        let checking = Some("Checking the driver plan…".into());
        if self.drivers.reading.is_some() && !unread_runs {
            return checking;
        }
        match &self.drivers.status {
            Some(Ok(status)) => match recommended(status) {
                None => Some("No driver plan for this host".into()),
                Some(plan) => plan
                    .blockers
                    .first()
                    .map(|blocker| format!("Blocked · {blocker}")),
            },
            _ if unread_runs => None,
            Some(Err(_)) => Some("The driver plan couldn't be read".into()),
            None => checking,
        }
    }

    /// Why the driver job can't be canceled now, when it can't.
    fn cancel_unavailable(&self) -> Option<String> {
        let job = self.drivers.job.as_ref()?;
        if !job.status.is_active() {
            return Some("The driver job has ended".into());
        }
        if let Some(reason) = self.needs_tuning_access() {
            return Some(reason);
        }
        (self.driver_request_sent(DriverRequest::Cancel)
            || self.drivers.canceling.as_ref() == Some(&job.id))
        .then(|| "Canceling…".into())
    }

    /// Why the host can't be restarted from here now, when it can't.
    fn reboot_unavailable(&self) -> Option<String> {
        if !self.reboot_required() {
            return Some("No restart is needed".into());
        }
        if let Some(reason) = self.needs_tuning_access() {
            return Some(reason);
        }
        self.driver_request_sent(DriverRequest::Reboot)
            .then(|| "Restarting…".into())
    }

    /// Why the palette's command can't run now, when it can't; none when `command` isn't one.
    pub(crate) fn driver_command_unavailable(&self, command: &Command) -> Option<Option<String>> {
        match command {
            Command::RunDriverPlan => Some(self.run_unavailable(true)),
            Command::RebootHost => Some(self.reboot_unavailable()),
            _ => None,
        }
    }

    /// Run, while there's a session, and Reboot, while the host needs a restart, as the palette
    /// lists them.
    pub(crate) fn driver_commands(&self) -> Vec<(Command, String)> {
        if !self.connected() {
            return Vec::new();
        }
        let mut commands = vec![(
            Command::RunDriverPlan,
            "Run recommended driver plan…".to_owned(),
        )];
        if self.reboot_required() {
            commands.push((Command::RebootHost, "Reboot now…".to_owned()));
        }
        commands
    }

    /// Shows Drivers with the confirmation to run the recommended driver plan over it.
    pub(crate) fn confirm_driver_plan(&mut self) -> Vec<Effect> {
        if self.run_unavailable(true).is_some() {
            return Vec::new();
        }
        let effects = self.perform(Command::GoTo(Section::Drivers));
        self.open_dialog(DialogState::RunDriverPlan { error: None });
        effects
    }

    /// Asks before restarting the host.
    pub(crate) fn confirm_reboot(&mut self) -> Vec<Effect> {
        if self.reboot_unavailable().is_none() {
            self.open_dialog(DialogState::Reboot { error: None });
        }
        Vec::new()
    }

    /// Answers the open Run or Reboot confirmation: starts the driver job or restarts the host.
    pub(crate) fn confirm_driver_dialog(&mut self) -> Vec<Effect> {
        let (request, daemon_request) = match &self.dialog {
            Some(DialogState::RunDriverPlan { .. }) => {
                if self.run_unavailable(false).is_some() {
                    return Vec::new();
                }
                let Some(plan) = self.driver_plan() else {
                    return Vec::new();
                };
                (
                    DriverRequest::Run,
                    DaemonRequest::StartDriverAction(StartDriverActionRequest {
                        action: plan.action,
                    }),
                )
            }
            Some(DialogState::Reboot { .. }) => {
                if self.reboot_unavailable().is_some() {
                    return Vec::new();
                }
                (DriverRequest::Reboot, DaemonRequest::RequestHostReboot)
            }
            _ => return Vec::new(),
        };
        if let Some(DialogState::RunDriverPlan { error } | DialogState::Reboot { error }) =
            &mut self.dialog
        {
            *error = None;
        }
        self.send_driver_request(request, daemon_request)
    }

    /// Asks prismd to cancel the active driver job.
    pub(crate) fn cancel_driver_job(&mut self) -> Vec<Effect> {
        if self.cancel_unavailable().is_some() {
            return Vec::new();
        }
        let Some(job) = &self.drivers.job else {
            return Vec::new();
        };
        let request = DaemonRequest::CancelDriverJob(DriverJobRequest {
            job_id: job.id.clone(),
        });
        self.send_driver_request(DriverRequest::Cancel, request)
    }

    fn send_driver_request(
        &mut self,
        request: DriverRequest,
        daemon_request: DaemonRequest,
    ) -> Vec<Effect> {
        let id = self.drafts.next_command();
        self.drivers.outcome = None;
        self.drivers.sent.insert(id, request);
        vec![Effect::CommandPrismd {
            id,
            request: daemon_request,
        }]
    }

    /// Takes in prismd's answer to a driver request, if `command` was one. A refused Run or Reboot
    /// says why in its confirmation, which closes once prismd accepts it. Says whether it was one.
    pub(crate) fn driver_answered(
        &mut self,
        command: CommandId,
        response: &DaemonResponse,
    ) -> bool {
        if self.drivers.reading == Some(command) {
            self.drivers.reading = None;
            self.drivers.status = Some(match response {
                DaemonResponse::DriverStatus(status) => Ok(Box::new(status.clone())),
                response => Err(refusal(response)
                    .unwrap_or_else(|| "prismd answered with something else.".into())),
            });
            return true;
        }
        let Some(request) = self.drivers.sent.remove(&command) else {
            return false;
        };
        let refused = refusal(response);
        if request == DriverRequest::Cancel
            && refused.is_none()
            && let Some(job) = &self.drivers.job
        {
            self.drivers.canceling = Some(job.id.clone());
        }
        let confirming = match (&mut self.dialog, request) {
            (Some(DialogState::RunDriverPlan { error }), DriverRequest::Run)
            | (Some(DialogState::Reboot { error }), DriverRequest::Reboot) => {
                error.clone_from(&refused.as_deref().map(sentence));
                true
            }
            _ => false,
        };
        if confirming && refused.is_none() {
            self.dialog = None;
        }
        if !confirming && let Some(why) = refused {
            self.drivers.outcome = Some(not_applied(request.refused(), Some(&sentence(&why))));
        }
        true
    }

    /// Takes in a new session's latest driver job, without notifying about one already ended,
    /// and reads the driver plan again if Drivers shows it.
    pub(crate) fn drivers_connected(&mut self, job: Option<DriverJob>) -> Vec<Effect> {
        self.drivers.job = job;
        if self.drivers_open() {
            self.read_driver_status()
        } else {
            Vec::new()
        }
    }

    /// Takes in prismd's push of a driver job. One that ended notifies, and the driver plan is
    /// read again, since its packages may have changed.
    pub(crate) fn driver_job_pushed(&mut self, job: DriverJob) -> Vec<Effect> {
        let ended = !job.status.is_active()
            && self
                .drivers
                .job
                .as_ref()
                .is_none_or(|before| before.id != job.id || before.status.is_active());
        self.drivers.job = Some(job);
        if !ended {
            return Vec::new();
        }
        self.drivers.canceling = None;
        let mut effects: Vec<_> = self.driver_job_notification().into_iter().collect();
        if self.drivers_open() {
            effects.extend(self.read_driver_status());
        }
        effects
    }

    /// The notification about the job that just ended, unless the user canceled it.
    fn driver_job_notification(&mut self) -> Option<Effect> {
        let job = self.drivers.job.as_ref()?;
        let summary = match job.status {
            DriverJobStatus::Succeeded => "NVIDIA driver job done",
            DriverJobStatus::RebootRequired => "NVIDIA driver job done · Reboot required",
            DriverJobStatus::ManualStepRequired => "NVIDIA driver job done · Manual step required",
            DriverJobStatus::Failed => "NVIDIA driver job failed",
            DriverJobStatus::Canceled | DriverJobStatus::Queued | DriverJobStatus::Running => {
                return None;
            }
        };
        let body = format!("{}. {}", action_title(job.action), explained(job).join(" "));
        self.notifications
            .send(Subject::Drivers, summary.into(), body)
            .map(Effect::Notify)
    }

    /// The reboot or manual step the latest driver job left outstanding, as Activity lists it
    /// among the problems.
    pub(crate) fn driver_problems(&self) -> Vec<ActivityItem> {
        let Some(job) = self.drivers.job.as_ref().filter(|job| needs_reboot(job)) else {
            return Vec::new();
        };
        vec![ActivityItem {
            title: state_name(job.status).into(),
            detail: [action_title(job.action).to_owned()]
                .into_iter()
                .chain(explained(job))
                .collect(),
            when: None,
            action: Some(self.button("Open Drivers", ActivityAction::OpenDrivers)),
        }]
    }

    /// The Drivers section as shown.
    pub fn drivers(&self) -> DriversSection {
        let (status, host, warnings, plan) = match &self.drivers.status {
            Some(Ok(status)) => (
                None,
                host_facts(status),
                status
                    .warnings
                    .iter()
                    .chain(&status.nvidia.warnings)
                    .cloned()
                    .collect(),
                recommended(status).map(|plan| DriverPlanView {
                    title: plan.label.clone(),
                    blockers: plan.blockers.clone(),
                    warnings: plan.warnings.clone(),
                    manual_steps: plan.manual_steps.clone(),
                    run_unavailable: self.run_unavailable(false),
                }),
            ),
            Some(Err(why)) => (
                Some(format!(
                    "prismd couldn't read the host's NVIDIA driver: {}",
                    sentence(why)
                )),
                Vec::new(),
                Vec::new(),
                None,
            ),
            None if self.connected() => (
                Some("Checking the host's NVIDIA driver…".into()),
                Vec::new(),
                Vec::new(),
                None,
            ),
            None => (
                Some(format!(
                    "Driver management needs prismd · {}",
                    self.prismd().reason().unwrap_or_default()
                )),
                Vec::new(),
                Vec::new(),
                None,
            ),
        };
        DriversSection {
            // Without a session, the status says why already.
            read_only: self.needs_tuning_access().filter(|_| self.connected()),
            status,
            host,
            warnings,
            plan,
            job: self.drivers.job.as_ref().map(|job| DriverJobView {
                title: action_title(job.action).into(),
                state: state_name(job.status).into(),
                active: job.status.is_active(),
                step: job.current_step.clone().filter(|_| job.status.is_active()),
                explanation: explained(job),
                log: job.log_tail.clone(),
                cancel_unavailable: self.cancel_unavailable(),
                reboot: needs_reboot(job),
                reboot_unavailable: self.reboot_unavailable(),
            }),
            outcome: self.drivers.outcome.clone(),
        }
    }

    /// The Run or Reboot confirmation as shown, if `dialog` is one.
    pub(crate) fn driver_dialog(&self, dialog: &DialogState) -> Option<DriverConfirmation> {
        Some(match dialog {
            DialogState::RunDriverPlan { error } => match self.driver_plan() {
                Some(plan) => DriverConfirmation {
                    title: format!("{}?", plan.label),
                    message: format!(
                        "prismd runs {} as root to change the host's NVIDIA driver packages. \
                         Keep the host on until the driver job ends.",
                        match self.driver_status_package_manager() {
                            Some(PackageManager::Dnf) => "DNF",
                            Some(PackageManager::Apt) => "APT",
                            _ => "the package manager",
                        }
                    ),
                    details: plan
                        .warnings
                        .iter()
                        .chain(&plan.manual_steps)
                        .cloned()
                        .collect(),
                    confirm_text: "Run",
                    confirm_unavailable: self.run_unavailable(false),
                    error: error.clone(),
                },
                None => DriverConfirmation {
                    title: "Run recommended driver plan?".into(),
                    message: self
                        .drivers()
                        .status
                        .unwrap_or_else(|| "This host has no driver plan.".into()),
                    details: Vec::new(),
                    confirm_text: "Run",
                    confirm_unavailable: self.run_unavailable(false),
                    error: error.clone(),
                },
            },
            DialogState::Reboot { error } => DriverConfirmation {
                title: "Restart the host now?".into(),
                message: "prismd asks systemd to restart the host, which closes every app. Save \
                          your work first."
                    .into(),
                details: Vec::new(),
                confirm_text: "Reboot",
                confirm_unavailable: self.reboot_unavailable(),
                error: error.clone(),
            },
            _ => return None,
        })
    }

    fn driver_status_package_manager(&self) -> Option<PackageManager> {
        self.driver_status()
            .map(|status| status.host.package_manager)
    }

    /// The session ended: what prismd said no longer holds, its confirmations have nothing left
    /// to send, and requests it didn't answer may or may not have happened.
    pub(crate) fn drivers_lost_session(&mut self) {
        if matches!(
            self.dialog,
            Some(DialogState::RunDriverPlan { .. } | DialogState::Reboot { .. })
        ) {
            self.dialog = None;
        }
        let unanswered = std::mem::take(&mut self.drivers.sent);
        self.drivers = Drivers::default();
        if let Some(request) = unanswered.into_values().next() {
            self.drivers.outcome = Some(format!(
                "The prismd connection ended before {} was answered. Check Drivers once Prism \
                 reconnects.",
                request.doing()
            ));
        }
    }
}
