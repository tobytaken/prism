//! Cooling in the staged loop: the fan control mode and fixed speed a GPU's draft stages, shared
//! by all its fans, and the cooling configuration in effect they're staged against.

use prism_gpu::cooling::{CoolingConfiguration, FanSetting};
use prism_gpu::fan_curve::{self, FanCurveSupport};
use prism_gpu::{CoolingInfo, FanControlInfo, FanControlMode};

use crate::core::{Core, Effect, Gpu, Latest, TuningControl};

/// How a GPU's fans are controlled: the GPU's default automatic cooling, a fixed speed, or a fan
/// curve that follows the temperature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanMode {
    Automatic,
    Fixed,
    Curve,
}

impl FanMode {
    /// Every mode, as the fan control mode offers them, left to right.
    pub const ALL: [Self; 3] = [Self::Automatic, Self::Fixed, Self::Curve];

    /// The name as a screen reader says it, such as "Fixed speed".
    pub fn name(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic",
            Self::Fixed => "Fixed speed",
            Self::Curve => "Fan curve",
        }
    }

    /// The name as the mode control shows it, such as "Fixed".
    pub fn short_name(self) -> &'static str {
        match self {
            Self::Automatic => "Auto",
            Self::Fixed => "Fixed",
            Self::Curve => "Curve",
        }
    }

    /// The mode of a fan's setting.
    fn of(setting: &FanSetting) -> Self {
        match setting {
            FanSetting::Automatic {} => Self::Automatic,
            FanSetting::Fixed { .. } => Self::Fixed,
            FanSetting::Curve { .. } => Self::Curve,
        }
    }
}

/// The setting every fan shares in `configuration`, if they share one.
pub(crate) fn shared_setting(configuration: &CoolingConfiguration) -> Option<&FanSetting> {
    match configuration {
        CoolingConfiguration::Shared { setting } => Some(setting),
        CoolingConfiguration::PerFan { fans } => {
            let first = &fans.first()?.setting;
            fans.iter()
                .all(|fan| &fan.setting == first)
                .then_some(first)
        }
    }
}

/// The mode every fan shares in `configuration`, if they share one.
pub(crate) fn mode_of(configuration: &CoolingConfiguration) -> Option<FanMode> {
    shared_setting(configuration).map(FanMode::of)
}

/// The fixed speed every fan shares in `configuration`, if they share one.
pub(crate) fn fixed_speed_of(configuration: &CoolingConfiguration) -> Option<i64> {
    match shared_setting(configuration)? {
        FanSetting::Fixed { speed_percent } => Some(*speed_percent),
        FanSetting::Automatic {} | FanSetting::Curve { .. } => None,
    }
}

/// A staged cooling configuration's mode as shown next to the mode control, such as "fixed"; a
/// fixed speed's stepper shows the speed.
pub(crate) fn staged_cooling_text(configuration: &CoolingConfiguration) -> String {
    match mode_of(configuration) {
        Some(mode) => mode.short_name().to_lowercase(),
        None => "per fan".into(),
    }
}

/// `setting` for every fan.
fn shared(setting: FanSetting) -> CoolingConfiguration {
    CoolingConfiguration::Shared { setting }
}

/// The GPU's cooling as its latest reading says.
pub(crate) fn cooling_read(gpu: &Gpu) -> Option<&CoolingInfo> {
    match &gpu.latest {
        Latest::Read(device) => Some(&device.cooling),
        Latest::Waiting | Latest::Missing { .. } => None,
    }
}

/// The fixed speeds every fan of the GPU accepts, from the highest minimum to the lowest maximum,
/// or why they can't share one.
pub(crate) fn fixed_speed_range(cooling: &CoolingInfo) -> Result<(i64, i64), &'static str> {
    let (min, max) = cooling
        .fans
        .iter()
        .map(FanControlInfo::speed_range)
        .reduce(|(min, max), (fan_min, fan_max)| (min.max(fan_min), max.min(fan_max)))
        .ok_or("This GPU reports no fans to set a fixed speed for")?;
    if min > max {
        return Err("The fans' supported speeds don't overlap, so they can't share a fixed speed");
    }
    Ok((min, max))
}

impl Core {
    /// The cooling configuration in effect on the GPU: as prismd last pushed its tuning state, or
    /// else, while every fan reports automatic control, the GPU's default automatic cooling.
    pub(crate) fn cooling_in_effect(&self, gpu: &Gpu) -> Option<CoolingConfiguration> {
        if let Some(cooling) = self
            .tuning
            .get(&gpu.id)
            .and_then(|state| state.reported.cooling.clone())
        {
            return Some(cooling);
        }
        let fans = &cooling_read(gpu)?.fans;
        (!fans.is_empty()
            && fans
                .iter()
                .all(|fan| fan.mode == Some(FanControlMode::Auto)))
        .then(|| shared(FanSetting::Automatic {}))
    }

    /// The cooling the GPU's draft stages, if it stages any.
    pub(crate) fn staged_cooling(&self, gpu_id: &str) -> Option<&CoolingConfiguration> {
        self.drafts.by_gpu.get(gpu_id)?.settings.cooling.as_ref()
    }

    /// The cooling the fan control mode shows: the one pending, or else the one in effect.
    pub(crate) fn shown_cooling(&self, gpu: &Gpu) -> Option<CoolingConfiguration> {
        self.staged_cooling(&gpu.id)
            .cloned()
            .or_else(|| self.cooling_in_effect(gpu))
    }

    /// Whether a custom fan curve can control the GPU's fans, as its latest reading says.
    pub(crate) fn curve_support(&self, gpu: &Gpu) -> Option<FanCurveSupport> {
        cooling_read(gpu).map(fan_curve::curve_support)
    }

    /// Stages `mode` for every fan of the selected GPU: a fixed speed starts from the one staged
    /// or in effect, or else from the fans' speed now, and a fan curve from the starter fan curve.
    /// Picking the mode in effect leaves cooling unchanged.
    pub(crate) fn stage_fan_mode(&mut self, mode: FanMode) -> Vec<Effect> {
        let Some(gpu) = self.stageable(TuningControl::Cooling) else {
            return Vec::new();
        };
        let in_effect = self.cooling_in_effect(gpu);
        if in_effect.as_ref().and_then(mode_of) == Some(mode) {
            let id = gpu.id.clone();
            return self.put_cooling(id, None);
        }
        let setting = match mode {
            FanMode::Automatic => FanSetting::Automatic {},
            FanMode::Fixed => {
                let Some(Ok((min, max))) = cooling_read(gpu).map(fixed_speed_range) else {
                    return Vec::new();
                };
                let staged = self.staged_cooling(&gpu.id).and_then(fixed_speed_of);
                let now = match &gpu.latest {
                    Latest::Read(device) => device.telemetry.fan_speed_percent.map(i64::from),
                    Latest::Waiting | Latest::Missing { .. } => None,
                };
                FanSetting::Fixed {
                    speed_percent: staged.or(now).unwrap_or(min).clamp(min, max),
                }
            }
            FanMode::Curve => match self.curve_support(gpu) {
                Some(FanCurveSupport::Available { starter, .. }) => {
                    FanSetting::Curve { points: starter }
                }
                Some(FanCurveSupport::Unavailable { .. }) | None => return Vec::new(),
            },
        };
        let id = gpu.id.clone();
        self.stage_cooling(id, shared(setting))
    }

    /// Stages `speed_percent` as the fixed speed of every fan of the selected GPU, kept within the
    /// range they all support.
    pub(crate) fn stage_fan_speed(&mut self, speed_percent: i64) -> Vec<Effect> {
        let Some(gpu) = self.stageable(TuningControl::Cooling) else {
            return Vec::new();
        };
        let Some(Ok((min, max))) = cooling_read(gpu).map(fixed_speed_range) else {
            return Vec::new();
        };
        let id = gpu.id.clone();
        self.stage_cooling(
            id,
            shared(FanSetting::Fixed {
                speed_percent: speed_percent.clamp(min, max),
            }),
        )
    }

    /// Stages `configuration` for the GPU's fans, unless it's the one in effect.
    fn stage_cooling(&mut self, id: String, configuration: CoolingConfiguration) -> Vec<Effect> {
        let in_effect = self
            .gpus()
            .iter()
            .find(|gpu| gpu.id == id)
            .and_then(|gpu| self.cooling_in_effect(gpu));
        let staged = (in_effect.as_ref() != Some(&configuration)).then_some(configuration);
        self.put_cooling(id, staged)
    }
}
