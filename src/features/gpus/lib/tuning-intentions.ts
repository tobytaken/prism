import type { PerformanceMutationAction } from "@/features/gpus/components/gpu-tuning-controls";
import {
  coolingConfiguration,
  type TuningEdits,
  type TuningSetting,
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
      return inTuningRange(
        gpu.power.defaultLimitWatts,
        gpu.power.minLimitWatts,
        gpu.power.maxLimitWatts,
      )
        ? { kind: "resetPowerLimit", gpuId }
        : null;
    return typeof value === "number" &&
      inTuningRange(value, gpu.power.minLimitWatts, gpu.power.maxLimitWatts)
      ? { kind: "setPowerLimit", gpuId, limitWatts: value }
      : null;
  }
  if (setting === "graphics" || setting === "memory") {
    const info = gpu.clocks[setting];
    const offset = value === "default" ? 0 : value;
    return info.support === "supported" &&
      typeof offset === "number" &&
      inTuningRange(offset, info.minOffsetMHz, info.maxOffsetMHz)
      ? {
          kind: "setClockOffsets",
          gpuId,
          graphicsOffsetMHz:
            setting !== "graphics"
              ? null
              : value === "default"
                ? value
                : offset,
          memoryOffsetMHz:
            setting !== "memory" ? null : value === "default" ? value : offset,
        }
      : null;
  }
  if (setting === "cooling") {
    const draft = value as TuningEdits["cooling"];
    const configuration = draft && coolingConfiguration(draft);
    return configuration && gpu.stableId
      ? { kind: "applyCooling", gpuId, stableId: gpu.stableId, configuration }
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
