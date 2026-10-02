import type {
  ControlSupport,
  GpuPerformanceDevice,
  GpuProfileSettings,
  GpuTuningProfile,
} from "@/features/gpus/types";

export function captureProfileSettings(
  gpu: GpuPerformanceDevice,
): GpuProfileSettings {
  if (!gpu.stableId?.trim()) {
    throw new Error(
      "Saving is unavailable because this GPU's stable identity could not be established.",
    );
  }
  const settings: GpuProfileSettings = {};
  function capture(
    key: keyof GpuProfileSettings,
    label: string,
    support: ControlSupport,
    value: number | null,
    error: string | null,
  ) {
    if (support === "unsupported") return;
    if (support !== "supported") {
      throw new Error(
        `${label}: support is unknown. ${error ?? "Refresh the GPU readings and try again."}`,
      );
    }
    if (value == null || !Number.isFinite(value)) {
      throw new Error(
        `${label}: current configured value is unavailable. ${error ?? "Refresh the GPU readings and try again."}`,
      );
    }
    settings[key] = { kind: "value", value };
  }
  capture(
    "powerLimitWatts",
    "Power limit",
    gpu.power.support,
    gpu.power.currentLimitWatts,
    gpu.power.currentValueError ?? gpu.power.reason,
  );
  capture(
    "graphicsOffsetMHz",
    "Graphics clock offset",
    gpu.clocks.graphics.support,
    gpu.clocks.graphics.currentOffsetMHz,
    gpu.clocks.graphics.currentValueError ?? gpu.clocks.graphics.reason,
  );
  capture(
    "memoryOffsetMHz",
    "Memory clock offset",
    gpu.clocks.memory.support,
    gpu.clocks.memory.currentOffsetMHz,
    gpu.clocks.memory.currentValueError ?? gpu.clocks.memory.reason,
  );
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
) {
  const readings = {
    powerLimitWatts: {
      support: gpu.power.support,
      current: gpu.power.currentLimitWatts,
      default: gpu.power.defaultLimitWatts,
    },
    graphicsOffsetMHz: {
      support: gpu.clocks.graphics.support,
      current: gpu.clocks.graphics.currentOffsetMHz,
      default: 0,
    },
    memoryOffsetMHz: {
      support: gpu.clocks.memory.support,
      current: gpu.clocks.memory.currentOffsetMHz,
      default: 0,
    },
  };
  let changed = false;
  let unavailable = false;
  for (const key of Object.keys(
    profile.settings,
  ) as (keyof GpuProfileSettings)[]) {
    const setting = profile.settings[key]!;
    const reading = readings[key];
    const desired = setting.kind === "value" ? setting.value : reading.default;
    if (
      reading.support !== "supported" ||
      reading.current == null ||
      desired == null
    ) {
      unavailable = true;
    } else if (reading.current !== desired) {
      changed = true;
    }
  }
  return { changed, unavailable };
}
