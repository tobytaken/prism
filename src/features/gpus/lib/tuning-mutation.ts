import * as api from "@/features/gpus/api";
import type { PerformanceMutationAction } from "@/features/gpus/components/gpu-tuning-screen";
import type { TuningSetting } from "@/features/gpus/hooks/use-tuning-edits";
import type { GpuCommandResponse, TuningOutcome } from "@/features/gpus/types";

export type TuningMutationResult = Partial<
  Record<TuningSetting, TuningOutcome>
>;
export type TuningMutationHandler = (
  action: PerformanceMutationAction,
) => Promise<TuningMutationResult> | void;

export async function runTuningMutation(
  action: PerformanceMutationAction,
): Promise<TuningMutationResult> {
  switch (action.kind) {
    case "setPowerLimit":
      return {
        power: commandOutcome(
          await api.setGpuPowerLimit(action.gpuId, action.limitWatts),
        ),
      };
    case "resetPowerLimit":
      return {
        power: commandOutcome(await api.resetGpuPowerLimit(action.gpuId)),
      };
    case "setFanSpeed":
      return {
        [`fan:${action.fanIndex}`]: commandOutcome(
          await api.setGpuFanSpeed(
            action.gpuId,
            action.fanIndex,
            action.speedPercent,
          ),
        ),
      };
    case "setFanModeAuto":
      return {
        [`fan:${action.fanIndex}`]: commandOutcome(
          await api.setGpuFanModeAuto(action.gpuId, action.fanIndex),
        ),
      };
    case "setClockOffsets":
    case "resetClockOffsets": {
      const result =
        action.kind === "setClockOffsets"
          ? await api.setGpuClockOffsets(
              action.gpuId,
              action.graphicsOffsetMHz,
              action.memoryOffsetMHz,
            )
          : await api.resetGpuClockOffsets(action.gpuId);
      return {
        ...(result.graphics && { graphics: result.graphics }),
        ...(result.memory && { memory: result.memory }),
      };
    }
  }
}

function commandOutcome(response: GpuCommandResponse): TuningOutcome {
  return {
    status: response.success ? "succeeded" : "failed",
    error: response.error,
  };
}
