import { useRef, useState } from "react";
import type { PerformanceMutationAction } from "@/features/gpus/components/gpu-tuning-controls";
import { retryTuningAction } from "@/features/gpus/lib/tuning-intentions";
import type {
  TuningMutationHandler,
  TuningMutationResult,
} from "@/features/gpus/lib/tuning-mutation";
import type {
  CoolingConfiguration,
  FanSetting,
  GpuPerformanceDevice,
  GpuProfileSettings,
  ProfileSetting,
  ProfileSettingKey,
  ProfileRangeCorrection,
  TuningOutcome,
} from "@/features/gpus/types";

export type TuningSetting =
  | "power"
  | "graphics"
  | "memory"
  | "cooling"
  | `fan:${number}`;
export type TuningEdits = {
  power?: number | null | "default";
  graphics?: number | null | "default";
  memory?: number | null | "default";
  /** A complete cooling draft, including its reviewed GPU and fan identities. */
  cooling?: CoolingDraft;
} & Partial<Record<`fan:${number}`, number | null | "auto">>;
/** A fan curve point as typed; null is an empty input. */
export type FanCurveDraftPoint = {
  temperatureC: number | null;
  speedPercent: number | null;
};
export type FanSettingDraft =
  | { mode: "automatic" }
  | { mode: "fixed"; speedPercent: number | null }
  | { mode: "curve"; points: FanCurveDraftPoint[] };
export type CoolingDraft = (
  | FanSettingDraft
  | {
      mode: "perFan";
      fans: { fanIndex: number; setting: FanSettingDraft }[];
    }
) & { stableId?: string | null; fanIndexes?: number[] };
type DeviceEdits = {
  edits: TuningEdits;
  coolingRevisions?: Partial<Record<TuningSetting, number>>;
  outcomes: Partial<Record<TuningSetting, TuningOutcome>>;
  profileOwners?: Partial<Record<TuningSetting, string>>;
  // Profile settings whose last successful application was an explicit default request.
  appliedDefaults?: Partial<Record<ProfileSettingKey, true>>;
};

function hasStaleCoolingDraft(
  device: DeviceEdits | undefined,
  revision: number | undefined,
) {
  return (
    revision != null &&
    Object.entries(device?.coolingRevisions ?? {}).some(
      ([setting, expected]) =>
        setting in (device?.edits ?? {}) && expected !== revision,
    )
  );
}

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
    if (
      (setting === "cooling" || setting.startsWith("fan:")) &&
      hasStaleCoolingDraft(device, gpu.cooling.control?.revision)
    )
      return null;
    const owner = device?.profileOwners?.[setting];
    const value = device?.edits[setting];
    if (owner && setting !== "cooling") {
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
    {
      reviewCorrections = false,
      coolingRevision = gpu.cooling.control?.revision ?? 0,
    } = {},
  ): Promise<ProfileRangeCorrection[] | undefined> {
    const intentions = actionIntentions(action, gpu);
    const coolingAction =
      action.kind === "applyCooling" ||
      action.kind === "setFanSpeed" ||
      action.kind === "setFanModeAuto" ||
      (action.kind === "applyProfile" && action.settings.cooling != null);
    if (coolingAction && gpu.cooling.control) {
      const revision = gpu.cooling.control.revision;
      const device = devices[gpu.id];
      if (hasStaleCoolingDraft(device, revision)) {
        return;
      }
      if (action.kind === "applyCooling")
        action = { ...action, expectedRevision: coolingRevision };
      else if (
        action.kind === "setFanSpeed" ||
        action.kind === "setFanModeAuto"
      )
        action = {
          ...action,
          expectedRevision: coolingRevision,
          stableId: gpu.stableId ?? undefined,
        };
    }
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
        coolingRevisions: coolingAction
          ? {
              ...device.coolingRevisions,
              ...Object.fromEntries(
                Object.keys(intentions)
                  .filter(
                    (setting) =>
                      setting === "cooling" || setting.startsWith("fan:"),
                  )
                  .map((setting) => [
                    setting,
                    setting in device.edits
                      ? (device.coolingRevisions?.[setting as TuningSetting] ??
                        coolingRevision)
                      : coolingRevision,
                  ]),
              ),
            }
          : device.coolingRevisions,
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
      const outcomes: DeviceEdits["outcomes"] = { ...device.outcomes };
      if (action.kind === "applyCooling" && result) {
        for (const [key, outcome] of Object.entries(result)) {
          if (key.startsWith("fan:"))
            outcomes[key as `fan:${number}`] = outcome as TuningOutcome;
        }
      }
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
      const coolingRevisions = { ...device.coolingRevisions };
      // A completed own write acknowledges only drafts based on the revision it replaced.
      // A stale rejection or transport error never acknowledges competing changes.
      if (result?.coolingRevision != null && coolingAction) {
        for (const key of Object.keys(coolingRevisions) as TuningSetting[]) {
          if (coolingRevisions[key] === coolingRevision)
            coolingRevisions[key] = result.coolingRevision;
        }
      }
      return {
        ...device,
        edits,
        outcomes,
        profileOwners,
        appliedDefaults,
        coolingRevisions,
      };
    });
  }

  return {
    devices,
    coolingNeedsReview(gpu: GpuPerformanceDevice) {
      return hasStaleCoolingDraft(
        devices[gpu.id],
        gpu.cooling.control?.revision,
      );
    },
    reviewCooling(gpuId: string, revision: number) {
      update(gpuId, (device) => ({
        ...device,
        coolingRevisions: Object.fromEntries(
          Object.keys(device.edits)
            .filter(
              (setting) => setting === "cooling" || setting.startsWith("fan:"),
            )
            .map((setting) => [setting, revision]),
        ),
      }));
    },
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
          const fans: Extract<
            CoolingConfiguration,
            { arrangement: "perFan" }
          >["fans"] = [];
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
            if (
              gpu?.stableId &&
              gpu.cooling.control &&
              (action.kind === "setFanSpeed" ||
                action.kind === "setFanModeAuto")
            ) {
              fans.push({
                fanIndex: action.fanIndex,
                setting:
                  action.kind === "setFanSpeed"
                    ? { mode: "fixed", speedPercent: action.speedPercent }
                    : { mode: "automatic" },
              });
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
          if (fans.length && gpu?.stableId)
            actions.push({
              kind: "applyCooling",
              gpuId,
              stableId: gpu.stableId,
              configuration: { arrangement: "perFan", fans },
              individualFanEdits: true,
            });
          if (gpu) {
            // Keep the original draft check, but advance the queue precondition after our own
            // writes. Another session changing cooling between requests still fails that check.
            let coolingRevision = gpu.cooling.control?.revision;
            for (const action of actions) {
              await apply(
                action,
                gpu,
                async (request) => {
                  const result = await handler(request);
                  coolingRevision = result?.coolingRevision ?? coolingRevision;
                  return result ?? {};
                },
                { coolingRevision },
              );
            }
          }
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
      coolingRevision?: number,
    ) {
      update(
        gpuId,
        ({
          edits,
          outcomes,
          profileOwners,
          appliedDefaults,
          coolingRevisions,
        }) => {
          const nextOutcomes = { ...outcomes };
          const nextOwners = { ...profileOwners };
          delete nextOwners[setting];
          delete nextOutcomes[setting];
          return {
            coolingRevisions:
              coolingRevision == null
                ? coolingRevisions
                : {
                    ...coolingRevisions,
                    [setting]:
                      setting in edits
                        ? (coolingRevisions?.[setting] ?? coolingRevision)
                        : coolingRevision,
                  },
            edits: { ...edits, [setting]: value },
            outcomes: nextOutcomes,
            profileOwners: nextOwners,
            appliedDefaults,
          };
        },
      );
    },
    discard(gpuId: string, setting: TuningSetting) {
      update(
        gpuId,
        ({
          edits,
          outcomes,
          profileOwners,
          appliedDefaults,
          coolingRevisions,
        }) => {
          const nextEdits = { ...edits };
          const nextOutcomes = { ...outcomes };
          const nextOwners = { ...profileOwners };
          delete nextEdits[setting];
          delete nextOutcomes[setting];
          delete nextOwners[setting];
          return {
            coolingRevisions,
            edits: nextEdits,
            outcomes: nextOutcomes,
            profileOwners: nextOwners,
            appliedDefaults,
          };
        },
      );
    },
    apply,
  };
}
export type TuningEditStore = ReturnType<typeof useTuningEdits>;

/** Preserve the complete configuration after a failed application. */
export function coolingDraft(
  configuration: CoolingConfiguration,
): CoolingDraft {
  return configuration.arrangement === "shared"
    ? configuration.setting
    : { mode: "perFan", fans: configuration.fans };
}

export function coolingConfiguration(
  draft: CoolingDraft,
): CoolingConfiguration | null {
  function setting(value: FanSettingDraft): FanSetting | null {
    if (value.mode === "automatic") return { mode: "automatic" };
    if (value.mode === "fixed")
      return Number.isInteger(value.speedPercent)
        ? { mode: "fixed", speedPercent: value.speedPercent! }
        : null;
    return value.points.every(
      (p) =>
        Number.isInteger(p.temperatureC) && Number.isInteger(p.speedPercent),
    )
      ? {
          mode: "curve",
          points: value.points.map((p) => ({
            temperatureC: p.temperatureC!,
            speedPercent: p.speedPercent!,
          })),
        }
      : null;
  }
  if (draft.mode !== "perFan") {
    const complete = setting(draft);
    return complete ? { arrangement: "shared", setting: complete } : null;
  }
  const fans = [];
  for (const fan of draft.fans) {
    const complete = setting(fan.setting);
    if (!complete) return null;
    fans.push({ fanIndex: fan.fanIndex, setting: complete });
  }
  return { arrangement: "perFan", fans };
}

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
      const { powerLimitWatts, graphicsOffsetMHz, memoryOffsetMHz, cooling } =
        action.settings;
      return {
        ...(cooling && {
          cooling: {
            ...coolingDraft(cooling),
            stableId: action.stableId,
            fanIndexes: gpu.cooling.fans.map((fan) => fan.index),
          },
        }),
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
    case "applyCooling":
      if (
        action.individualFanEdits &&
        action.configuration.arrangement === "perFan"
      )
        return Object.fromEntries(
          action.configuration.fans.flatMap(({ fanIndex, setting }) =>
            setting.mode === "curve"
              ? []
              : [
                  [
                    `fan:${fanIndex}`,
                    setting.mode === "automatic"
                      ? "auto"
                      : setting.speedPercent,
                  ],
                ],
          ),
        );
      return {
        cooling: {
          ...coolingDraft(action.configuration),
          stableId: action.stableId,
          fanIndexes:
            action.fanIndexes ?? gpu.cooling.fans.map((fan) => fan.index),
        },
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
