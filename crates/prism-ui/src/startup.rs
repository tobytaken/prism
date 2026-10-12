//! Settings → Startup: the host-wide startup behavior and each GPU's startup profile (ADR 0002),
//! as prismd pushes them. Anyone with tuning access can change them, and the latest change wins.
//! They change only as prismd pushes them; answers only say how a change went (ADR 0010).

use prism_protocol::{
    DaemonCommandResponse, DaemonRequest, DaemonResponse, SetStartupBehaviorRequest,
    SetStartupProfileRequest, StartupBehavior, TuningStoreHealth,
};

use crate::core::{CommandId, Core, Effect};
use crate::view::{StartupBehaviorChoice, StartupChoice, StartupGpu, StartupSection, gpu_label};

/// The startup behaviors, in the order Settings → Startup offers them.
pub const STARTUP_BEHAVIORS: [StartupBehavior; 3] = [
    StartupBehavior::Off,
    StartupBehavior::StartupProfiles,
    StartupBehavior::RestoreLastApplied,
];

/// What Settings → Startup calls the startup behavior, such as "Last applied tuning values".
fn behavior_name(behavior: StartupBehavior) -> &'static str {
    match behavior {
        StartupBehavior::Off => "Nothing",
        StartupBehavior::StartupProfiles => "Startup profiles",
        StartupBehavior::RestoreLastApplied => "Last applied tuning values",
    }
}

/// A startup change sent to prismd, waiting for its answer.
#[derive(Debug, Clone)]
pub(crate) enum StartupChange {
    Behavior(StartupBehavior),
    /// Making the profile named the GPU's startup profile, or leaving it with none.
    Profile(Option<String>),
}

impl StartupChange {
    /// What Settings → Startup says once prismd refused it, such as "Quiet not made the startup
    /// profile: …".
    fn refused(&self, why: &str) -> String {
        match self {
            Self::Behavior(behavior) => format!(
                "Startup behavior not set to {}: {why}",
                behavior_name(*behavior)
            ),
            Self::Profile(Some(name)) => format!("{name} not made the startup profile: {why}"),
            Self::Profile(None) => format!("Startup profile not cleared: {why}"),
        }
    }
}

impl Core {
    /// Settings → Startup as shown.
    pub fn startup_settings(&self) -> StartupSection {
        let startup = self.activity.startup.as_ref();
        let chosen = startup.map(|startup| startup.behavior);
        let possibly_stale: Vec<_> = startup
            .filter(|startup| startup.behavior == StartupBehavior::RestoreLastApplied)
            .map(|startup| startup.possibly_stale.iter().collect())
            .unwrap_or_default();
        StartupSection {
            behaviors: STARTUP_BEHAVIORS
                .into_iter()
                .map(|behavior| StartupBehaviorChoice {
                    behavior,
                    name: behavior_name(behavior).into(),
                    chosen: chosen == Some(behavior),
                })
                .collect(),
            read_only: self.needs_tuning_access(),
            suspended: matches!(
                self.activity.store_health,
                Some(TuningStoreHealth::Unhealthy { .. })
            )
            .then(|| {
                "Startup behavior is suspended because Prism's tuning store can't be used. \
                 Activity explains why."
                    .into()
            }),
            possibly_stale: (!possibly_stale.is_empty()).then(|| {
                let gpus: Vec<_> = possibly_stale
                    .into_iter()
                    .map(|gpu_id| self.gpu_name(gpu_id))
                    .collect();
                format!(
                    "A recent change to {} couldn't be recorded for startup, so the next boot \
                     may restore older values until a later change is recorded.",
                    gpus.join(" and ")
                )
            }),
            outcome: self.startup_outcome.clone(),
            gpus: match startup {
                Some(_) => self
                    .gpus()
                    .iter()
                    .enumerate()
                    .map(|(index, gpu)| self.startup_gpu(&gpu.id, gpu_label(index, gpu)))
                    .collect(),
                None => Vec::new(),
            },
            empty: startup
                .is_none()
                .then(|| "The startup choices show once Prism is connected to prismd.".into()),
        }
    }

    /// The GPU's startup profile, with the caller's profiles of the GPU to choose from.
    fn startup_gpu(&self, gpu_id: &str, gpu: String) -> StartupGpu {
        let designated = self.startup_profile(gpu_id);
        let mut choices: Vec<_> = [StartupChoice {
            profile_id: None,
            name: "None".into(),
            chosen: designated.is_none(),
            unavailable: None,
        }]
        .into_iter()
        .chain(self.profiles_of(gpu_id).map(|profile| StartupChoice {
            profile_id: Some(profile.id.clone()),
            name: profile.name.clone(),
            chosen: designated.is_some_and(|(id, _)| *id == profile.id),
            unavailable: None,
        }))
        .collect();
        // Another user's startup profile shows by name; only they can choose it.
        if let Some((id, name)) = designated
            && !choices.iter().any(|choice| choice.chosen)
        {
            choices.push(StartupChoice {
                profile_id: Some(id.clone()),
                name: name.clone(),
                chosen: true,
                unavailable: Some("Another user's profile".into()),
            });
        }
        StartupGpu {
            gpu_id: gpu_id.into(),
            gpu,
            startup: designated.map_or_else(|| "None".into(), |(_, name)| name.clone()),
            choices,
        }
    }

    /// The GPU's startup profile, as its id and name, whoever it belongs to.
    fn startup_profile(&self, gpu_id: &str) -> Option<(&String, &String)> {
        self.activity
            .startup
            .as_ref()?
            .profiles
            .iter()
            .find(|startup| startup.gpu_id == gpu_id)
            .map(|startup| (&startup.profile.id, &startup.profile.name))
    }

    /// Sets the host-wide startup behavior, with tuning access, unless it's chosen already.
    pub(crate) fn set_startup_behavior(&mut self, behavior: StartupBehavior) -> Vec<Effect> {
        let Some(startup) = &self.activity.startup else {
            return Vec::new();
        };
        if startup.behavior == behavior {
            return Vec::new();
        }
        self.send_startup_change(
            StartupChange::Behavior(behavior),
            DaemonRequest::SetStartupBehavior(SetStartupBehaviorRequest { behavior }),
        )
    }

    /// Makes the caller's profile, by id, the GPU's startup profile, or leaves the GPU with none,
    /// with tuning access, unless it's so already.
    pub(crate) fn set_startup_profile(
        &mut self,
        gpu_id: String,
        profile_id: Option<String>,
    ) -> Vec<Effect> {
        if self.activity.startup.is_none()
            || self.startup_profile(&gpu_id).map(|(id, _)| id) == profile_id.as_ref()
        {
            return Vec::new();
        }
        let name = match &profile_id {
            Some(id) => match self.profile(id) {
                Some(profile) if profile.gpu_id == gpu_id => Some(profile.name.clone()),
                _ => return Vec::new(),
            },
            None => None,
        };
        self.send_startup_change(
            StartupChange::Profile(name),
            DaemonRequest::SetStartupProfile(SetStartupProfileRequest { gpu_id, profile_id }),
        )
    }

    /// Sends a startup change, with tuning access, clearing why the last one was refused.
    fn send_startup_change(
        &mut self,
        change: StartupChange,
        request: DaemonRequest,
    ) -> Vec<Effect> {
        if self.needs_tuning_access().is_some() {
            return Vec::new();
        }
        self.startup_outcome = None;
        let id = self.drafts.next_command();
        self.startup_commands.insert(id, change);
        vec![Effect::CommandPrismd { id, request }]
    }

    /// Takes in prismd's answer to a startup change, if `command` was one: why it was refused,
    /// if it was. Says whether it was one.
    pub(crate) fn startup_answered(
        &mut self,
        command: CommandId,
        response: &DaemonResponse,
    ) -> bool {
        let Some(sent) = self.startup_commands.remove(&command) else {
            return false;
        };
        let refusal = match response {
            DaemonResponse::Command(DaemonCommandResponse { success: true, .. }) => None,
            DaemonResponse::Command(DaemonCommandResponse { error, .. }) => {
                Some(error.clone().unwrap_or_else(|| "prismd refused it.".into()))
            }
            DaemonResponse::Error(error) => Some(error.message.clone()),
            _ => Some("prismd answered with something else. Try again.".into()),
        };
        if let Some(why) = refusal {
            self.startup_outcome = Some(sent.refused(&why));
        }
        true
    }

    /// The session ended: changes it didn't answer may or may not have happened.
    pub(crate) fn startup_lost_session(&mut self) {
        self.startup_commands.clear();
        self.startup_outcome = None;
    }
}
