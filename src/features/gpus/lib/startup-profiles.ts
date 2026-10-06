import type {
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
      return "Boot will skip this profile: a default or allowed range is unavailable.";
    if (value < min || value > max)
      return "Boot will skip this profile: a value is out of range. Values are never clamped at boot.";
  }
  return null;
}
