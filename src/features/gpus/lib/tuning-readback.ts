import type { GpuPerformanceDevice } from "@/features/gpus/types";

export function unavailableTuningReadback(
  gpu: GpuPerformanceDevice,
  error: string,
): GpuPerformanceDevice {
  return {
    ...gpu,
    power: { ...gpu.power, currentLimitWatts: null, currentValueError: error },
    clocks: {
      graphics: {
        ...gpu.clocks.graphics,
        currentOffsetMHz: null,
        currentValueError: error,
      },
      memory: {
        ...gpu.clocks.memory,
        currentOffsetMHz: null,
        currentValueError: error,
      },
    },
    cooling: {
      ...gpu.cooling,
      fans: gpu.cooling.fans.map((fan) => ({
        ...fan,
        currentSpeedPercent: null,
        mode: null,
      })),
    },
  };
}
