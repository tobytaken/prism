import type { PerformanceMutationAction } from "@/features/gpus/components/gpu-tuning-screen";
import type {
  ControlSupport,
  GpuPerformanceDevice,
  GpuProfileSettings,
  GpuTuningProfile,
  ProfileRangeCorrection,
  ProfileSetting,
  ProfileSettingKey,
} from "@/features/gpus/types";

export const PROFILE_SETTING_KEYS = [
  "powerLimitWatts",
  "graphicsOffsetMHz",
  "memoryOffsetMHz",
] as const satisfies readonly ProfileSettingKey[];

export function isProfileSettingKey(
  value: unknown,
): value is ProfileSettingKey {
  return (PROFILE_SETTING_KEYS as readonly unknown[]).includes(value);
}

// Matches what the IPC and daemon boundaries accept, so a stored profile can always be sent.
export function isProfileSetting(value: unknown): value is ProfileSetting {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    return false;
  const setting = value as Record<string, unknown>;
  const keys = Object.keys(setting).length;
  return (
    (setting.kind === "default" && keys === 1) ||
    (setting.kind === "value" &&
      Number.isSafeInteger(setting.value) &&
      keys === 2)
  );
}

type ProfileReading = {
  label: string;
  support: ControlSupport;
  current: number | null;
  default: number | null;
  error: string | null;
};

function profileReadings(
  gpu: GpuPerformanceDevice,
): Record<ProfileSettingKey, ProfileReading> {
  return {
    powerLimitWatts: {
      label: "Power limit",
      support: gpu.power.support,
      current: gpu.power.currentLimitWatts,
      default: gpu.power.defaultLimitWatts,
      error: gpu.power.currentValueError ?? gpu.power.reason,
    },
    graphicsOffsetMHz: {
      label: "Graphics clock offset",
      support: gpu.clocks.graphics.support,
      current: gpu.clocks.graphics.currentOffsetMHz,
      default: 0,
      error:
        gpu.clocks.graphics.currentValueError ?? gpu.clocks.graphics.reason,
    },
    memoryOffsetMHz: {
      label: "Memory clock offset",
      support: gpu.clocks.memory.support,
      current: gpu.clocks.memory.currentOffsetMHz,
      default: 0,
      error: gpu.clocks.memory.currentValueError ?? gpu.clocks.memory.reason,
    },
  };
}

// An applied default choice describes the GPU only while the current value is
// the resolved default; numeric equality alone never establishes default intent.
function appliedSetting(
  reading: ProfileReading,
  followsDefault: boolean,
): ProfileSetting | null {
  if (
    reading.support !== "supported" ||
    reading.current == null ||
    !Number.isFinite(reading.current)
  )
    return null;
  return followsDefault && reading.current === reading.default
    ? { kind: "default" }
    : { kind: "value", value: reading.current };
}

export function captureProfileSettings(
  gpu: GpuPerformanceDevice,
  appliedDefaults: readonly ProfileSettingKey[] = [],
): GpuProfileSettings {
  if (!gpu.stableId?.trim()) {
    throw new Error(
      "Saving is unavailable because this GPU's stable identity could not be established.",
    );
  }
  const settings: GpuProfileSettings = {};
  const readings = profileReadings(gpu);
  for (const key of PROFILE_SETTING_KEYS) {
    const reading = readings[key];
    if (reading.support === "unsupported") continue;
    if (reading.support !== "supported") {
      throw new Error(
        `${reading.label}: support is unknown. ${reading.error ?? "Refresh the GPU readings and try again."}`,
      );
    }
    const setting = appliedSetting(reading, appliedDefaults.includes(key));
    if (!setting) {
      throw new Error(
        `${reading.label}: current configured value is unavailable. ${reading.error ?? "Refresh the GPU readings and try again."}`,
      );
    }
    settings[key] = setting;
  }
  if (Object.keys(settings).length === 0) {
    throw new Error(
      "This GPU has no supported profile settings (power limit or clock offsets).",
    );
  }
  return settings;
}

export function profileSaveBlocker(gpu: GpuPerformanceDevice): string | null {
  try {
    captureProfileSettings(gpu);
    return null;
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
}

export function compareProfile(
  gpu: GpuPerformanceDevice,
  profile: GpuTuningProfile,
  appliedDefaults: readonly ProfileSettingKey[] = [],
) {
  const readings = profileReadings(gpu);
  let changed = false;
  let unavailable = false;
  for (const key of Object.keys(profile.settings) as ProfileSettingKey[]) {
    const setting = profile.settings[key]!;
    const applied = appliedSetting(
      readings[key],
      appliedDefaults.includes(key),
    );
    if (!applied) {
      unavailable = true;
    } else if (
      applied.kind !== setting.kind ||
      (applied.kind === "value" &&
        setting.kind === "value" &&
        applied.value !== setting.value)
    ) {
      changed = true;
    }
  }
  return { changed, unavailable };
}

/** The profile's settings once reviewed corrections move saved values to their bounds. */
export function correctedSettings(
  settings: GpuProfileSettings,
  corrections: readonly ProfileRangeCorrection[] = [],
): GpuProfileSettings {
  const corrected = { ...settings };
  for (const correction of corrections)
    corrected[correction.setting] = {
      kind: "value",
      value: correction.proposedValue,
    };
  return corrected;
}

/** Applies a saved profile by id, first saving any reviewed corrections to it. */
export function profileApplication(
  gpu: GpuPerformanceDevice,
  profile: GpuTuningProfile,
  corrections?: ProfileRangeCorrection[],
): PerformanceMutationAction {
  return {
    kind: "applyProfile",
    gpuId: gpu.id,
    stableId: profile.gpuId,
    settings: correctedSettings(profile.settings, corrections),
    profileId: profile.id,
    ...(corrections && { corrections }),
  };
}
