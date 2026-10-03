import type {
  ControlSupport,
  GpuPerformanceDevice,
  GpuProfileSettings,
  GpuTuningProfile,
  ProfileSetting,
  ProfileSettingKey,
} from "@/features/gpus/types";

export const PROFILE_SETTING_KEYS = [
  "powerLimitWatts",
  "graphicsOffsetMHz",
  "memoryOffsetMHz",
] as const satisfies readonly ProfileSettingKey[];

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
