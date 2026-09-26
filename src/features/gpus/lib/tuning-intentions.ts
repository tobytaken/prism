import type { PerformanceMutationAction } from "@/features/gpus/components/gpu-tuning-screen";
import type {
  TuningEdits,
  TuningSetting,
} from "@/features/gpus/hooks/use-tuning-edits";
import type { GpuPerformanceDevice } from "@/features/gpus/types";

export function inTuningRange(
  value: number | null,
  min: number | null,
  max: number | null,
) {
  return (
    value != null &&
    Number.isFinite(value) &&
    min != null &&
    max != null &&
    value >= min &&
    value <= max
  );
}

export function retryTuningAction(
  gpu: GpuPerformanceDevice,
  setting: TuningSetting,
  value: TuningEdits[TuningSetting],
): PerformanceMutationAction | null {
  const gpuId = gpu.id;
  if (setting === "power") {
    if (gpu.power.support !== "supported") return null;
    if (value === "default")
      return gpu.power.defaultLimitWatts == null
        ? null
        : { kind: "resetPowerLimit", gpuId };
    return typeof value === "number" &&
      inTuningRange(value, gpu.power.minLimitWatts, gpu.power.maxLimitWatts)
      ? { kind: "setPowerLimit", gpuId, limitWatts: value }
      : null;
  }
  if (setting === "graphics" || setting === "memory") {
    const info = gpu.clocks[setting];
    return info.support === "supported" &&
      typeof value === "number" &&
      inTuningRange(value, info.minOffsetMHz, info.maxOffsetMHz)
      ? {
          kind: "setClockOffsets",
          gpuId,
          graphicsOffsetMHz: setting === "graphics" ? value : null,
          memoryOffsetMHz: setting === "memory" ? value : null,
        }
      : null;
  }
  const fanIndex = Number(setting.slice(4));
  const fan = gpu.cooling.fans.find((item) => item.index === fanIndex);
  if (!gpu.cooling.isSupported || !fan) return null;
  if (value === "auto") return { kind: "setFanModeAuto", gpuId, fanIndex };
  return typeof value === "number" &&
    inTuningRange(value, fan.minSpeedPercent ?? 0, fan.maxSpeedPercent ?? 100)
    ? { kind: "setFanSpeed", gpuId, fanIndex, speedPercent: value }
    : null;
}
