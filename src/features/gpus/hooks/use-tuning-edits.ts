import { useRef, useState } from "react";
import type { PerformanceMutationAction } from "@/features/gpus/components/gpu-tuning-screen";
import { retryTuningAction } from "@/features/gpus/lib/tuning-intentions";
import type {
  TuningMutationHandler,
  TuningMutationResult,
} from "@/features/gpus/lib/tuning-mutation";
import type {
  GpuPerformanceDevice,
  NumericProfileSettings,
  ProfileRangeCorrection,
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
  profileOwners?: Partial<Record<TuningSetting, string>>;
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
  function retryAction(gpu: GpuPerformanceDevice, setting: TuningSetting) {
    const device = devices[gpu.id];
    const owner = device?.profileOwners?.[setting];
    const value = device?.edits[setting];
    if (owner) {
      if (gpu.stableId !== owner || typeof value !== "number") return null;
      const key = profileSettingKey(setting);
      return key
        ? {
            kind: "applyProfile" as const,
            gpuId: gpu.id,
            stableId: owner,
            settings: { [key]: value },
          }
        : null;
    }
    return retryTuningAction(gpu, setting, value);
  }
  // With reviewCorrections, proposed range corrections are returned instead of retained as edits.
  async function apply(
    action: PerformanceMutationAction,
    gpu: GpuPerformanceDevice,
    handler: TuningMutationHandler,
    { reviewCorrections = false } = {},
  ): Promise<ProfileRangeCorrection[] | undefined> {
    const intentions = actionIntentions(action, gpu);
    // Existing Apply buttons also retry retained profile instructions through validation.
    if (action.kind !== "applyProfile") {
      const owner = (Object.keys(intentions) as TuningSetting[])
        .map((setting) => devices[gpu.id]?.profileOwners?.[setting])
        .find((owner) => owner != null);
      const settings: NumericProfileSettings = {};
      if (
        owner &&
        Object.entries(intentions).every(([setting, value]) => {
          const key = profileSettingKey(setting as TuningSetting);
          if (!key || typeof value !== "number") return false;
          settings[key] = value;
          return true;
        })
      ) {
        action = {
          kind: "applyProfile",
          gpuId: gpu.id,
          stableId: owner,
          settings,
        };
      }
    }
    update(gpu.id, (device) => {
      const profileOwners = { ...device.profileOwners };
      for (const setting of Object.keys(intentions) as TuningSetting[]) {
        if (action.kind === "applyProfile")
          profileOwners[setting] = action.stableId;
        else delete profileOwners[setting];
      }
      return {
        ...device,
        profileOwners,
        edits: { ...device.edits, ...intentions },
      };
    });
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
    if (reviewCorrections && result?.corrections?.length) {
      // Nothing was written, so declining the review must leave no trace.
      update(gpu.id, (device) => {
        const edits = { ...device.edits };
        const profileOwners = { ...device.profileOwners };
        for (const setting of Object.keys(intentions) as TuningSetting[]) {
          if (edits[setting] !== intentions[setting]) continue;
          delete edits[setting];
          delete profileOwners[setting];
        }
        return { ...device, edits, profileOwners };
      });
      return result.corrections;
    }
    update(gpu.id, (device) => {
      const edits = { ...device.edits };
      const outcomes = { ...device.outcomes };
      const profileOwners = { ...device.profileOwners };
      for (const setting of Object.keys(intentions) as TuningSetting[]) {
        if (edits[setting] !== intentions[setting]) continue;
        const outcome = result?.[setting] ?? {
          status: "unknown",
          error: "No application outcome received",
        };
        outcomes[setting] = outcome;
        if (outcome.status === "succeeded") {
          delete edits[setting];
          delete profileOwners[setting];
        }
      }
      return { edits, outcomes, profileOwners };
    });
  }

  return {
    devices,
    retryAction,
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
            const action = gpu && retryAction(gpu, setting);
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
            const profile = actions.find(
              (action) => action.kind === "applyProfile",
            );
            if (
              action.kind === "applyProfile" &&
              profile?.kind === "applyProfile"
            ) {
              Object.assign(profile.settings, action.settings);
            } else if (action.kind === "setClockOffsets" && clocks) {
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
      update(gpuId, ({ edits, outcomes, profileOwners }) => {
        const nextOutcomes = { ...outcomes };
        const nextOwners = { ...profileOwners };
        delete nextOwners[setting];
        delete nextOutcomes[setting];
        return {
          edits: { ...edits, [setting]: value },
          outcomes: nextOutcomes,
          profileOwners: nextOwners,
        };
      });
    },
    discard(gpuId: string, setting: TuningSetting) {
      update(gpuId, ({ edits, outcomes, profileOwners }) => {
        const nextEdits = { ...edits };
        const nextOutcomes = { ...outcomes };
        const nextOwners = { ...profileOwners };
        delete nextEdits[setting];
        delete nextOutcomes[setting];
        delete nextOwners[setting];
        return {
          edits: nextEdits,
          outcomes: nextOutcomes,
          profileOwners: nextOwners,
        };
      });
    },
    apply,
  };
}
export type TuningEditStore = ReturnType<typeof useTuningEdits>;

function profileSettingKey(
  setting: TuningSetting,
): keyof NumericProfileSettings | null {
  return setting === "power"
    ? "powerLimitWatts"
    : setting === "graphics"
      ? "graphicsOffsetMHz"
      : setting === "memory"
        ? "memoryOffsetMHz"
        : null;
}

function actionIntentions(
  action: PerformanceMutationAction,
  gpu: GpuPerformanceDevice,
): TuningEdits {
  switch (action.kind) {
    case "applyProfile":
      return {
        ...(action.settings.powerLimitWatts != null && {
          power: action.settings.powerLimitWatts,
        }),
        ...(action.settings.graphicsOffsetMHz != null && {
          graphics: action.settings.graphicsOffsetMHz,
        }),
        ...(action.settings.memoryOffsetMHz != null && {
          memory: action.settings.memoryOffsetMHz,
        }),
      };
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
