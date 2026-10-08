use super::backend::DaemonBackend;
use super::cooling::CoolingController;
use crate::cooling::{ApplyCoolingRequest, CoolingApplication};
use crate::gpu::{GpuTargetRequest, SetGpuClockOffsetsRequest, SetGpuPowerLimitRequest};
use crate::protocol::{
    ApplyGpuProfileRequest, ClockOffsetResponse, ProfileApplicationResponse,
    ProfileRangeCorrection, ProfileSetting, ProfileSettingKey, RangeBound, TuningOutcome,
    TuningOutcomeStatus,
};

/// Manual operations may review range corrections and update manual intent. Temporary automatic
/// applications/restorations use the same validation/controller but never become manual state.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TuningOrigin {
    Manual,
    TemporaryAutomatic,
}

// Validate the entire profile before any live write.
pub(super) fn apply_profile(
    backend: &dyn DaemonBackend,
    cooling: &CoolingController,
    request: ApplyGpuProfileRequest,
    origin: TuningOrigin,
) -> ProfileApplicationResponse {
    apply(backend, Some(cooling), request, origin)
}

/// Boot results are shared across users. Only manual loads may disclose saved values
/// and offer the profile owner's range-correction review.
pub(super) fn apply_unreviewed_profile(
    backend: &dyn DaemonBackend,
    request: ApplyGpuProfileRequest,
) -> ProfileApplicationResponse {
    apply(backend, None, request, TuningOrigin::TemporaryAutomatic)
}

fn apply(
    backend: &dyn DaemonBackend,
    cooling: Option<&CoolingController>,
    request: ApplyGpuProfileRequest,
    origin: TuningOrigin,
) -> ProfileApplicationResponse {
    let review_ranges = origin == TuningOrigin::Manual;
    use crate::gpu::ControlSupport;
    let settings = &request.settings;
    let blocked = |message: String, corrections: Vec<ProfileRangeCorrection>| {
        let outcome = || TuningOutcome {
            status: TuningOutcomeStatus::Unattempted,
            error: Some(message.clone()),
        };
        ProfileApplicationResponse {
            cooling: settings.cooling.as_ref().map(|_| CoolingApplication {
                revision: cooling.map_or(0, |c| c.control(&request.gpu_id).revision),
                outcome: outcome(),
                fans: vec![],
            }),
            power: settings.power_limit_watts.map(|_| outcome()),
            graphics: settings.graphics_offset_mhz.map(|_| outcome()),
            memory: settings.memory_offset_mhz.map(|_| outcome()),
            corrections,
        }
    };
    let performance = backend.performance();
    let mut owners = performance.gpus.iter().filter(|gpu| {
        !request.gpu_id.trim().is_empty()
            && gpu.stable_id.as_deref() == Some(request.gpu_id.as_str())
    });
    let Some(gpu) = owners.next() else {
        return blocked("The owning GPU is absent or its stable identity could not be verified. Refresh the GPU readings and try again.".into(), vec![]);
    };
    if owners.next().is_some() {
        return blocked("The owning GPU's stable identity is ambiguous. Refresh the GPU readings and try again.".into(), vec![]);
    }
    // Resolve explicit defaults from the same fresh readings used for validation.
    let mut errors = Vec::new();
    let mut range_errors = Vec::new();
    let mut resolve = |setting_key: ProfileSettingKey,
                       label: &str,
                       setting: Option<ProfileSetting>,
                       support: ControlSupport,
                       reason: &Option<String>,
                       default: Option<i64>,
                       min: Option<i64>,
                       max: Option<i64>|
     -> Option<i64> {
        let setting = setting?;
        match support {
            ControlSupport::Unsupported => errors.push(format!(
                "{label} is unsupported. {}",
                reason.as_deref().unwrap_or("Choose a compatible profile.")
            )),
            ControlSupport::Unknown => errors.push(format!(
                "{label} support is unknown. {}",
                reason
                    .as_deref()
                    .unwrap_or("Refresh the GPU readings and try again.")
            )),
            ControlSupport::Supported => {
                let value = match (setting, default) {
                    (ProfileSetting::Value { value }, _) => value,
                    (ProfileSetting::Default {}, Some(value)) => value,
                    (ProfileSetting::Default {}, None) => {
                        errors.push(format!("{label}: the default is unavailable. Refresh the GPU readings and try again."));
                        return None;
                    }
                };
                match (min, max) {
                    (Some(min), Some(max)) if min <= max => {
                        if value >= min && value <= max {
                            return Some(value);
                        }
                        // Only a saved number can be corrected; a default has no saved value to adjust.
                        let ProfileSetting::Value { .. } = setting else {
                            errors.push(format!("{label}: the default {value} is outside the current allowed range {min} to {max}. Refresh the GPU readings and try again."));
                            return None;
                        };
                        let (proposed_value, bound) = if value < min {
                            (min, RangeBound::Minimum)
                        } else {
                            (max, RangeBound::Maximum)
                        };
                        range_errors.push((
                            if review_ranges {
                                format!("{label}: requested {value} is outside the current allowed range {min} to {max}. Discard this change and load the profile again to review a correction.")
                            } else {
                                format!("{label} is outside the current allowed range. Load the profile manually to review a correction.")
                            },
                            ProfileRangeCorrection::Numeric(crate::protocol::NumericRangeCorrection { setting: setting_key, saved_value: value, proposed_value, bound }),
                        ));
                    },
                    _ => errors.push(format!("{label}: the current allowed range is unavailable. Refresh the GPU readings and try again.")),
                }
            }
        }
        None
    };
    let power = resolve(
        ProfileSettingKey::PowerLimitWatts,
        "Power limit",
        settings.power_limit_watts,
        gpu.power.support,
        &gpu.power.reason,
        gpu.power.default_limit_watts.map(i64::from),
        gpu.power.min_limit_watts.map(i64::from),
        gpu.power.max_limit_watts.map(i64::from),
    );
    let graphics = resolve(
        ProfileSettingKey::GraphicsOffsetMhz,
        "Graphics clock offset",
        settings.graphics_offset_mhz,
        gpu.clocks.graphics.support,
        &gpu.clocks.graphics.reason,
        Some(0),
        gpu.clocks.graphics.min_offset_mhz.map(i64::from),
        gpu.clocks.graphics.max_offset_mhz.map(i64::from),
    );
    let memory = resolve(
        ProfileSettingKey::MemoryOffsetMhz,
        "Memory clock offset",
        settings.memory_offset_mhz,
        gpu.clocks.memory.support,
        &gpu.clocks.memory.reason,
        Some(0),
        gpu.clocks.memory.min_offset_mhz.map(i64::from),
        gpu.clocks.memory.max_offset_mhz.map(i64::from),
    );
    if let Some(configuration) = &settings.cooling {
        if cooling.is_none() {
            errors.push(
                "Automatic cooling profile application is not available through this caller yet."
                    .into(),
            );
        }
        let problems = super::cooling_profiles::problems(gpu, configuration);
        if !problems.is_empty() {
            if let Some(correction) = super::cooling_profiles::correction(gpu, configuration) {
                range_errors.push((problems.join("\n"), correction));
            } else {
                errors.extend(problems);
            }
        }
    }
    // Range corrections are offered only when they alone would make the profile applicable.
    let (range_messages, corrections): (Vec<_>, Vec<_>) = range_errors.into_iter().unzip();
    errors.extend(range_messages);
    if !errors.is_empty() {
        let corrections = if review_ranges && errors.len() == corrections.len() {
            corrections
        } else {
            vec![]
        };
        return blocked(errors.join("\n"), corrections);
    }
    let mut failed = false;
    let mut execute = |operation: &mut dyn FnMut() -> Result<(), String>| {
        if failed {
            return TuningOutcome {
                status: TuningOutcomeStatus::Unattempted,
                error: Some(
                    "An earlier profile setting failed. Retry or discard this instruction.".into(),
                ),
            };
        }
        match operation() {
            Ok(()) => TuningOutcome {
                status: TuningOutcomeStatus::Succeeded,
                error: None,
            },
            Err(error) => {
                failed = true;
                TuningOutcome {
                    status: TuningOutcomeStatus::Failed,
                    error: Some(error),
                }
            }
        }
    };
    let mut response = ProfileApplicationResponse {
        cooling: None,
        power: power.map(|value| {
            execute(&mut || {
                backend.set_power_limit(SetGpuPowerLimitRequest {
                    gpu_id: request.gpu_id.clone(),
                    limit_watts: value as u32,
                })
            })
        }),
        graphics: graphics.map(|value| {
            execute(&mut || {
                backend.set_clock_offsets(SetGpuClockOffsetsRequest {
                    gpu_id: request.gpu_id.clone(),
                    graphics_offset_mhz: Some(value as i32),
                    memory_offset_mhz: None,
                })
            })
        }),
        memory: memory.map(|value| {
            execute(&mut || {
                backend.set_clock_offsets(SetGpuClockOffsetsRequest {
                    gpu_id: request.gpu_id.clone(),
                    graphics_offset_mhz: None,
                    memory_offset_mhz: Some(value as i32),
                })
            })
        }),
        corrections: vec![],
    };
    response.cooling = settings.cooling.as_ref().map(|configuration| {
        if failed {
            CoolingApplication {
                revision: cooling.unwrap().control(&request.gpu_id).revision,
                outcome: TuningOutcome {
                    status: TuningOutcomeStatus::Unattempted,
                    error: Some(
                        "An earlier profile setting failed. Retry or discard this instruction."
                            .into(),
                    ),
                },
                fans: vec![],
            }
        } else {
            cooling.unwrap().apply(
                backend,
                ApplyCoolingRequest {
                    gpu_id: request.gpu_id.clone(),
                    configuration: configuration.clone(),
                    expected_revision: None,
                    fan_indexes: None,
                },
            )
        }
    });
    response
}

// Split the existing live command so a later failure cannot hide an earlier write.
// These commands deliberately never touch PersistedGpuState.
pub(super) fn apply_live_offsets(
    backend: &dyn DaemonBackend,
    request: SetGpuClockOffsetsRequest,
) -> ClockOffsetResponse {
    let mut failed = false;
    let mut apply = |graphics: bool, value: Option<i32>| {
        value.map(|value| {
            if failed {
                return TuningOutcome {
                    status: TuningOutcomeStatus::Unattempted,
                    error: Some("An earlier clock offset failed".into()),
                };
            }
            let result = backend.set_clock_offsets(SetGpuClockOffsetsRequest {
                gpu_id: request.gpu_id.clone(),
                graphics_offset_mhz: if graphics { Some(value) } else { None },
                memory_offset_mhz: if graphics { None } else { Some(value) },
            });
            match result {
                Ok(()) => TuningOutcome {
                    status: TuningOutcomeStatus::Succeeded,
                    error: None,
                },
                Err(error) => {
                    failed = true;
                    TuningOutcome {
                        status: TuningOutcomeStatus::Failed,
                        error: Some(error),
                    }
                }
            }
        })
    };
    ClockOffsetResponse {
        graphics: apply(true, request.graphics_offset_mhz),
        memory: apply(false, request.memory_offset_mhz),
    }
}

pub(super) fn reset_live_offsets(
    backend: &dyn DaemonBackend,
    request: GpuTargetRequest,
) -> ClockOffsetResponse {
    use crate::gpu::ControlSupport;
    let performance = backend.performance();
    let gpu = performance.gpus.iter().find(|gpu| gpu.id == request.gpu_id);
    let unavailable = |message: &str| {
        Some(TuningOutcome {
            status: TuningOutcomeStatus::Unattempted,
            error: Some(message.into()),
        })
    };
    let Some(gpu) = gpu else {
        return ClockOffsetResponse {
            graphics: unavailable("GPU unavailable"),
            memory: unavailable("GPU unavailable"),
        };
    };
    if gpu.clocks.graphics.support == ControlSupport::Unknown
        || gpu.clocks.memory.support == ControlSupport::Unknown
    {
        return ClockOffsetResponse {
            graphics: unavailable("Clock offset support is unknown"),
            memory: unavailable("Clock offset support is unknown"),
        };
    }
    apply_live_offsets(
        backend,
        SetGpuClockOffsetsRequest {
            gpu_id: request.gpu_id,
            graphics_offset_mhz: (gpu.clocks.graphics.support == ControlSupport::Supported)
                .then_some(0),
            memory_offset_mhz: (gpu.clocks.memory.support == ControlSupport::Supported)
                .then_some(0),
        },
    )
}
