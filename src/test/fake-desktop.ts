import { vi } from "vitest";

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
 * the caller's profiles, and each GPU's last applied tuning values. Profile application follows
 * the daemon's validation, range corrections and per-setting outcomes. Every method is a spy, so
 * tests can assert calls or override one response.
 */
export function createFakeDesktop({
  response = createPerformanceResponse(),
  profiles = [],
}: {
  response?: GpuPerformanceResponse;
  profiles?: GpuTuningProfile[];
} = {}) {
  /** Setters that fail with this message on every GPU until removed. */
  const failures: Partial<Record<Setting, string>> = {};
  /** Last applied tuning values per GPU UUID, as the daemon records them. */
  const lastApplied: Record<string, GpuProfileSettings> = {};
  let nextId = profiles.length + 1;

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

  function applySettings(
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
    saveProfile: vi
      .spyOn(api, "saveProfile")
      .mockImplementation(async ({ gpuId, name }) => {
        const owners = response.gpus.filter((gpu) => gpu.stableId === gpuId);
        if (owners.length !== 1)
          throw new Error(
            `Profiles are unavailable because this GPU's stable identity (UUID) could not be verified. ${REFRESH}`,
          );
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
        const defaults = Object.entries(lastApplied[gpuId] ?? {})
          .filter(([, setting]) => setting.kind === "default")
          .map(([key]) => key as ProfileSettingKey);
        const saved: GpuTuningProfile = {
          id: `profile-${nextId++}`,
          name: trimmed,
          gpuId,
          settings: captureProfileSettings(owners[0], defaults),
        };
        profiles.push(saved);
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
        return applySettings(gpuId, settings);
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
          application: applySettings(saved.gpuId, settings),
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

  return {
    response,
    /** The first GPU, as the fake's readings report it. */
    get gpu() {
      return response.gpus[0];
    },
    profiles,
    failures,
    lastApplied,
    ...spies,
    /** How many live tuning writes the app requested. */
    liveWrites: () =>
      spies.setPowerLimit.mock.calls.length +
      spies.resetPowerLimit.mock.calls.length +
      spies.setClockOffsets.mock.calls.length +
      spies.resetClockOffsets.mock.calls.length,
  };
}

export type FakeDesktop = ReturnType<typeof createFakeDesktop>;
