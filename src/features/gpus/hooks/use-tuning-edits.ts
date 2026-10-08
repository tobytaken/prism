import { useRef, useState } from "react";
import type { PerformanceMutationAction } from "@/features/gpus/components/gpu-tuning-controls";
import { retryTuningAction } from "@/features/gpus/lib/tuning-intentions";
import type {
  TuningMutationHandler,
  TuningMutationResult,
} from "@/features/gpus/lib/tuning-mutation";
import type {
  GpuPerformanceDevice,
  GpuProfileSettings,
  ProfileSetting,
  ProfileSettingKey,
  ProfileRangeCorrection,
  TuningOutcome,
} from "@/features/gpus/types";

export type TuningSetting = "power" | "graphics" | "memory" | `fan:${number}`;
export type TuningEdits = {
  power?: number | null | "default";
  graphics?: number | null | "default";
  memory?: number | null | "default";
} & Partial<Record<`fan:${number}`, number | null | "auto">>;
/** A fan curve point as typed; null is an empty input. */
export type FanCurveDraftPoint = {
  temperatureC: number | null;
  speedPercent: number | null;
};
type DeviceEdits = {
  edits: TuningEdits;
  outcomes: Partial<Record<TuningSetting, TuningOutcome>>;
  profileOwners?: Partial<Record<TuningSetting, string>>;
  // Profile settings whose last successful application was an explicit default request.
  appliedDefaults?: Partial<Record<ProfileSettingKey, true>>;
};

// Observations never acknowledge an intention: only a command outcome or discard does.
export function useTuningEdits() {
  const [devices, setDevices] = useState<Record<string, DeviceEdits>>({});
  // Curve drafts are screen-local like other edits, but only previewed: Prism can't apply them
  // yet, so they aren't unapplied tuning changes.
  const [curveDrafts, setCurveDrafts] = useState<
    Record<string, FanCurveDraftPoint[]>
  >({});
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
      const key = profileSettingKey(setting);
      const instruction = profileInstruction(value);
      if (gpu.stableId !== owner || !key || !instruction) return null;
      return {
        kind: "applyProfile" as const,
        gpuId: gpu.id,
        stableId: owner,
        settings: { [key]: instruction },
      };
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
      const settings: GpuProfileSettings = {};
      if (
        owner &&
        Object.entries(intentions).every(([setting, value]) => {
          const key = profileSettingKey(setting as TuningSetting);
          const instruction = profileInstruction(value);
          if (!key || !instruction) return false;
          settings[key] = instruction;
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
    const withdrawIntentions = () =>
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
    let result: TuningMutationResult | void;
    try {
      result = await handler(action);
    } catch (error) {
      // The daemon saves reviewed corrections before writing, so a rejected correction is
      // reported with its review rather than as an outcome.
      if (action.kind === "applyProfile" && action.corrections) {
        withdrawIntentions();
        throw error;
      }
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
      withdrawIntentions();
      return result.corrections;
    }
    update(gpu.id, (device) => {
      const edits = { ...device.edits };
      const outcomes = { ...device.outcomes };
      const profileOwners = { ...device.profileOwners };
      const appliedDefaults = { ...device.appliedDefaults };
      for (const setting of Object.keys(intentions) as TuningSetting[]) {
        const key = profileSettingKey(setting);
        // The GPU accepted this instruction even if the user has since edited it again.
        if (key && result?.[setting]?.status === "succeeded") {
          if (intentions[setting] === "default") appliedDefaults[key] = true;
          else delete appliedDefaults[key];
        }
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
      return { edits, outcomes, profileOwners, appliedDefaults };
    });
  }

  return {
    devices,
    retryAction,
    appliedDefaults(gpuId: string): ProfileSettingKey[] {
      return Object.keys(
        devices[gpuId]?.appliedDefaults ?? {},
      ) as ProfileSettingKey[];
    },
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
                    error: !gpu
                      ? "GPU unavailable."
                      : device.edits[setting] === "default"
                        ? "The default for this control is unavailable or outside its current range. Refresh the GPU readings and try again."
                        : "The edit requires an available control and a value within its current range.",
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
      update(gpuId, ({ edits, outcomes, profileOwners, appliedDefaults }) => {
        const nextOutcomes = { ...outcomes };
        const nextOwners = { ...profileOwners };
        delete nextOwners[setting];
        delete nextOutcomes[setting];
        return {
          edits: { ...edits, [setting]: value },
          outcomes: nextOutcomes,
          profileOwners: nextOwners,
          appliedDefaults,
        };
      });
    },
    discard(gpuId: string, setting: TuningSetting) {
      update(gpuId, ({ edits, outcomes, profileOwners, appliedDefaults }) => {
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
          appliedDefaults,
        };
      });
    },
    apply,
    /** The GPU's edited fan curve, or null while it shows the starter curve. */
    curveDraft(gpuId: string): FanCurveDraftPoint[] | null {
      return curveDrafts[gpuId] ?? null;
    },
    /** Null returns the GPU to its starter curve. */
    changeCurveDraft(gpuId: string, points: FanCurveDraftPoint[] | null) {
      setCurveDrafts((current) => {
        const next = { ...current };
        if (points) next[gpuId] = points;
        else delete next[gpuId];
        return next;
      });
    },
  };
}
export type TuningEditStore = ReturnType<typeof useTuningEdits>;

function profileInstruction(value: unknown): ProfileSetting | null {
  if (value === "default") return { kind: "default" };
  return typeof value === "number" ? { kind: "value", value } : null;
}

function profileSettingKey(setting: TuningSetting): ProfileSettingKey | null {
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
    case "applyProfile": {
      const intention = (setting: ProfileSetting) =>
        setting.kind === "default" ? ("default" as const) : setting.value;
      const { powerLimitWatts, graphicsOffsetMHz, memoryOffsetMHz } =
        action.settings;
      return {
        ...(powerLimitWatts && { power: intention(powerLimitWatts) }),
        ...(graphicsOffsetMHz && { graphics: intention(graphicsOffsetMHz) }),
        ...(memoryOffsetMHz && { memory: intention(memoryOffsetMHz) }),
      };
    }
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
        ...(gpu.clocks.graphics.support !== "unsupported" && {
          graphics: "default" as const,
        }),
        ...(gpu.clocks.memory.support !== "unsupported" && {
          memory: "default" as const,
        }),
      };
  }
}
