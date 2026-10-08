import type {
  CoolingConfiguration,
  GpuPerformanceDevice,
  GpuTuningProfile,
} from "@/features/gpus/types";
import { PROFILE_SETTING_KEYS } from "./tuning-profiles";

/** Explains why an allowed designation would currently be skipped at boot. */
export function startupProfileWarning(
  profile: GpuTuningProfile,
  gpu?: GpuPerformanceDevice,
) {
  if (!gpu) return "Boot will skip this profile while its GPU is absent.";
  for (const key of PROFILE_SETTING_KEYS) {
    const instruction = profile.settings[key];
    if (!instruction) continue;
    const power = key === "powerLimitWatts";
    const control = power
      ? gpu.power
      : key === "graphicsOffsetMHz"
        ? gpu.clocks.graphics
        : gpu.clocks.memory;
    if (control.support !== "supported")
      return "Boot will skip this profile: a setting is unsupported or its support is unknown.";
    const min = power
      ? gpu.power.minLimitWatts
      : key === "graphicsOffsetMHz"
        ? gpu.clocks.graphics.minOffsetMHz
        : gpu.clocks.memory.minOffsetMHz;
    const max = power
      ? gpu.power.maxLimitWatts
      : key === "graphicsOffsetMHz"
        ? gpu.clocks.graphics.maxOffsetMHz
        : gpu.clocks.memory.maxOffsetMHz;
    const value =
      instruction.kind === "value"
        ? instruction.value
        : power
          ? gpu.power.defaultLimitWatts
          : 0;
    if (value == null || min == null || max == null || min > max)
      return RANGE_UNAVAILABLE;
    if (value < min || value > max) return OUT_OF_RANGE;
  }
  return profile.settings.cooling
    ? coolingWarning(profile.settings.cooling, gpu)
    : null;
}

const RANGE_UNAVAILABLE =
  "Boot will skip this profile: a default or allowed range is unavailable.";
const OUT_OF_RANGE =
  "Boot will skip this profile: a value is out of range. Values are never clamped at boot.";

/** The capability checks the daemon repeats at boot; it remains the authority on curves. */
function coolingWarning(
  configuration: CoolingConfiguration,
  gpu: GpuPerformanceDevice,
) {
  const { cooling } = gpu;
  if (!cooling.isSupported || !cooling.fans.length)
    return "Boot will skip this profile: fan control is unavailable.";
  const targets =
    configuration.arrangement === "shared"
      ? cooling.fans.map((fan) => ({ fan, setting: configuration.setting }))
      : configuration.fans.map(({ fanIndex, setting }) => ({
          fan: cooling.fans.find((fan) => fan.index === fanIndex),
          setting,
        }));
  if (targets.length !== cooling.fans.length || targets.some(({ fan }) => !fan))
    return "Boot will skip this profile: its fans don't match the fans this GPU reports.";
  for (const { fan, setting } of targets) {
    if (setting.mode === "automatic") continue;
    const min = fan!.minSpeedPercent;
    const max = fan!.maxSpeedPercent;
    if (min == null || max == null || min > max) return RANGE_UNAVAILABLE;
    if (setting.mode === "fixed") {
      if (setting.speedPercent < min || setting.speedPercent > max)
        return OUT_OF_RANGE;
      continue;
    }
    const support = cooling.curveSupport;
    if (support?.status === "unavailable")
      return `Boot will skip this profile: fan curves are unavailable. ${support.reason}`;
    if (
      setting.points.some(
        (point) => point.speedPercent < min || point.speedPercent > max,
      ) ||
      (support &&
        !setting.points.some(
          (point) =>
            point.speedPercent === max &&
            point.temperatureC <= support.fullSpeedTemperatureC,
        ))
    )
      return OUT_OF_RANGE;
  }
  return null;
}
