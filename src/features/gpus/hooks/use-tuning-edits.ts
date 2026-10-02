import { useRef, useState } from "react";
import type { PerformanceMutationAction } from "@/features/gpus/components/gpu-tuning-screen";
import { retryTuningAction } from "@/features/gpus/lib/tuning-intentions";
import type {
  TuningMutationHandler,
  TuningMutationResult,
} from "@/features/gpus/lib/tuning-mutation";
import type {
  GpuPerformanceDevice,
  TuningOutcome,
} from "@/features/gpus/types";

export type TuningSetting = "power" | "graphics" | "memory" | `fan:${number}`;
export type TuningEdits = {
  power?: number | null | "default";
  graphics?: number | null;
  memory?: number | null;
} & Partial<Record<`fan:${number}`, number | null | "auto">>;
type DeviceEdits = {
  edits: TuningEdits;
  outcomes: Partial<Record<TuningSetting, TuningOutcome>>;
};

// Observations never acknowledge an intention: only a command outcome or discard does.
export function useTuningEdits() {
  const [devices, setDevices] = useState<Record<string, DeviceEdits>>({});
  const [applyAllPending, setApplyAllPending] = useState(false);
  const applyAllRunning = useRef(false);
  function update(gpuId: string, change: (device: DeviceEdits) => DeviceEdits) {
    setDevices((current) => ({
      ...current,
      [gpuId]: change(current[gpuId] ?? { edits: {}, outcomes: {} }),
    }));
  }
  async function apply(
    action: PerformanceMutationAction,
    gpu: GpuPerformanceDevice,
    handler: TuningMutationHandler,
  ) {
    const intentions = actionIntentions(action, gpu);
    update(gpu.id, (device) => ({
      ...device,
      edits: { ...device.edits, ...intentions },
    }));
    let result: TuningMutationResult | void;
    try {
      result = await handler(action);
    } catch (error) {
      // A rejected transport cannot establish which writes reached the GPU.
      const outcome: TuningOutcome = {
        status: "unknown",
        error: error instanceof Error ? error.message : String(error),
      };
      result = Object.fromEntries(
        Object.keys(intentions).map((setting) => [setting, outcome]),
      );
    }
    update(gpu.id, (device) => {
      const edits = { ...device.edits };
      const outcomes = { ...device.outcomes };
      for (const setting of Object.keys(intentions) as TuningSetting[]) {
        if (edits[setting] !== intentions[setting]) continue;
        const outcome = result?.[setting] ?? {
          status: "unknown",
          error: "No application outcome received",
        };
        outcomes[setting] = outcome;
        if (outcome.status === "succeeded") delete edits[setting];
      }
      return { edits, outcomes };
    });
  }

  return {
    devices,
    applyAllPending,
    async applyAll(
      gpus: readonly GpuPerformanceDevice[],
      handler: TuningMutationHandler,
    ) {
      if (applyAllRunning.current) return;
      applyAllRunning.current = true;
      setApplyAllPending(true);
      try {
        for (const [gpuId, device] of Object.entries(devices)) {
          const gpu = gpus.find((gpu) => gpu.id === gpuId);
          const actions: PerformanceMutationAction[] = [];
          for (const setting of Object.keys(device.edits) as TuningSetting[]) {
            const action =
              gpu && retryTuningAction(gpu, setting, device.edits[setting]);
            if (!action) {
              update(gpuId, (current) => ({
                ...current,
                outcomes: {
                  ...current.outcomes,
                  [setting]: {
                    status: "unattempted",
                    error: gpu
                      ? "The edit requires an available control and a value within its current range."
                      : "GPU unavailable.",
                  },
                },
              }));
              continue;
            }
            const clocks = actions.find(
              (action) => action.kind === "setClockOffsets",
            );
            if (action.kind === "setClockOffsets" && clocks) {
              clocks.graphicsOffsetMHz ??= action.graphicsOffsetMHz;
              clocks.memoryOffsetMHz ??= action.memoryOffsetMHz;
            } else {
              actions.push(action);
            }
          }
          if (gpu)
            for (const action of actions) await apply(action, gpu, handler);
        }
      } finally {
        applyAllRunning.current = false;
        setApplyAllPending(false);
      }
    },
    change<K extends TuningSetting>(
      gpuId: string,
      setting: K,
      value: TuningEdits[K],
    ) {
      update(gpuId, ({ edits, outcomes }) => {
        const nextOutcomes = { ...outcomes };
        delete nextOutcomes[setting];
        return {
          edits: { ...edits, [setting]: value },
          outcomes: nextOutcomes,
        };
      });
    },
    discard(gpuId: string, setting: TuningSetting) {
      update(gpuId, ({ edits, outcomes }) => {
        const nextEdits = { ...edits };
        const nextOutcomes = { ...outcomes };
        delete nextEdits[setting];
        delete nextOutcomes[setting];
        return { edits: nextEdits, outcomes: nextOutcomes };
      });
    },
    apply,
  };
}
export type TuningEditStore = ReturnType<typeof useTuningEdits>;

function actionIntentions(
  action: PerformanceMutationAction,
  gpu: GpuPerformanceDevice,
): TuningEdits {
  switch (action.kind) {
    case "setPowerLimit":
      return { power: action.limitWatts };
    case "resetPowerLimit":
      return { power: "default" };
    case "setFanSpeed":
      return { [`fan:${action.fanIndex}`]: action.speedPercent };
    case "setFanModeAuto":
      return { [`fan:${action.fanIndex}`]: "auto" as const };
    case "setClockOffsets":
      return {
        ...(action.graphicsOffsetMHz != null && {
          graphics: action.graphicsOffsetMHz,
        }),
        ...(action.memoryOffsetMHz != null && {
          memory: action.memoryOffsetMHz,
        }),
      };
    case "resetClockOffsets":
      return {
        ...(gpu.clocks.graphics.support !== "unsupported" && { graphics: 0 }),
        ...(gpu.clocks.memory.support !== "unsupported" && { memory: 0 }),
      };
  }
}
