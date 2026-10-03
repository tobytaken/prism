import * as api from "@/features/gpus/api";
import type { PerformanceMutationAction } from "@/features/gpus/components/gpu-tuning-screen";
import type { TuningSetting } from "@/features/gpus/hooks/use-tuning-edits";
import type {
  GpuCommandResponse,
  ProfileRangeCorrection,
  TuningOutcome,
} from "@/features/gpus/types";

export type TuningOutcomeMap = Partial<Record<TuningSetting, TuningOutcome>>;
export type TuningMutationResult = TuningOutcomeMap & {
  /** Proposed by fresh validation when only range bounds blocked a profile; nothing was written. */
  corrections?: ProfileRangeCorrection[];
};
export type TuningMutationHandler = (
  action: PerformanceMutationAction,
) => Promise<TuningMutationResult> | void;

export async function runTuningMutation(
  action: PerformanceMutationAction,
): Promise<TuningMutationResult> {
  switch (action.kind) {
    case "applyProfile": {
      const result = await api.applyGpuProfile({
        gpuId: action.stableId,
        settings: action.settings,
      });
      return {
        ...(result.power && { power: result.power }),
        ...(result.graphics && { graphics: result.graphics }),
        ...(result.memory && { memory: result.memory }),
        ...(result.corrections?.length && { corrections: result.corrections }),
      };
    }
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
              offsetValue(action.graphicsOffsetMHz),
              offsetValue(action.memoryOffsetMHz),
            )
          : await api.resetGpuClockOffsets(action.gpuId);
      return {
        ...(result.graphics && { graphics: result.graphics }),
        ...(result.memory && { memory: result.memory }),
      };
    }
  }
}

function offsetValue(offset: number | "default" | null) {
  return offset === "default" ? 0 : offset;
}

function commandOutcome(response: GpuCommandResponse): TuningOutcome {
  return {
    status: response.success ? "succeeded" : "failed",
    error: response.error,
  };
}
