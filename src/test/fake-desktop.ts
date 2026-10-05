import { vi, type MockInstance } from "vitest";

import {
  captureProfileSettings,
  PROFILE_SETTING_KEYS,
} from "@/features/gpus/lib/tuning-profiles";
import type {
  ClockOffsetResponse,
  ControlSupport,
  GpuPerformanceDevice,
  GpuPerformanceResponse,
  GpuProfileSettings,
  GpuTuningProfile,
  GpuTuningState,
  OffsetInstruction,
  ProfileApplicationResponse,
  ProfileRangeCorrection,
  ProfileSetting,
  ProfileSettingKey,
  TuningOutcome,
} from "@/features/gpus/types";
import { createPerformanceResponse } from "./gpu-fixtures";

type Setting = "power" | "graphics" | "memory";

const SETTINGS: Record<ProfileSettingKey, Setting> = {
  powerLimitWatts: "power",
  graphicsOffsetMHz: "graphics",
  memoryOffsetMHz: "memory",
};

const REFRESH = "Refresh the GPU readings and try again.";
const STALE =
  "The GPU's values changed since you last looked. Review them and save again.";

type Control = {
  label: string;
  support: ControlSupport;
  reason: string | null;
  default: number | null;
  min: number | null;
  max: number | null;
  write: (value: number) => void;
};

function control(gpu: GpuPerformanceDevice, key: ProfileSettingKey): Control {
  if (key === "powerLimitWatts")
    return {
      label: "Power limit",
      support: gpu.power.support,
      reason: gpu.power.reason,
      default: gpu.power.defaultLimitWatts,
      min: gpu.power.minLimitWatts,
      max: gpu.power.maxLimitWatts,
      write: (value) => (gpu.power.currentLimitWatts = value),
    };
  const info =
    key === "graphicsOffsetMHz" ? gpu.clocks.graphics : gpu.clocks.memory;
  return {
    label:
      key === "graphicsOffsetMHz"
        ? "Graphics clock offset"
        : "Memory clock offset",
    support: info.support,
    reason: info.reason,
    default: 0,
    min: info.minOffsetMHz,
    max: info.maxOffsetMHz,
    write: (value) => (info.currentOffsetMHz = value),
  };
}

const succeeded: TuningOutcome = { status: "succeeded", error: null };

/**
 * A stateful stand-in for the desktop API backed by the daemon: GPU readings that writes change,
 * the caller's profiles, other users' profiles, each GPU's current profile and last applied
 * tuning values, and the revision that guards saves. Profile application follows the daemon's
 * validation, range corrections and per-setting outcomes. Every method is a spy, so tests can
 * assert calls or override one response.
 */
export function createFakeDesktop({
  response = createPerformanceResponse(),
  profiles = [],
  otherUsersProfiles = [],
  currentProfiles = {},
}: {
  response?: GpuPerformanceResponse;
  profiles?: GpuTuningProfile[];
  /** Profiles the caller can't list, which can still be a GPU's current profile. */
  otherUsersProfiles?: GpuTuningProfile[];
  /** Each GPU's current profile id, by GPU UUID. */
  currentProfiles?: Record<string, string>;
} = {}) {
  /** Setters that fail with this message on every GPU until removed. */
  const failures: Partial<Record<Setting, string>> = {};
  /** Last applied tuning values per GPU UUID, as the daemon records them. */
  const lastApplied: Record<string, GpuProfileSettings> = {};
  const tuningStateListeners = new Set<(state: GpuTuningState) => void>();
  let nextId = profiles.length + 1;

  function tuningState(gpu: GpuPerformanceDevice): GpuTuningState {
    const gpuId = gpu.stableId!;
    const reported = {
      powerLimitWatts: gpu.power.currentLimitWatts,
      graphicsOffsetMHz: gpu.clocks.graphics.currentOffsetMHz,
      memoryOffsetMHz: gpu.clocks.memory.currentOffsetMHz,
    };
    const controls = PROFILE_SETTING_KEYS.map((key) => {
      const { support, default: resolved } = control(gpu, key);
      return [support, reported[key], resolved];
    });
    const currentId = currentProfiles[gpuId];
    const current = [...profiles, ...otherUsersProfiles].find(
      (profile) => profile.id === currentId,
    );
    return {
      gpuId,
      revision: JSON.stringify([
        controls,
        lastApplied[gpuId],
        current ? { id: current.id, name: current.name } : null,
      ]),
      reported,
      currentProfile: current ? { id: current.id, name: current.name } : null,
    };
  }
  function owner(gpuId: string) {
    const owners = response.gpus.filter((gpu) => gpu.stableId === gpuId);
    if (owners.length !== 1)
      throw new Error(
        `Profiles are unavailable because this GPU's stable identity (UUID) could not be verified. ${REFRESH}`,
      );
    return owners[0];
  }
  function checkRevision(gpu: GpuPerformanceDevice, revision: string) {
    if (tuningState(gpu).revision !== revision) throw new Error(STALE);
  }
  function capture(gpu: GpuPerformanceDevice) {
    const defaults = Object.entries(lastApplied[gpu.stableId!] ?? {})
      .filter(([, setting]) => setting.kind === "default")
      .map(([key]) => key as ProfileSettingKey);
    return captureProfileSettings(gpu, defaults);
  }

  const byId = (gpuId: string) => response.gpus.find((gpu) => gpu.id === gpuId);
  const record = (
    gpu: GpuPerformanceDevice | undefined,
    key: ProfileSettingKey,
    setting: ProfileSetting,
  ) => {
    if (!gpu?.stableId) return;
    lastApplied[gpu.stableId] = {
      ...lastApplied[gpu.stableId],
      [key]: setting,
    };
  };
  const write = (
    gpu: GpuPerformanceDevice | undefined,
    setting: Setting,
    change: (gpu: GpuPerformanceDevice) => void,
  ): string | null => {
    if (failures[setting]) return failures[setting]!;
    if (!gpu) return "GPU unavailable";
    change(gpu);
    return null;
  };

  /** Applies the settings; a saved profile's application also updates the current profile. */
  function applySettings(
    gpuId: string,
    settings: GpuProfileSettings,
    profileId?: string,
  ): ProfileApplicationResponse {
    const result = writeSettings(gpuId, settings);
    const outcomes = Object.values(result).filter(
      (outcome): outcome is TuningOutcome =>
        outcome != null && "status" in outcome,
    );
    const applied = outcomes.filter(({ status }) => status === "succeeded");
    if (profileId && applied.length === outcomes.length)
      currentProfiles[gpuId] = profileId;
    else if (profileId && applied.length) delete currentProfiles[gpuId];
    return result;
  }

  function writeSettings(
    gpuId: string,
    settings: GpuProfileSettings,
  ): ProfileApplicationResponse {
    const included = PROFILE_SETTING_KEYS.filter((key) => settings[key]);
    const blocked = (
      error: string,
      corrections: ProfileRangeCorrection[] = [],
    ): ProfileApplicationResponse => {
      const result: ProfileApplicationResponse = {
        power: null,
        graphics: null,
        memory: null,
      };
      for (const key of included)
        result[SETTINGS[key]] = { status: "unattempted", error };
      if (corrections.length) result.corrections = corrections;
      return result;
    };
    const owners = response.gpus.filter(
      (gpu) => gpuId.trim() && gpu.stableId === gpuId,
    );
    if (owners.length !== 1)
      return blocked(
        owners.length
          ? `The owning GPU's stable identity is ambiguous. ${REFRESH}`
          : `The owning GPU is absent or its stable identity could not be verified. ${REFRESH}`,
      );
    const gpu = owners[0];
    const errors: string[] = [];
    // Like the daemon, range problems are listed after every other blocker.
    const rangeErrors: string[] = [];
    const corrections: ProfileRangeCorrection[] = [];
    const values: Partial<Record<ProfileSettingKey, number>> = {};
    for (const key of included) {
      const setting = settings[key]!;
      const { label, support, reason, min, max, ...info } = control(gpu, key);
      if (support === "unsupported") {
        errors.push(
          `${label} is unsupported. ${reason ?? "Choose a compatible profile."}`,
        );
        continue;
      }
      if (support === "unknown") {
        errors.push(`${label} support is unknown. ${reason ?? REFRESH}`);
        continue;
      }
      const value = setting.kind === "value" ? setting.value : info.default;
      if (value == null) {
        errors.push(`${label}: the default is unavailable. ${REFRESH}`);
        continue;
      }
      if (min == null || max == null || min > max) {
        errors.push(
          `${label}: the current allowed range is unavailable. ${REFRESH}`,
        );
        continue;
      }
      if (value >= min && value <= max) {
        values[key] = value;
        continue;
      }
      if (setting.kind === "default") {
        errors.push(
          `${label}: the default ${value} is outside the current allowed range ${min} to ${max}. ${REFRESH}`,
        );
        continue;
      }
      rangeErrors.push(
        `${label}: requested ${value} is outside the current allowed range ${min} to ${max}. Discard this change and load the profile again to review a correction.`,
      );
      corrections.push({
        setting: key,
        savedValue: value,
        proposedValue: value < min ? min : max,
        bound: value < min ? "minimum" : "maximum",
      });
    }
    if (errors.length || rangeErrors.length)
      return blocked(
        [...errors, ...rangeErrors].join("\n"),
        errors.length === 0 ? corrections : [],
      );
    const result: ProfileApplicationResponse = {
      power: null,
      graphics: null,
      memory: null,
    };
    let failed = false;
    for (const key of included) {
      const setting = SETTINGS[key];
      if (failed) {
        result[setting] = {
          status: "unattempted",
          error:
            "An earlier profile setting failed. Retry or discard this instruction.",
        };
        continue;
      }
      const error = write(gpu, setting, (gpu) =>
        control(gpu, key).write(values[key]!),
      );
      if (error) {
        failed = true;
        result[setting] = { status: "failed", error };
      } else {
        result[setting] = succeeded;
        record(gpu, key, settings[key]!);
      }
    }
    return result;
  }

  function profile(profileId: string) {
    const found = profiles.find((profile) => profile.id === profileId);
    if (!found)
      throw new Error(
        "The GPU tuning profile no longer exists. No GPU settings were changed.",
      );
    return found;
  }

  function setOffsets(
    gpuId: string,
    graphics: OffsetInstruction,
    memory: OffsetInstruction,
  ): ClockOffsetResponse {
    const gpu = byId(gpuId);
    let failed = false;
    const apply = (key: ProfileSettingKey, instruction: OffsetInstruction) => {
      if (instruction == null) return null;
      if (failed)
        return {
          status: "unattempted",
          error: "An earlier clock offset failed",
        } satisfies TuningOutcome;
      const value = instruction === "default" ? 0 : instruction;
      const error = write(gpu, SETTINGS[key], (gpu) =>
        control(gpu, key).write(value),
      );
      if (error) {
        failed = true;
        return { status: "failed", error } satisfies TuningOutcome;
      }
      record(
        gpu,
        key,
        instruction === "default"
          ? { kind: "default" }
          : { kind: "value", value },
      );
      return succeeded;
    };
    return {
      graphics: apply("graphicsOffsetMHz", graphics),
      memory: apply("memoryOffsetMHz", memory),
    };
  }

  const api = window.prism.gpu;
  const spies = {
    getPerformance: vi
      .spyOn(api, "getPerformance")
      .mockImplementation(async () => structuredClone(response)),
    listProfiles: vi
      .spyOn(api, "listProfiles")
      .mockImplementation(async () => structuredClone(profiles)),
    renameProfile: vi
      .spyOn(api, "renameProfile")
      .mockImplementation(async ({ profileId, name }) => {
        const saved = profile(profileId);
        const trimmed = name.trim();
        if (!trimmed) throw new Error("Enter a name for the profile.");
        if (
          profiles.some(
            (other) =>
              other.id !== profileId &&
              other.gpuId === saved.gpuId &&
              other.name.toLowerCase() === trimmed.toLowerCase(),
          )
        )
          throw new Error(
            `A profile named "${trimmed}" already exists for this GPU. Choose another name.`,
          );
        saved.name = trimmed;
        return structuredClone(saved);
      }),
    deleteProfile: vi
      .spyOn(api, "deleteProfile")
      .mockImplementation(async ({ profileId }) => {
        profile(profileId);
        const index = profiles.findIndex((profile) => profile.id === profileId);
        profiles.splice(index, 1);
        for (const [gpuId, currentId] of Object.entries(currentProfiles))
          if (currentId === profileId) delete currentProfiles[gpuId];
        return { success: true, error: null };
      }),
    getTuningStates: vi
      .spyOn(api, "getTuningStates")
      .mockImplementation(async () =>
        response.gpus
          .filter(
            (gpu) =>
              gpu.stableId?.trim() &&
              response.gpus.filter((other) => other.stableId === gpu.stableId)
                .length === 1,
          )
          .map(tuningState),
      ),
    onTuningStateChanged: vi
      .spyOn(api, "onTuningStateChanged")
      .mockImplementation((listener) => {
        tuningStateListeners.add(listener);
        return () => tuningStateListeners.delete(listener);
      }),
    saveProfileToCurrent: vi
      .spyOn(api, "saveProfileToCurrent")
      .mockImplementation(async ({ gpuId, revision }) => {
        const gpu = owner(gpuId);
        checkRevision(gpu, revision);
        const currentId = currentProfiles[gpuId];
        const current = profiles.find((profile) => profile.id === currentId);
        const others = otherUsersProfiles.find(
          (profile) => profile.id === currentId,
        );
        if (others)
          throw new Error(
            `The current profile "${others.name}" belongs to another user, so you can't save over it. Save as a new profile instead.`,
          );
        if (!current)
          throw new Error(
            "This GPU has no current profile. Save as a new profile instead.",
          );
        current.settings = capture(gpu);
        return structuredClone(current);
      }),
    saveProfile: vi
      .spyOn(api, "saveProfile")
      .mockImplementation(async ({ gpuId, name, revision }) => {
        const gpu = owner(gpuId);
        checkRevision(gpu, revision);
        const trimmed = name.trim();
        if (
          profiles.some(
            (profile) =>
              profile.gpuId === gpuId &&
              profile.name.toLowerCase() === trimmed.toLowerCase(),
          )
        )
          throw new Error(
            `A profile named "${trimmed}" already exists for this GPU. Choose another name.`,
          );
        const saved: GpuTuningProfile = {
          id: `profile-${nextId++}`,
          name: trimmed,
          gpuId,
          settings: capture(gpu),
        };
        profiles.push(saved);
        currentProfiles[gpuId] = saved.id;
        return structuredClone(saved);
      }),
    applyProfile: vi
      .spyOn(api, "applyProfile")
      .mockImplementation(async ({ gpuId, settings }) =>
        applySettings(gpuId, settings),
      ),
    applyProfileById: vi
      .spyOn(api, "applyProfileById")
      .mockImplementation(async ({ profileId }) => {
        const { gpuId, settings } = profile(profileId);
        return applySettings(gpuId, settings, profileId);
      }),
    correctAndApplyProfile: vi
      .spyOn(api, "correctAndApplyProfile")
      .mockImplementation(async ({ profileId, corrections }) => {
        const saved = profile(profileId);
        const settings = { ...saved.settings };
        for (const { setting, savedValue, proposedValue } of corrections) {
          const current = settings[setting];
          if (current?.kind !== "value" || current.value !== savedValue)
            throw new Error(
              "The saved profile changed while its corrections were being reviewed. Load it again to review current corrections. No GPU settings were changed.",
            );
          settings[setting] = { kind: "value", value: proposedValue };
        }
        saved.settings = settings;
        return {
          profile: structuredClone(saved),
          application: applySettings(saved.gpuId, settings, profileId),
        };
      }),
    setPowerLimit: vi
      .spyOn(api, "setPowerLimit")
      .mockImplementation(async (gpuId, watts) => {
        const gpu = byId(gpuId);
        const error = write(gpu, "power", (gpu) => {
          gpu.power.currentLimitWatts = watts;
        });
        if (!error)
          record(gpu, "powerLimitWatts", { kind: "value", value: watts });
        return { success: !error, error };
      }),
    resetPowerLimit: vi
      .spyOn(api, "resetPowerLimit")
      .mockImplementation(async (gpuId) => {
        const gpu = byId(gpuId);
        const error = write(gpu, "power", (gpu) => {
          gpu.power.currentLimitWatts = gpu.power.defaultLimitWatts;
        });
        if (!error) record(gpu, "powerLimitWatts", { kind: "default" });
        return { success: !error, error };
      }),
    setClockOffsets: vi
      .spyOn(api, "setClockOffsets")
      .mockImplementation(async (gpuId, graphics, memory) =>
        setOffsets(gpuId, graphics, memory),
      ),
    resetClockOffsets: vi
      .spyOn(api, "resetClockOffsets")
      .mockImplementation(async (gpuId) => {
        const gpu = byId(gpuId);
        const supported = (support?: ControlSupport) =>
          support === "supported" ? ("default" as const) : null;
        return setOffsets(
          gpuId,
          supported(gpu?.clocks.graphics.support),
          supported(gpu?.clocks.memory.support),
        );
      }),
    setFanSpeed: vi
      .spyOn(api, "setFanSpeed")
      .mockImplementation(async (gpuId, fanIndex, speedPercent) => {
        const fan = byId(gpuId)?.cooling.fans[fanIndex];
        if (fan)
          Object.assign(fan, {
            currentSpeedPercent: speedPercent,
            mode: "manual",
          });
        return { success: true, error: null };
      }),
    setFanModeAuto: vi
      .spyOn(api, "setFanModeAuto")
      .mockImplementation(async (gpuId, fanIndex) => {
        const fan = byId(gpuId)?.cooling.fans[fanIndex];
        if (fan) fan.mode = "auto";
        return { success: true, error: null };
      }),
  };

  // Like the daemon, every write or save pushes the tuning state of each GPU it changed.
  const identifiedStates = () =>
    new Map(
      response.gpus
        .filter((gpu) => gpu.stableId?.trim())
        .map((gpu) => [gpu.stableId!, JSON.stringify(tuningState(gpu))]),
    );
  for (const spy of [
    spies.saveProfile,
    spies.saveProfileToCurrent,
    spies.renameProfile,
    spies.deleteProfile,
    spies.applyProfile,
    spies.applyProfileById,
    spies.correctAndApplyProfile,
    spies.setPowerLimit,
    spies.resetPowerLimit,
    spies.setClockOffsets,
    spies.resetClockOffsets,
  ] as unknown as MockInstance<(...args: unknown[]) => Promise<unknown>>[]) {
    const write = spy.getMockImplementation()!;
    spy.mockImplementation(async (...args) => {
      const before = identifiedStates();
      try {
        return await write(...args);
      } finally {
        for (const [gpuId, state] of identifiedStates())
          if (before.get(gpuId) !== state) {
            const changed = JSON.parse(state) as GpuTuningState;
            for (const listener of tuningStateListeners) listener(changed);
          }
      }
    });
  }

  return {
    response,
    /** The first GPU, as the fake's readings report it. */
    get gpu() {
      return response.gpus[0];
    },
    profiles,
    currentProfiles,
    failures,
    lastApplied,
    ...spies,
    /** Pushes the GPU's tuning state to the app, as the daemon does after another session's change. */
    pushTuningState(gpuId: string) {
      const state = tuningState(owner(gpuId));
      for (const listener of tuningStateListeners) listener(state);
    },
    /** How many live tuning writes the app requested. */
    liveWrites: () =>
      spies.setPowerLimit.mock.calls.length +
      spies.resetPowerLimit.mock.calls.length +
      spies.setClockOffsets.mock.calls.length +
      spies.resetClockOffsets.mock.calls.length,
  };
}

export type FakeDesktop = ReturnType<typeof createFakeDesktop>;
