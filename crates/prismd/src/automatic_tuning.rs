//! Automatic operations capture the immediate pre-attempt values and roll back only successful
//! settings, in reverse order. All callers run on the GPU's operation queue.
use super::backend::DaemonBackend;
use super::cooling::{CoolingApplicationSource, CoolingController};
use super::profiles::{capture_cooling, reading, verified_gpu};
use super::tuning::{self, TuningOrigin, TuningRequest};
use prism_gpu::cooling::CoolingApplication;
use prism_gpu::{TuningOutcome, TuningOutcomeStatus};
use prism_protocol::{
    ProfileApplicationResponse, ProfileSetting, ProfileSettingKey, ProfileSettings, RollbackResult,
    RollbackStatus,
};

/// The daemon's reading of a profile application's per-setting outcomes.
pub(super) trait ProfileApplicationOutcomes: Sized {
    fn numeric(&self) -> impl Iterator<Item = (ProfileSettingKey, &TuningOutcome)>;
    fn outcomes(&self) -> impl Iterator<Item = &TuningOutcome>;
    fn all_settings_succeeded(&self) -> bool;
    fn unattempted(settings: &ProfileSettings, message: &str) -> Self;
}

impl ProfileApplicationOutcomes for ProfileApplicationResponse {
    fn numeric(&self) -> impl Iterator<Item = (ProfileSettingKey, &TuningOutcome)> {
        [
            (ProfileSettingKey::PowerLimitWatts, self.power.as_ref()),
            (ProfileSettingKey::GraphicsOffsetMhz, self.graphics.as_ref()),
            (ProfileSettingKey::MemoryOffsetMhz, self.memory.as_ref()),
        ]
        .into_iter()
        .filter_map(|(key, outcome)| Some((key, outcome?)))
    }

    fn outcomes(&self) -> impl Iterator<Item = &TuningOutcome> {
        self.numeric()
            .map(|(_, outcome)| outcome)
            .chain(self.cooling.as_ref().map(|cooling| &cooling.outcome))
    }

    fn all_settings_succeeded(&self) -> bool {
        self.outcomes().all(succeeded)
    }

    fn unattempted(settings: &ProfileSettings, message: &str) -> Self {
        let outcome = || unattempted(message);
        Self {
            power: settings.power_limit_watts.map(|_| outcome()),
            graphics: settings.graphics_offset_mhz.map(|_| outcome()),
            memory: settings.memory_offset_mhz.map(|_| outcome()),
            cooling: settings.cooling.as_ref().map(|_| CoolingApplication {
                revision: 0,
                outcome: outcome(),
                fans: vec![],
            }),
            corrections: vec![],
            rollback: Some(RollbackResult::default()),
        }
    }
}

fn succeeded(outcome: &TuningOutcome) -> bool {
    outcome.status == TuningOutcomeStatus::Succeeded
}

fn unattempted(message: &str) -> TuningOutcome {
    TuningOutcome {
        status: TuningOutcomeStatus::Unattempted,
        error: Some(message.into()),
    }
}

/// Capture actual policy, including a fault's verified automatic fallback or a hardware reset.
/// A suspended curve definition is never itself a rollback target.
pub(super) fn capture_cooling_baseline(
    gpu: &prism_gpu::GpuPerformanceDevice,
    control: Option<prism_gpu::cooling::CoolingControl>,
) -> Option<prism_gpu::cooling::CoolingConfiguration> {
    let mut gpu = gpu.clone();
    gpu.cooling.control = control;
    if let Ok(configuration) = capture_cooling(&gpu) {
        return configuration;
    }
    if gpu
        .cooling
        .fans
        .iter()
        .all(|fan| fan.mode == Some(prism_gpu::FanControlMode::Auto))
    {
        gpu.cooling.control = None;
        return capture_cooling(&gpu).ok().flatten();
    }
    // A cooling fault can leave healthy fixed fans manual while curve fans return to Auto.
    // Preserve those known targets, but never infer a suspended curve from a sampled speed.
    let applied = gpu.cooling.control.as_ref()?.applied.as_ref()?;
    let mut fans = Vec::new();
    for fan in &gpu.cooling.fans {
        use prism_gpu::cooling::{CoolingConfiguration, FanConfiguration, FanSetting};
        let setting = match fan.mode? {
            prism_gpu::FanControlMode::Auto => FanSetting::Automatic {},
            prism_gpu::FanControlMode::Manual => {
                let setting = match applied {
                    CoolingConfiguration::Shared { setting } => setting,
                    CoolingConfiguration::PerFan { fans } => {
                        &fans.iter().find(|f| f.fan_index == fan.index)?.setting
                    }
                };
                match setting {
                    FanSetting::Fixed { .. } => setting.clone(),
                    _ => return None,
                }
            }
        };
        fans.push(FanConfiguration {
            fan_index: fan.index,
            setting,
        });
    }
    (!fans.is_empty()).then_some(prism_gpu::cooling::CoolingConfiguration::PerFan { fans })
}

pub(super) fn apply(
    backend: &dyn DaemonBackend,
    cooling: Option<&CoolingController>,
    request: TuningRequest,
) -> ProfileApplicationResponse {
    apply_with_baseline(backend, cooling, request).0
}

pub(super) fn apply_with_baseline(
    backend: &dyn DaemonBackend,
    cooling: Option<&CoolingController>,
    request: TuningRequest,
) -> (ProfileApplicationResponse, ProfileSettings) {
    let blocked = |message: &str| {
        (
            ProfileApplicationResponse::unattempted(&request.settings, message),
            ProfileSettings::default(),
        )
    };
    let readings = backend.performance();
    let gpu = match verified_gpu(&readings, &request.gpu_id) {
        Ok(gpu) => gpu,
        Err(error) => return blocked(&error.message),
    };
    let mut before = ProfileSettings::default();
    let mut expected = ProfileSettings::default();
    for key in ProfileSettingKey::ALL {
        let Some(setting) = request.settings.get(key) else {
            continue;
        };
        let reading = reading(gpu, key);
        let Some(value) = reading.current else {
            return blocked(&format!(
                "{}: the value before this attempt can't be read. Nothing was changed.",
                reading.label
            ));
        };
        before.set(key, ProfileSetting::Value { value });
        let value = match setting {
            ProfileSetting::Value { value } => Some(value),
            ProfileSetting::Default {} => reading.default,
        };
        if let Some(value) = value {
            expected.set(key, ProfileSetting::Value { value });
        }
    }
    if request.settings.cooling.is_some() {
        before.cooling = capture_cooling_baseline(gpu, cooling.map(|c| c.control(&request.gpu_id)));
        if before.cooling.is_none() {
            return blocked(
                "Cooling: the configuration before this attempt can't be read. Nothing was changed.",
            );
        }
    }
    let expected_fans: std::collections::BTreeSet<_> =
        gpu.cooling.fans.iter().map(|fan| fan.index).collect();
    let gpu_id = request.gpu_id.clone();
    let mut response = tuning::apply(
        backend,
        cooling,
        request,
        TuningOrigin::TemporaryAutomatic,
        CoolingApplicationSource::NonManual,
    );
    let mut rollback = RollbackResult::default();
    if !response.all_settings_succeeded() {
        // Numeric settings execute in power/graphics/memory order. Failed and unattempted
        // settings are never rolled back. Each rollback revalidates capabilities and readings.
        for (key, outcome) in response.numeric().collect::<Vec<_>>().into_iter().rev() {
            if !succeeded(outcome) {
                continue;
            }
            let readings = backend.performance();
            let current = verified_gpu(&readings, &gpu_id)
                .ok()
                .and_then(|gpu| reading(gpu, key).current)
                .map(|value| ProfileSetting::Value { value });
            let result = if current.is_none() || current != expected.get(key) {
                unattempted("Rollback blocked: the expected current value could not be verified.")
            } else {
                let mut settings = ProfileSettings::default();
                settings.set(key, before.get(key).unwrap());
                let restored = tuning::apply(
                    backend,
                    cooling,
                    TuningRequest::new(gpu_id.clone(), settings),
                    TuningOrigin::TemporaryAutomatic,
                    CoolingApplicationSource::NonManual,
                );
                let mut result = restored.numeric().next().unwrap().1.clone();
                if succeeded(&result) {
                    let readings = backend.performance();
                    let actual = verified_gpu(&readings, &gpu_id)
                        .ok()
                        .and_then(|gpu| reading(gpu, key).current)
                        .map(|value| ProfileSetting::Value { value });
                    if actual != before.get(key) {
                        result = TuningOutcome {
                            status: TuningOutcomeStatus::Failed,
                            error: Some(
                                "Rollback was accepted but its result could not be verified."
                                    .into(),
                            ),
                        };
                    }
                }
                result
            };
            match key {
                ProfileSettingKey::PowerLimitWatts => rollback.power = Some(result),
                ProfileSettingKey::GraphicsOffsetMhz => rollback.graphics = Some(result),
                ProfileSettingKey::MemoryOffsetMhz => rollback.memory = Some(result),
            }
        }
        if response
            .cooling
            .as_ref()
            .is_some_and(|c| succeeded(&c.outcome))
        {
            let controller = cooling.unwrap();
            let readings = backend.performance();
            let unchanged = verified_gpu(&readings, &gpu_id).is_ok_and(|gpu| {
                let mut gpu = gpu.clone();
                gpu.cooling.control = Some(controller.control(&gpu_id));
                gpu.cooling
                    .fans
                    .iter()
                    .map(|fan| fan.index)
                    .collect::<std::collections::BTreeSet<_>>()
                    == expected_fans
                    && !controller.changed_outside_prism(&gpu)
                    && capture_cooling(&gpu).ok().flatten().is_some()
            });
            let mut result = if unchanged {
                tuning::apply(
                    backend,
                    cooling,
                    TuningRequest::new(
                        gpu_id.clone(),
                        ProfileSettings {
                            cooling: before.cooling.clone(),
                            ..ProfileSettings::default()
                        },
                    ),
                    TuningOrigin::TemporaryAutomatic,
                    CoolingApplicationSource::NonManual,
                )
                .cooling
                .unwrap()
            } else {
                CoolingApplication {
                    revision: controller.control(&gpu_id).revision,
                    outcome: unattempted(
                        "Cooling rollback blocked: the expected controls could not be verified.",
                    ),
                    fans: vec![],
                }
            };
            if succeeded(&result.outcome) {
                let readings = backend.performance();
                let verified = verified_gpu(&readings, &gpu_id).ok().and_then(|gpu| {
                    let mut gpu = gpu.clone();
                    gpu.cooling.control = Some(controller.control(&gpu_id));
                    capture_cooling(&gpu).ok().flatten()
                });
                if verified != before.cooling {
                    result.outcome = TuningOutcome {
                        status: TuningOutcomeStatus::Failed,
                        error: Some(
                            "Cooling rollback was accepted but its result could not be verified."
                                .into(),
                        ),
                    };
                    for fan in &mut result.fans {
                        let mode = verified_gpu(&readings, &gpu_id)
                            .ok()
                            .and_then(|gpu| {
                                gpu.cooling
                                    .fans
                                    .iter()
                                    .find(|reported| reported.index == fan.fan_index)
                            })
                            .and_then(|fan| fan.mode);
                        let setting =
                            before
                                .cooling
                                .as_ref()
                                .and_then(|configuration| match configuration {
                                    prism_gpu::cooling::CoolingConfiguration::Shared {
                                        setting,
                                    } => Some(setting),
                                    prism_gpu::cooling::CoolingConfiguration::PerFan { fans } => {
                                        fans.iter()
                                            .find(|f| f.fan_index == fan.fan_index)
                                            .map(|f| &f.setting)
                                    }
                                });
                        let expected_mode = setting.map(|setting| match setting {
                            prism_gpu::cooling::FanSetting::Automatic {} => {
                                prism_gpu::FanControlMode::Auto
                            }
                            _ => prism_gpu::FanControlMode::Manual,
                        });
                        if mode.is_none() || mode != expected_mode {
                            fan.outcome = result.outcome.clone();
                        }
                    }
                }
            }
            rollback.cooling = Some(result);
        }
        if let Some(controller) = cooling
            && let Some(suspension) = controller.control(&gpu_id).suspension
        {
            rollback.cooling_restoration = suspension.restoration;
        }
        let results: Vec<_> = [&rollback.power, &rollback.graphics, &rollback.memory]
            .into_iter()
            .flatten()
            .chain(rollback.cooling.as_ref().map(|c| &c.outcome))
            .collect();
        rollback.status = if results.iter().any(|r| !succeeded(r))
            || rollback
                .cooling_restoration
                .iter()
                .any(|fan| fan.status != prism_gpu::cooling::RestorationStatus::Restored)
        {
            RollbackStatus::RestoreFailed
        } else if !results.is_empty() {
            RollbackStatus::Restored
        } else {
            RollbackStatus::NotNeeded
        };
    }
    response.rollback = Some(rollback);
    (response, before)
}
