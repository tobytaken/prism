//! Profile validation uses the same cooling rules as live application. Corrections are proposals,
//! never application-time clamping, and must still form a valid complete configuration.
use prism_gpu::GpuPerformanceDevice;
use prism_gpu::cooling::{CoolingConfiguration, FanSetting};
use prism_protocol::{CoolingSettingKey, ProfileRangeCorrection};

pub(super) fn problems(
    gpu: &GpuPerformanceDevice,
    configuration: &CoolingConfiguration,
) -> Vec<String> {
    let (shared, targets) = match configuration {
        CoolingConfiguration::Shared { setting } => (
            true,
            gpu.cooling
                .fans
                .iter()
                .map(|f| (f.index, setting.clone()))
                .collect::<Vec<_>>(),
        ),
        CoolingConfiguration::PerFan { fans } => (
            false,
            fans.iter()
                .map(|f| (f.fan_index, f.setting.clone()))
                .collect(),
        ),
    };
    let mut problems = super::cooling::application_problems(gpu, shared, &targets);
    if !shared
        && (targets.len() != gpu.cooling.fans.len()
            || gpu
                .cooling
                .fans
                .iter()
                .any(|fan| !targets.iter().any(|(index, _)| *index == fan.index)))
    {
        problems.push("A per-fan profile must configure every fan currently reported by the GPU. Review its cooling configuration.".into());
    }
    for (index, setting) in &targets {
        if matches!(setting, FanSetting::Fixed { .. }) && gpu.cooling.fans.iter().find(|f| f.index == *index).is_some_and(|f| !matches!((f.min_speed_percent, f.max_speed_percent), (Some(min), Some(max)) if min <= max)) {
            problems.push(format!("Fan {}: the current allowed speed range is unavailable.", index + 1));
        }
    }
    problems
}

pub(super) fn correction(
    gpu: &GpuPerformanceDevice,
    saved: &CoolingConfiguration,
) -> Option<ProfileRangeCorrection> {
    if problems(gpu, saved).is_empty() {
        return None;
    }
    let correct = |setting: &mut FanSetting, indexes: Vec<u32>| -> Option<()> {
        if matches!(setting, FanSetting::Automatic {}) {
            return Some(());
        }
        let fans: Option<Vec<_>> = indexes
            .iter()
            .map(|index| gpu.cooling.fans.iter().find(|f| f.index == *index))
            .collect();
        let fans = fans?;
        let min = fans
            .iter()
            .map(|f| f.min_speed_percent)
            .collect::<Option<Vec<_>>>()?
            .into_iter()
            .max()? as i64;
        let max = fans
            .iter()
            .map(|f| f.max_speed_percent)
            .collect::<Option<Vec<_>>>()?
            .into_iter()
            .min()? as i64;
        if min > max {
            return None;
        }
        match setting {
            FanSetting::Automatic {} => {}
            FanSetting::Fixed { speed_percent } => {
                *speed_percent = (*speed_percent).clamp(min, max)
            }
            FanSetting::Curve { points } => {
                let maximum = i64::from(gpu.cooling.max_operating_temperature_c?);
                let boundary = maximum - prism_gpu::fan_curve::FULL_SPEED_MARGIN_C;
                if boundary < 0 {
                    return None;
                }
                for point in points.iter_mut() {
                    point.speed_percent = point.speed_percent.clamp(min, max);
                    point.temperature_c = point.temperature_c.clamp(0, maximum);
                }
                // The deadline constrains when maximum speed is reached, not every point.
                // Keep later plateau points; validate ordering after proposing any move.
                if !points.iter().any(|point| point.speed_percent == max) {
                    points.last_mut()?.speed_percent = max;
                }
                let full_speed = points.iter_mut().find(|point| point.speed_percent == max)?;
                full_speed.temperature_c = full_speed.temperature_c.min(boundary);
            }
        }
        Some(())
    };
    let mut proposed = saved.clone();
    match &mut proposed {
        CoolingConfiguration::Shared { setting } => {
            correct(setting, gpu.cooling.fans.iter().map(|f| f.index).collect())?
        }
        CoolingConfiguration::PerFan { fans } => {
            for fan in fans {
                correct(&mut fan.setting, vec![fan.fan_index])?;
            }
        }
    }
    if &proposed == saved || !problems(gpu, &proposed).is_empty() {
        return None;
    }
    Some(ProfileRangeCorrection::Cooling {
        setting: CoolingSettingKey::Cooling,
        saved_value: saved.clone(),
        proposed_value: proposed,
    })
}
