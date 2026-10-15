//! Cooling in the staged loop: the fan control mode and fixed speed a GPU's draft stages, shared
//! by all its fans or set for each fan, the remembered fan curve it recalls, and the cooling
//! configuration in effect they're staged against.

use prism_gpu::cooling::{CoolingConfiguration, FanConfiguration, FanNumber, FanSetting};
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
    pub(crate) fn of(setting: &FanSetting) -> Self {
        match setting {
            FanSetting::Automatic {} => Self::Automatic,
            FanSetting::Fixed { .. } => Self::Fixed,
            FanSetting::Curve { .. } => Self::Curve,
        }
    }
}

/// How a GPU's fans share their cooling: one configuration for every fan, or each fan with its
/// own mode and settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanArrangement {
    Shared,
    PerFan,
}

impl FanArrangement {
    /// The name as shown and as a screen reader says it, such as "Per fan".
    pub fn name(self) -> &'static str {
        match self {
            Self::Shared => "Shared",
            Self::PerFan => "Per fan",
        }
    }

    /// The arrangement of `configuration`.
    pub(crate) fn of(configuration: &CoolingConfiguration) -> Self {
        match configuration {
            CoolingConfiguration::Shared { .. } => Self::Shared,
            CoolingConfiguration::PerFan { .. } => Self::PerFan,
        }
    }
}

/// The fans a cooling control sets: every fan of the GPU together, or one fan, by index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fans {
    Every,
    One(u32),
}

impl Fans {
    /// The indexes of these fans among the GPU's.
    pub(crate) fn indexes(self, cooling: &CoolingInfo) -> Vec<u32> {
        match self {
            Self::Every => cooling.fans.iter().map(|fan| fan.index).collect(),
            Self::One(index) => vec![index],
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

/// The setting `fans` follow in `configuration`, if they follow one.
pub(crate) fn setting_of(configuration: &CoolingConfiguration, fans: Fans) -> Option<&FanSetting> {
    match (configuration, fans) {
        (_, Fans::Every) => shared_setting(configuration),
        (CoolingConfiguration::Shared { setting }, Fans::One(_)) => Some(setting),
        (CoolingConfiguration::PerFan { fans }, Fans::One(index)) => fans
            .iter()
            .find(|fan| fan.fan_index == index)
            .map(|fan| &fan.setting),
    }
}

/// The mode `fans` follow in `configuration`, if they follow one.
pub(crate) fn mode_of(configuration: &CoolingConfiguration, fans: Fans) -> Option<FanMode> {
    setting_of(configuration, fans).map(FanMode::of)
}

/// The fixed speed `fans` run at in `configuration`, if they run at one.
pub(crate) fn fixed_speed_of(configuration: &CoolingConfiguration, fans: Fans) -> Option<i64> {
    match setting_of(configuration, fans)? {
        FanSetting::Fixed { speed_percent } => Some(*speed_percent),
        FanSetting::Automatic {} | FanSetting::Curve { .. } => None,
    }
}

/// `setting` for every fan.
pub(crate) fn shared(setting: FanSetting) -> CoolingConfiguration {
    CoolingConfiguration::Shared { setting }
}

/// `configuration` with `fans` set to `setting`: shared by every fan, or one fan's own with the
/// GPU's other fans keeping theirs.
fn with_setting(
    configuration: Option<&CoolingConfiguration>,
    fans: Fans,
    setting: FanSetting,
    cooling: &CoolingInfo,
) -> CoolingConfiguration {
    let Fans::One(index) = fans else {
        return shared(setting);
    };
    let mut fans = per_fan(configuration, &Fans::Every.indexes(cooling));
    match fans.iter_mut().find(|fan| fan.fan_index == index) {
        Some(fan) => fan.setting = setting,
        None => fans.push(FanConfiguration {
            fan_index: index,
            setting,
        }),
    }
    CoolingConfiguration::PerFan { fans }
}

/// `configuration` as a setting for each fan of `indexes`: what they share, or automatic cooling
/// when it's unknown.
fn per_fan(configuration: Option<&CoolingConfiguration>, indexes: &[u32]) -> Vec<FanConfiguration> {
    let setting = match configuration {
        Some(CoolingConfiguration::PerFan { fans }) => return fans.clone(),
        Some(CoolingConfiguration::Shared { setting }) => setting.clone(),
        None => FanSetting::Automatic {},
    };
    indexes
        .iter()
        .map(|&fan_index| FanConfiguration {
            fan_index,
            setting: setting.clone(),
        })
        .collect()
}

/// Whether `a` and `b` give each of the GPU's fans the same setting, whether they share it or
/// not.
fn same_for_each_fan(
    a: &CoolingConfiguration,
    b: &CoolingConfiguration,
    cooling: &CoolingInfo,
) -> bool {
    let each = |configuration| {
        let mut fans = per_fan(Some(configuration), &Fans::Every.indexes(cooling));
        fans.sort_by_key(|fan| fan.fan_index);
        fans
    };
    each(a) == each(b)
}

/// A cooling configuration as a review says it, such as "fixed 55%" or "fan 1 automatic · fan 2
/// fan curve".
pub(crate) fn cooling_text(configuration: &CoolingConfiguration) -> String {
    let setting_text = |setting: &FanSetting| match setting {
        FanSetting::Automatic {} => "automatic".to_owned(),
        FanSetting::Fixed { speed_percent } => format!("fixed {speed_percent}%"),
        FanSetting::Curve { .. } => "fan curve".to_owned(),
    };
    match configuration {
        CoolingConfiguration::Shared { setting } => setting_text(setting),
        CoolingConfiguration::PerFan { fans } => fans
            .iter()
            .map(|fan| {
                format!(
                    "fan {} {}",
                    FanNumber(fan.fan_index),
                    setting_text(&fan.setting)
                )
            })
            .collect::<Vec<_>>()
            .join(" · "),
    }
}

/// The GPU's cooling as its latest reading says.
pub(crate) fn cooling_read(gpu: &Gpu) -> Option<&CoolingInfo> {
    match &gpu.latest {
        Latest::Read(device) => Some(&device.cooling),
        Latest::Waiting | Latest::Missing { .. } => None,
    }
}

/// The fixed speeds `fans` all accept, from the highest minimum to the lowest maximum, or why
/// they can't share one.
pub(crate) fn fixed_speed_range(
    cooling: &CoolingInfo,
    fans: Fans,
) -> Result<(i64, i64), &'static str> {
    let indexes = fans.indexes(cooling);
    let (min, max) = cooling
        .fans
        .iter()
        .filter(|fan| indexes.contains(&fan.index))
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

    /// The arrangement of the cooling shown: per-fan settings, or else one shared configuration.
    pub(crate) fn shown_arrangement(&self, gpu: &Gpu) -> FanArrangement {
        self.shown_cooling(gpu)
            .as_ref()
            .map_or(FanArrangement::Shared, FanArrangement::of)
    }

    /// The fan picked to show and set on its own, while the GPU has it.
    fn picked_fan(&self, gpu: &Gpu) -> Option<u32> {
        let picked = self.picked_fan?;
        cooling_read(gpu)?
            .fans
            .iter()
            .any(|fan| fan.index == picked)
            .then_some(picked)
    }

    /// The fans the fan control mode, fixed speed and fan curve editor show and set: the fan
    /// picked, or else every fan while they share one configuration and the first fan while each
    /// has its own settings.
    pub(crate) fn controlled_fans(&self, gpu: &Gpu) -> Fans {
        if let Some(index) = self.picked_fan(gpu) {
            return Fans::One(index);
        }
        match self.shown_arrangement(gpu) {
            FanArrangement::Shared => Fans::Every,
            FanArrangement::PerFan => cooling_read(gpu)
                .and_then(|cooling| cooling.fans.first())
                .map_or(Fans::Every, |fan| Fans::One(fan.index)),
        }
    }

    /// Whether a custom fan curve can control `fans` of the GPU, as its latest reading says.
    pub(crate) fn curve_support(&self, gpu: &Gpu, fans: Fans) -> Option<FanCurveSupport> {
        cooling_read(gpu)
            .map(|cooling| fan_curve::curve_support_for(cooling, &fans.indexes(cooling)))
    }

    /// Stages `mode` for the fans the selected GPU's controls set: a fixed speed starts from the
    /// one staged or in effect, or else from their speed now, and a fan curve from the starter fan
    /// curve. Picking the mode in effect puts back the setting in effect.
    pub(crate) fn stage_fan_mode(&mut self, mode: FanMode) -> Vec<Effect> {
        self.stage_fan_setting(|core, gpu, cooling, fans| {
            let in_effect = core
                .cooling_in_effect(gpu)
                .and_then(|in_effect| setting_of(&in_effect, fans).cloned())
                .filter(|setting| FanMode::of(setting) == mode);
            Some(match (in_effect, mode) {
                (Some(setting), _) => setting,
                (None, FanMode::Automatic) => FanSetting::Automatic {},
                (None, FanMode::Fixed) => {
                    let (min, max) = fixed_speed_range(cooling, fans).ok()?;
                    let staged = core
                        .staged_cooling(&gpu.id)
                        .and_then(|staged| fixed_speed_of(staged, fans));
                    FanSetting::Fixed {
                        speed_percent: staged
                            .or(speed_now(gpu, fans))
                            .unwrap_or(min)
                            .clamp(min, max),
                    }
                }
                (None, FanMode::Curve) => match core.curve_support(gpu, fans)? {
                    FanCurveSupport::Available { starter, .. } => {
                        FanSetting::Curve { points: starter }
                    }
                    FanCurveSupport::Unavailable { .. } => return None,
                },
            })
        })
    }

    /// Stages `speed_percent` as the fixed speed of the fans the selected GPU's controls set,
    /// kept within the range they all support.
    pub(crate) fn stage_fan_speed(&mut self, speed_percent: i64) -> Vec<Effect> {
        self.stage_fan_setting(|_, _, cooling, fans| {
            let (min, max) = fixed_speed_range(cooling, fans).ok()?;
            Some(FanSetting::Fixed {
                speed_percent: speed_percent.clamp(min, max),
            })
        })
    }

    /// Stages the setting `choose` makes for the fans the selected GPU's controls set, in the
    /// cooling shown, while its cooling can be staged and its fans have been read; none leaves
    /// cooling as it is.
    pub(crate) fn stage_fan_setting(
        &mut self,
        choose: impl FnOnce(&Self, &Gpu, &CoolingInfo, Fans) -> Option<FanSetting>,
    ) -> Vec<Effect> {
        let Some(gpu) = self.stageable(TuningControl::Cooling) else {
            return Vec::new();
        };
        let Some(cooling) = cooling_read(gpu) else {
            return Vec::new();
        };
        let fans = self.controlled_fans(gpu);
        let Some(setting) = choose(self, gpu, cooling, fans) else {
            return Vec::new();
        };
        let configuration = with_setting(self.shown_cooling(gpu).as_ref(), fans, setting, cooling);
        let id = gpu.id.clone();
        self.stage_cooling(id, configuration)
    }

    /// Shows and sets fan `index` of the selected GPU on its own, which gives it its own settings
    /// once one changes; or else, with none, every fan together, which stages one shared
    /// configuration from the setting shown in place of per-fan settings, kept within the fixed
    /// speeds every fan accepts.
    pub(crate) fn pick_fans(&mut self, index: Option<u32>) -> Vec<Effect> {
        let effects = match index {
            Some(_) => Vec::new(),
            None => self.share_cooling(),
        };
        self.picked_fan = index;
        effects
    }

    /// Stages one configuration every fan of the selected GPU shares, in place of per-fan
    /// settings: the setting the controls show, kept within the fixed speeds every fan accepts.
    fn share_cooling(&mut self) -> Vec<Effect> {
        let Some(gpu) = self.stageable(TuningControl::Cooling) else {
            return Vec::new();
        };
        let Some(cooling) = cooling_read(gpu) else {
            return Vec::new();
        };
        let Some(shown) = self.shown_cooling(gpu) else {
            return Vec::new();
        };
        if FanArrangement::of(&shown) == FanArrangement::Shared {
            return Vec::new();
        }
        let setting = setting_of(&shown, self.controlled_fans(gpu))
            .cloned()
            .unwrap_or(FanSetting::Automatic {});
        let setting = match setting {
            FanSetting::Fixed { speed_percent } => match fixed_speed_range(cooling, Fans::Every) {
                Ok((min, max)) => FanSetting::Fixed {
                    speed_percent: speed_percent.clamp(min, max),
                },
                Err(_) => FanSetting::Automatic {},
            },
            setting => setting,
        };
        let id = gpu.id.clone();
        self.stage_cooling(id, shared(setting))
    }

    /// Why the selected GPU's remembered fan curve can't be recalled into its draft now, when it
    /// can't.
    pub(crate) fn recall_unavailable(&self) -> Option<String> {
        let selected = self.selected_gpu();
        let Some(gpu) = self
            .gpus()
            .iter()
            .find(|gpu| Some(&gpu.id) == selected.as_ref())
        else {
            return Some("No GPU is selected".into());
        };
        if let Some(reason) = self.tuning_access().reason() {
            return Some(format!("Needs tuning access · {reason}"));
        }
        if let Some(reason) = self.cooling_read_only(gpu) {
            return Some(reason);
        }
        let Some(remembered) = self.remembered_curve(&gpu.id) else {
            return Some("No remembered fan curve".into());
        };
        if self.applying(&gpu.id) {
            return Some("Applying…".into());
        }
        match self.staged_cooling(&gpu.id) {
            Some(staged) if staged == remembered => Some("Already pending".into()),
            None if self.cooling_in_effect(gpu).as_ref() == Some(remembered) => {
                Some("Already in effect".into())
            }
            _ => None,
        }
    }

    /// The user's remembered fan curve and fan arrangement for the GPU, as prismd last pushed it.
    fn remembered_curve(&self, gpu_id: &str) -> Option<&CoolingConfiguration> {
        self.cooling.get(gpu_id)?.remembered_curve.as_ref()
    }

    /// Stages the selected GPU's remembered fan curve, with its arrangement, in place of any
    /// cooling pending. It reaches the GPU only with the next Apply.
    pub(crate) fn recall_remembered_curve(&mut self) -> Vec<Effect> {
        if self.recall_unavailable().is_some() {
            return Vec::new();
        }
        let Some(gpu) = self.stageable(TuningControl::Cooling) else {
            return Vec::new();
        };
        let Some(remembered) = self.remembered_curve(&gpu.id).cloned() else {
            return Vec::new();
        };
        let id = gpu.id.clone();
        self.stage_cooling(id, remembered)
    }

    /// Stages `configuration` for the GPU's fans, unless it gives each fan the setting in effect.
    pub(crate) fn stage_cooling(
        &mut self,
        id: String,
        configuration: CoolingConfiguration,
    ) -> Vec<Effect> {
        let gpu = self.gpus().iter().find(|gpu| gpu.id == id);
        let unchanged = match (
            gpu.and_then(|gpu| self.cooling_in_effect(gpu)),
            gpu.and_then(cooling_read),
        ) {
            (Some(in_effect), Some(cooling)) => {
                same_for_each_fan(&in_effect, &configuration, cooling)
            }
            (in_effect, _) => in_effect.as_ref() == Some(&configuration),
        };
        let staged = (!unchanged).then_some(configuration);
        self.put_cooling(id, staged)
    }
}

/// The speed `fans` of the GPU run at now, as its latest reading says.
pub(crate) fn speed_now(gpu: &Gpu, fans: Fans) -> Option<i64> {
    let Latest::Read(device) = &gpu.latest else {
        return None;
    };
    match fans {
        Fans::Every => device.telemetry.fan_speed_percent,
        Fans::One(index) => {
            device
                .cooling
                .fans
                .iter()
                .find(|fan| fan.index == index)?
                .current_speed_percent
        }
    }
    .map(i64::from)
}
