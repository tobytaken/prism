//! Cooling faults and external control (ADR 0006). When prismd stops following a GPU's fan curve,
//! after a sensor or fan-command failure or because another tool changed fan control, the curve is
//! kept and only an explicit application brings it back. Prism shows the suspension inline in the
//! Fans track, as a problem in Activity and, after a fault, which is a failure pause, as PAUSED in
//! the sidebar and in a desktop notification. Retry, or Resume after external control, applies
//! the kept configuration again; what's shown changes only with prismd's next push.

use std::collections::BTreeMap;

use prism_gpu::TuningOutcomeStatus;
use prism_gpu::cooling::{CurveSuspension, FanRestoration, RestorationStatus, SuspensionKind};
use prism_protocol::{
    ApplyGpuProfileRequest, DaemonRequest, DaemonResponse, GpuCoolingControl, ProfileSettings,
};

use crate::activity::{ActivityAction, sentence};
use crate::cooling::cooling_read;
use crate::core::{CommandId, Core, Effect};
use crate::drafts::not_applied;
use crate::notifications::{GpuConcern, Subject};
use crate::palette::Command;
use crate::view::{ActivityItem, CoolingProblem, gpu_label};

/// The Retries and Resumes sent to prismd, and why prismd refused the latest on each GPU.
#[derive(Debug, Default)]
pub(crate) struct CoolingFaults {
    /// Each one prismd hasn't answered, with its GPU's id.
    retrying: BTreeMap<CommandId, String>,
    /// Why the latest wasn't applied, by GPU id, until the next one or the suspension changes.
    outcomes: BTreeMap<String, String>,
}

/// How Prism words a suspension of one kind.
struct Wording {
    /// Such as "Fan curve paused".
    title: &'static str,
    /// What the button is called and does: "Retry" or "Resume".
    verb: &'static str,
    /// What the button says while prismd hasn't answered it.
    waiting: &'static str,
    /// What prismd's refusal says.
    refused: &'static str,
    /// What brings the curve back.
    next: &'static str,
}

fn wording(kind: SuspensionKind) -> Wording {
    match kind {
        SuspensionKind::Fault => Wording {
            title: "Fan curve paused",
            verb: "Retry",
            waiting: "Retrying…",
            refused: "Not retried",
            next: "Prism kept the curve, but won't follow it again until you retry.",
        },
        SuspensionKind::ExternalControl => Wording {
            title: "Fan curve suspended",
            verb: "Resume",
            waiting: "Resuming…",
            refused: "Not resumed",
            next: "Prism left the other tool's fan control alone. Resume when you want Prism to \
                   follow the curve again.",
        },
    }
}

/// Why the curve stopped, how each fan fell back, and what brings the curve back.
fn explained(suspension: &CurveSuspension) -> Vec<String> {
    [sentence(&suspension.reason)]
        .into_iter()
        .chain(suspension.restoration.iter().map(fallback))
        .chain([wording(suspension.kind).next.to_owned()])
        .collect()
}

/// A fan's return to automatic cooling, such as "Fan 1: automatic cooling not restored: …".
fn fallback(fan: &FanRestoration) -> String {
    let what = match fan.status {
        RestorationStatus::Restored => "restored",
        RestorationStatus::Unverified => "not confirmed",
        RestorationStatus::Failed => "not restored",
    };
    sentence(&not_applied(
        &format!("Fan {}: automatic cooling {what}", fan.fan_index),
        fan.detail.as_deref(),
    ))
}

impl Core {
    /// Why Prism stopped following the GPU's fan curve, as prismd last pushed it.
    fn suspension(&self, gpu_id: &str) -> Option<&CurveSuspension> {
        self.cooling.get(gpu_id)?.suspension.as_ref()
    }

    /// Whether the GPU's cooling is in a failure pause after a cooling fault.
    pub(crate) fn cooling_paused(&self, gpu_id: &str) -> bool {
        self.suspension(gpu_id)
            .is_some_and(|suspension| suspension.kind == SuspensionKind::Fault)
    }

    /// The GPU's suspended fan curve as the Fans track explains it, if Prism stopped following it.
    pub(crate) fn cooling_problem(&self, gpu_id: &str) -> Option<CoolingProblem> {
        let suspension = self.suspension(gpu_id)?;
        let wording = wording(suspension.kind);
        Some(CoolingProblem {
            title: wording.title.into(),
            detail: explained(suspension),
            action: wording.verb.into(),
            unavailable: self.retry_unavailable(gpu_id),
            outcome: self.cooling_faults.outcomes.get(gpu_id).cloned(),
        })
    }

    /// Each GPU's suspended fan curve, as Activity lists it among the problems.
    pub(crate) fn cooling_problems(&self) -> Vec<ActivityItem> {
        self.gpus()
            .iter()
            .filter_map(|gpu| {
                let problem = self.cooling_problem(&gpu.id)?;
                Some(ActivityItem {
                    title: problem.title.clone(),
                    detail: [self.gpu_name(&gpu.id)]
                        .into_iter()
                        .chain(problem.detail)
                        .chain(problem.outcome)
                        .collect(),
                    when: None,
                    action: Some(self.button(
                        &problem.action,
                        ActivityAction::RetryCooling(gpu.id.clone()),
                    )),
                })
            })
            .collect()
    }

    /// Retry, or Resume, for each GPU whose curve Prism stopped following, as the palette lists
    /// them, such as "Retry GPU 0 · RTX 5090 · 01:00.0".
    pub(crate) fn retry_commands(&self) -> Vec<(Command, String)> {
        self.gpus()
            .iter()
            .enumerate()
            .filter_map(|(index, gpu)| {
                let suspension = self.suspension(&gpu.id)?;
                Some((
                    Command::RetryCooling(gpu.id.clone()),
                    format!(
                        "{} {}",
                        wording(suspension.kind).verb,
                        gpu_label(index, gpu)
                    ),
                ))
            })
            .collect()
    }

    /// Why the GPU's suspended fan curve can't be retried, or resumed, now, when it can't.
    pub(crate) fn retry_unavailable(&self, gpu_id: &str) -> Option<String> {
        let Some(suspension) = self.suspension(gpu_id) else {
            return Some("Prism follows the fan curve".into());
        };
        if self
            .cooling
            .get(gpu_id)
            .and_then(|control| control.applied.as_ref())
            .is_none()
        {
            return Some("Prism has no cooling configuration to apply again".into());
        }
        if let Some(reason) = self.tuning_access().reason() {
            return Some(format!("Needs tuning access · {reason}"));
        }
        let gpu = self.gpus().iter().find(|gpu| gpu.id == gpu_id)?;
        if let Some(reason) = self.cooling_read_only(gpu) {
            return Some(reason);
        }
        if self.cooling_faults.retrying.values().any(|id| id == gpu_id) {
            return Some(wording(suspension.kind).waiting.into());
        }
        if self.applying(gpu_id) {
            return Some("Applying…".into());
        }
        None
    }

    /// Applies the GPU's kept cooling configuration again, for its fans as they are now, which
    /// brings the suspended curve back once prismd follows it.
    pub(crate) fn retry_cooling(&mut self, gpu_id: String) -> Vec<Effect> {
        if self.retry_unavailable(&gpu_id).is_some() {
            return Vec::new();
        }
        let Some(applied) = self
            .cooling
            .get(&gpu_id)
            .and_then(|control| control.applied.clone())
        else {
            return Vec::new();
        };
        let fan_indexes = self
            .gpus()
            .iter()
            .find(|gpu| gpu.id == gpu_id)
            .and_then(cooling_read)
            .map(|cooling| cooling.fans.iter().map(|fan| fan.index).collect());
        let request = DaemonRequest::ApplyProfile(ApplyGpuProfileRequest {
            gpu_id: gpu_id.clone(),
            settings: ProfileSettings {
                cooling: Some(applied),
                ..ProfileSettings::default()
            },
            expected_revision: self.revision_of(&gpu_id),
            fan_indexes,
        });
        let id = self.drafts.next_command();
        self.cooling_faults.outcomes.remove(&gpu_id);
        self.cooling_faults.retrying.insert(id, gpu_id);
        vec![Effect::CommandPrismd { id, request }]
    }

    /// Takes in prismd's answer to a Retry or Resume, if `command` was one, keeping why it
    /// wasn't applied when nothing prismd pushes says so. Says whether it was one.
    pub(crate) fn cooling_retry_answered(
        &mut self,
        command: CommandId,
        response: &DaemonResponse,
    ) -> bool {
        let Some(gpu_id) = self.cooling_faults.retrying.remove(&command) else {
            return false;
        };
        let why = match response {
            DaemonResponse::Error(error) => Some(error.message.clone()),
            // A failed application suspends the curve again, with its own reason.
            DaemonResponse::ProfileApplication(application) => application
                .cooling
                .as_ref()
                .map(|cooling| &cooling.outcome)
                .filter(|outcome| outcome.status == TuningOutcomeStatus::Unattempted)
                .map(|outcome| outcome.error.clone().unwrap_or_default()),
            _ => None,
        };
        if let Some(why) = why {
            let kind = self
                .suspension(&gpu_id)
                .map(|suspension| suspension.kind)
                .unwrap_or_default();
            let what = wording(kind).refused;
            let why = (!why.is_empty()).then_some(why.as_str());
            self.cooling_faults
                .outcomes
                .insert(gpu_id, not_applied(what, why));
        }
        true
    }

    /// Takes in prismd's push of Prism's control of a GPU's fans. A new cooling fault notifies,
    /// and one that ended takes its notification back.
    pub(crate) fn cooling_control_pushed(&mut self, control: GpuCoolingControl) -> Vec<Effect> {
        let gpu_id = control.gpu_id.clone();
        let now = control.suspension.clone();
        let before = self
            .cooling
            .insert(gpu_id.clone(), control)
            .and_then(|control| control.suspension);
        if before == now {
            return Vec::new();
        }
        self.cooling_faults.outcomes.remove(&gpu_id);
        match now {
            Some(suspension) if suspension.kind == SuspensionKind::Fault => {
                let summary = format!("Fan curve paused on {}", self.gpu_name(&gpu_id));
                let body = notification_body(&suspension);
                self.notifications
                    .send(Subject::Gpu(gpu_id, GpuConcern::Cooling), summary, body)
                    .map(Effect::Notify)
                    .into_iter()
                    .collect()
            }
            _ if before.is_some_and(|before| before.kind == SuspensionKind::Fault) => self
                .notifications
                .withdraw(&Subject::Gpu(gpu_id, GpuConcern::Cooling))
                .map(Effect::WithdrawNotification)
                .into_iter()
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The session ended: Retries it didn't answer may or may not have happened.
    pub(crate) fn cooling_faults_lost_session(&mut self) {
        self.cooling_faults = CoolingFaults::default();
    }
}

/// Why the curve stopped, and whether the fans are back under automatic cooling.
fn notification_body(suspension: &CurveSuspension) -> String {
    let reason = sentence(&suspension.reason);
    if suspension.restoration.is_empty() {
        return reason;
    }
    let fallback = if suspension
        .restoration
        .iter()
        .all(|fan| fan.status == RestorationStatus::Restored)
    {
        "Prism returned its fans to automatic cooling."
    } else {
        "Prism couldn't confirm every fan returned to automatic cooling."
    };
    format!("{reason} {fallback}")
}
