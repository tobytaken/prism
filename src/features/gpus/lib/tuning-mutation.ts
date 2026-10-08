import * as api from "@/features/gpus/api";
import type { PerformanceMutationAction } from "@/features/gpus/components/gpu-tuning-controls";
import type { TuningSetting } from "@/features/gpus/hooks/use-tuning-edits";
import type {
  GpuCommandResponse,
  ProfileRangeCorrection,
  TuningOutcome,
} from "@/features/gpus/types";

export type TuningOutcomeMap = Partial<Record<TuningSetting, TuningOutcome>>;
export type TuningMutationResult = TuningOutcomeMap & {
  coolingRevision?: number;
  /** Proposed by fresh validation when only range bounds blocked a profile; nothing was written. */
  corrections?: ProfileRangeCorrection[];
};
export type TuningMutationHandler = (
  action: PerformanceMutationAction,
) => Promise<TuningMutationResult> | void;

/** The failure to show for a rejected action; a rejected correction is shown in its review. */
export function mutationErrorMessage(
  error: Error | null,
  action: PerformanceMutationAction | undefined,
) {
  if (action?.kind === "applyProfile" && action.corrections) return null;
  return error?.message ?? null;
}

export async function runTuningMutation(
  action: PerformanceMutationAction,
): Promise<TuningMutationResult> {
  if (
    (action.kind === "setFanSpeed" || action.kind === "setFanModeAuto") &&
    action.stableId &&
    action.expectedRevision != null
  ) {
    const result = await api.applyGpuCooling({
      gpuId: action.stableId,
      expectedRevision: action.expectedRevision,
      configuration: {
        arrangement: "perFan",
        fans: [
          {
            fanIndex: action.fanIndex,
            setting:
              action.kind === "setFanSpeed"
                ? { mode: "fixed", speedPercent: action.speedPercent }
                : { mode: "automatic" },
          },
        ],
      },
    });
    return {
      [`fan:${action.fanIndex}`]: result.fans[0]?.outcome ?? result.outcome,
      ...(result.outcome.status !== "unattempted" && {
        coolingRevision: result.revision,
      }),
    };
  }
  switch (action.kind) {
    case "applyProfile": {
      const result = action.corrections
        ? (
            await api.correctAndApplyGpuProfile({
              profileId: action.profileId!,
              corrections: action.corrections,
            })
          ).application
        : action.profileId
          ? await api.applyGpuProfileById({ profileId: action.profileId })
          : await api.applyGpuProfile({
              gpuId: action.stableId,
              settings: action.settings,
            });
      return {
        ...(result.cooling && {
          cooling: {
            ...result.cooling.outcome,
            error:
              result.cooling.outcome.error ??
              (result.cooling.fans
                .filter((f) => f.outcome.error)
                .map((f) => `Fan ${f.fanIndex + 1}: ${f.outcome.error}`)
                .join("; ") ||
                null),
          },
          ...(result.cooling.outcome.status !== "unattempted" && {
            coolingRevision: result.cooling.revision,
          }),
        }),
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
    case "applyCooling": {
      const application = await api.applyGpuCooling({
        gpuId: action.stableId,
        configuration: action.configuration,
        ...(action.fanIndexes && { fanIndexes: action.fanIndexes }),
        expectedRevision: action.expectedRevision,
      });
      const fanOutcomes =
        action.configuration.arrangement === "perFan"
          ? action.configuration.fans.map(({ fanIndex }) => ({
              fanIndex,
              outcome:
                application.fans.find((fan) => fan.fanIndex === fanIndex)
                  ?.outcome ?? application.outcome,
            }))
          : application.fans;
      const unsuccessful = application.fans
        .filter((fan) => fan.outcome.status === "failed")
        .map((fan) => `Fan ${fan.fanIndex + 1}: ${fan.outcome.error}`);
      return {
        ...Object.fromEntries(
          fanOutcomes.map((fan) => [`fan:${fan.fanIndex}`, fan.outcome]),
        ),
        ...(application.outcome.status !== "unattempted" && {
          coolingRevision: application.revision,
        }),
        cooling: {
          status: application.outcome.status,
          error:
            application.outcome.error ??
            (unsuccessful.length ? unsuccessful.join("; ") : null),
        },
      };
    }
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
