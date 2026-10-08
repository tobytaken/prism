import { vi, type MockInstance } from "vitest";

import {
  captureProfileSettings,
  PROFILE_SETTING_KEYS,
} from "@/features/gpus/lib/tuning-profiles";
import type {
  ClockOffsetResponse,
  CoolingApplication,
  CoolingConfiguration,
  CoolingControl,
  FanRestoration,
  FanSetting,
  ControlSupport,
  FanCurvePoint,
  FanCurveProblem,
  FanCurveSupport,
  GpuPerformanceDevice,
  GpuPerformanceResponse,
  GpuProfileSettings,
  GpuTuningProfile,
  GpuTuningState,
  GpuDaemonStatus,
  OffsetInstruction,
  ProfileApplicationResponse,
  ProfileRangeCorrection,
  ProfileSetting,
  ProfileSettingKey,
  TuningOutcome,
  StartupBehavior,
  StartupOutcome,
} from "@/features/gpus/types";
import type {
  AutomaticSwitchingStatus,
  GameEntry,
  GameId,
  SteamApp,
  RunningGamesState,
} from "@/features/games/types";
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

/** Targeted support at the same desktop boundary as the shared preview fixture. */
function targetedCurveSupport(
  gpu: GpuPerformanceDevice,
  fanIndex: number,
): FanCurveSupport {
  const fan = gpu.cooling.fans.find((fan) => fan.index === fanIndex);
  const maxTemperature = gpu.cooling.maxOperatingTemperatureC;
  if (
    !fan ||
    fan.minSpeedPercent == null ||
    fan.maxSpeedPercent == null ||
    maxTemperature == null
  )
    return {
      status: "unavailable",
      reason:
        "The fan's speed range or maximum operating temperature is unavailable.",
    };
  const min = fan.minSpeedPercent,
    max = fan.maxSpeedPercent,
    full = maxTemperature - 5;
  return {
    status: "available",
    maxOperatingTemperatureC: maxTemperature,
    fullSpeedTemperatureC: full,
    minSpeedPercent: min,
    maxSpeedPercent: max,
    starter: [
      { temperatureC: full - 40, speedPercent: min },
      {
        temperatureC: full - 20,
        speedPercent: Math.ceil(min + (max - min) * 0.5),
      },
      {
        temperatureC: full - 10,
        speedPercent: Math.ceil(min + (max - min) * 0.75),
      },
      { temperatureC: full, speedPercent: max },
    ],
  };
}

/** The daemon's shared fan curve validation, against the limits it reported for the GPU. */
function fanCurveProblems(
  support: FanCurveSupport,
  points: FanCurvePoint[],
): FanCurveProblem[] {
  if (support.status === "unavailable")
    return [{ point: null, message: support.reason }];
  const {
    maxOperatingTemperatureC: maxTemperature,
    fullSpeedTemperatureC: fullSpeed,
    minSpeedPercent: min,
    maxSpeedPercent: max,
  } = support;
  const problems: FanCurveProblem[] = [];
  const problem = (point: number, message: string) =>
    problems.push({ point, message: `Point ${point + 1}: ${message}` });
  if (points.length < 2 || points.length > 16)
    problems.push({ point: null, message: "A curve needs 2 to 16 points." });
  points.forEach(({ temperatureC, speedPercent }, index) => {
    if (temperatureC < 0 || temperatureC > maxTemperature)
      problem(index, `temperature must be between 0 and ${maxTemperature}°C.`);
    if (speedPercent < min || speedPercent > max)
      problem(index, `speed must be between ${min}% and ${max}%.`);
    const previous = points[index - 1];
    if (previous && temperatureC <= previous.temperatureC)
      problem(
        index,
        `temperature must be higher than point ${index}'s ${previous.temperatureC}°C.`,
      );
    if (previous && speedPercent < previous.speedPercent)
      problem(
        index,
        `speed can't be lower than point ${index}'s ${previous.speedPercent}%.`,
      );
  });
  if (
    points.length &&
    !points.some(
      (point) => point.temperatureC <= fullSpeed && point.speedPercent === max,
    )
  )
    problems.push({
      point: null,
      message: `The curve must reach ${max}% by ${fullSpeed}°C, 5°C below the GPU's ${maxTemperature}°C maximum operating temperature.`,
    });
  return problems;
}

/** The daemon's curve output: a straight line between points, rounded up, ends held. */
function requestedSpeed(points: FanCurvePoint[], temperatureC: number) {
  if (temperatureC <= points[0].temperatureC) return points[0].speedPercent;
  const last = points[points.length - 1];
  if (temperatureC >= last.temperatureC) return last.speedPercent;
  const high = points.findIndex((point) => point.temperatureC >= temperatureC);
  const [from, to] = [points[high - 1], points[high]];
  return (
    from.speedPercent +
    Math.ceil(
      ((to.speedPercent - from.speedPercent) *
        (temperatureC - from.temperatureC)) /
        (to.temperatureC - from.temperatureC),
    )
  );
}

/** Each fan's applied setting, by fan index. */
function fanSettings(
  applied: CoolingConfiguration | null,
  gpu: GpuPerformanceDevice,
): Map<number, FanSetting> {
  if (!applied) return new Map();
  if (applied.arrangement === "shared")
    return new Map(gpu.cooling.fans.map((fan) => [fan.index, applied.setting]));
  return new Map(applied.fans.map((fan) => [fan.fanIndex, fan.setting]));
}

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
  startupBehavior = "off",
  startupProfiles = {},
  startupOutcomes = [],
  possiblyStale = [],
  steamApps = [],
  games = [],
  automaticSwitching = false,
  runningGames = { status: "available", games: [] },
  switchingStatus = { condition: { status: "off" }, gpus: [] },
}: {
  response?: GpuPerformanceResponse;
  profiles?: GpuTuningProfile[];
  /** Profiles the caller can't list, which can still be a GPU's current profile. */
  otherUsersProfiles?: GpuTuningProfile[];
  /** Each GPU's current profile id, by GPU UUID. */
  currentProfiles?: Record<string, string>;
  startupBehavior?: StartupBehavior;
  startupProfiles?: Record<string, string>;
  startupOutcomes?: StartupOutcome[];
  /** GPU UUIDs whose last applied tuning values the daemon reports as possibly stale. */
  possiblyStale?: string[];
  /** Fully installed Steam apps in the caller's libraries. */
  steamApps?: SteamApp[];
  /** The caller's game associations; installed is derived from steamApps. */
  games?: Omit<GameEntry, "installed">[];
  automaticSwitching?: boolean;
  runningGames?: RunningGamesState;
  switchingStatus?: AutomaticSwitchingStatus;
} = {}) {
  /** Setters that fail with this message on every GPU until removed. */
  const failures: Partial<Record<Setting, string>> = {};
  /** Fans whose speed writes fail with this message until removed. */
  const fanFailures: Record<number, string> = {};
  /** Last applied tuning values per GPU UUID, as the daemon records them. */
  const lastApplied: Record<string, GpuProfileSettings> = {};
  const daemonListeners = new Set<(status: GpuDaemonStatus) => void>();
  vi.spyOn(window.prism.gpu, "onDaemonStatusChanged").mockImplementation(
    (listener) => {
      daemonListeners.add(listener);
      return () => {
        daemonListeners.delete(listener);
      };
    },
  );
  const runningListeners = new Set<(state: RunningGamesState) => void>();
  const switchingListeners = new Set<
    (status: AutomaticSwitchingStatus) => void
  >();
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
  const coolingControl = (gpu: GpuPerformanceDevice): CoolingControl =>
    (gpu.cooling.control ??= {
      revision: 0,
      applied: null,
      requested: [],
      curveTemperatureC: null,
      suspension: null,
      recovery: null,
    });
  /** Like the daemon, stops the GPU's curve and returns its fans to automatic cooling. */
  const suspendCurve = (
    gpu: GpuPerformanceDevice,
    reason: string,
    restoration?: FanRestoration[],
  ) => {
    const control = coolingControl(gpu);
    if (!control.requested.length) return;
    control.revision += 1;
    control.recovery = null;
    control.suspension = {
      kind: "fault",
      reason,
      restoration:
        restoration ??
        control.requested.map(({ fanIndex }) => ({
          fanIndex,
          status: "restored" as const,
          detail: null,
        })),
    };
    // Only a confirmed restoration means the driver reports automatic control.
    for (const { fanIndex, status } of control.suspension.restoration)
      if (status === "restored")
        gpu.cooling.fans.find((fan) => fan.index === fanIndex)!.mode = "auto";
    control.requested = [];
    control.curveTemperatureC = null;
  };
  /** Replaces the fans' applied settings, keeping the arrangement only when every fan took it. */
  const applyFans = (
    gpu: GpuPerformanceDevice,
    configuration: CoolingConfiguration,
    targets: [number, FanSetting][],
  ): CoolingApplication => {
    const control = coolingControl(gpu);
    control.revision += 1;
    control.recovery = null;
    const outcomes: CoolingApplication["fans"] = [];
    const settings = fanSettings(control.applied, gpu);
    let failure: string | null = null;
    for (const [fanIndex, setting] of targets) {
      if (failure) {
        outcomes.push({
          fanIndex,
          outcome: {
            status: "unattempted",
            error: "An earlier fan failed. Retry or discard this change.",
          },
        });
        continue;
      }
      const fan = gpu.cooling.fans.find((fan) => fan.index === fanIndex)!;
      const speed =
        setting.mode === "fixed"
          ? setting.speedPercent
          : setting.mode === "curve"
            ? requestedSpeed(setting.points, gpu.telemetry.temperatureC!)
            : null;
      const rejected = fanFailures[fanIndex] && speed != null;
      if (!rejected || setting.mode === "curve") {
        control.requested = control.requested.filter(
          (request) => request.fanIndex !== fanIndex,
        );
        settings.set(fanIndex, setting);
        if (setting.mode === "curve")
          control.requested.push({ fanIndex, speedPercent: speed! });
      }
      if (rejected) {
        failure = `Fan ${fanIndex + 1}: ${fanFailures[fanIndex]}`;
        outcomes.push({
          fanIndex,
          outcome: { status: "failed", error: fanFailures[fanIndex] },
        });
        continue;
      }
      Object.assign(
        fan,
        speed == null
          ? { mode: "auto" }
          : { mode: "manual", currentSpeedPercent: speed },
      );
      outcomes.push({ fanIndex, outcome: succeeded });
    }
    control.requested.sort((a, b) => a.fanIndex - b.fanIndex);
    control.applied =
      configuration.arrangement === "shared" &&
      outcomes.every(
        (fan) =>
          fan.outcome.status === "succeeded" ||
          (configuration.setting.mode === "curve" &&
            fan.outcome.status === "failed"),
      )
        ? configuration
        : {
            arrangement: "perFan",
            fans: [...settings].map(([fanIndex, setting]) => ({
              fanIndex,
              setting,
            })),
          };
    control.curveTemperatureC = control.requested.length
      ? gpu.telemetry.temperatureC
      : null;
    if (failure) suspendCurve(gpu, failure);
    else if (
      !control.requested.length ||
      targets.some(([, s]) => s.mode === "curve")
    )
      control.suspension = null;
    return {
      revision: control.revision,
      outcome: {
        status: outcomes.every((fan) => fan.outcome.status === "succeeded")
          ? "succeeded"
          : "failed",
        error: null,
      },
      fans: outcomes,
    };
  };
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

  const sameGame = (a: GameId, b: GameId) =>
    a.kind === b.kind && a.appId === b.appId;
  /** The caller's games by name, each installed while its Steam app is, as the daemon reports. */
  function gamesState() {
    return structuredClone({
      automaticSwitching,
      games: games
        .map((game) => {
          const app = steamApps.find(({ appId }) => appId === game.game.appId);
          return { ...game, name: app?.name ?? game.name, installed: !!app };
        })
        .sort((a, b) => a.name.localeCompare(b.name)),
    });
  }
  function association(game: GameId) {
    const found = games.find((entry) => sameGame(entry.game, game));
    if (!found) throw new Error("This game is no longer on your Games page.");
    return found;
  }

  const gamesApi = window.prism.games;
  vi.spyOn(gamesApi, "getRunning").mockImplementation(async () => runningGames);
  vi.spyOn(gamesApi, "onRunningChanged").mockImplementation((listener) => {
    runningListeners.add(listener);
    return () => {
      runningListeners.delete(listener);
    };
  });
  vi.spyOn(gamesApi, "getSwitchingStatus").mockImplementation(async () =>
    structuredClone(switchingStatus),
  );
  vi.spyOn(gamesApi, "onSwitchingStatusChanged").mockImplementation(
    (listener) => {
      switchingListeners.add(listener);
      return () => {
        switchingListeners.delete(listener);
      };
    },
  );
  const gameSpies = {
    listSteamApps: vi
      .spyOn(gamesApi, "listSteamApps")
      .mockImplementation(async () =>
        structuredClone(
          [...steamApps].sort((a, b) => a.name.localeCompare(b.name)),
        ),
      ),
    getGames: vi
      .spyOn(gamesApi, "get")
      .mockImplementation(async () => gamesState()),
    addSteamGame: vi
      .spyOn(gamesApi, "addSteamGame")
      .mockImplementation(async ({ appId }) => {
        const app = steamApps.find((app) => app.appId === appId);
        if (!app)
          throw new Error(
            `Steam app ${appId} isn't installed in your Steam libraries.`,
          );
        if (!games.some(({ game }) => game.appId === appId))
          games.push({
            game: { kind: "steam", appId },
            name: app.name,
            profiles: {},
          });
        return gamesState();
      }),
    setGameProfile: vi
      .spyOn(gamesApi, "setProfile")
      .mockImplementation(async ({ game, gpuId, profileId }) => {
        if (profileId !== null) {
          const linked = profile(profileId);
          if (linked.gpuId !== gpuId)
            throw new Error(
              `The profile "${linked.name}" belongs to another GPU.`,
            );
        }
        const entry = association(game);
        if (profileId === null) delete entry.profiles[gpuId];
        else entry.profiles[gpuId] = profileId;
        return gamesState();
      }),
    removeGame: vi
      .spyOn(gamesApi, "remove")
      .mockImplementation(async ({ game }) => {
        const index = games.findIndex((entry) => sameGame(entry.game, game));
        if (index >= 0) games.splice(index, 1);
        return gamesState();
      }),
    setAutomaticSwitching: vi
      .spyOn(gamesApi, "setAutomaticSwitching")
      .mockImplementation(async ({ enabled }) => {
        automaticSwitching = enabled;
        return gamesState();
      }),
  };

  const api = window.prism.gpu;
  const spies = {
    getStartupState: vi
      .spyOn(api, "getStartupState")
      .mockImplementation(async () => ({
        behavior: startupBehavior,
        profiles: Object.entries(startupProfiles).flatMap(([gpuId, id]) => {
          const saved = [...profiles, ...otherUsersProfiles].find(
            (profile) => profile.id === id,
          );
          return saved ? [{ gpuId, profile: { id, name: saved.name } }] : [];
        }),
        outcomes: structuredClone(startupOutcomes),
        possiblyStale: [...possiblyStale],
      })),
    setStartupBehavior: vi
      .spyOn(api, "setStartupBehavior")
      .mockImplementation(async ({ behavior }) => {
        startupBehavior = behavior;
        return { success: true, error: null };
      }),
    setStartupProfile: vi
      .spyOn(api, "setStartupProfile")
      .mockImplementation(async ({ gpuId, profileId }) => {
        if (profileId === null) delete startupProfiles[gpuId];
        else {
          if (profile(profileId).gpuId !== gpuId)
            throw new Error("The startup profile belongs to another GPU.");
          startupProfiles[gpuId] = profileId;
        }
        return { success: true, error: null };
      }),
    dismissStartupOutcome: vi
      .spyOn(api, "dismissStartupOutcome")
      .mockImplementation(async ({ gpuId }) => {
        const index = startupOutcomes.findIndex(
          (outcome) => outcome.gpuId === gpuId,
        );
        if (index >= 0) startupOutcomes.splice(index, 1);
        return { success: true, error: null };
      }),
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
        for (const [gpuId, startupId] of Object.entries(startupProfiles))
          if (startupId === profileId) delete startupProfiles[gpuId];
        for (const game of games)
          for (const [gpuId, linkedId] of Object.entries(game.profiles))
            if (linkedId === profileId) delete game.profiles[gpuId];
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
        const gpu = byId(gpuId);
        const setting = { mode: "fixed" as const, speedPercent };
        if (gpu)
          applyFans(gpu, { arrangement: "perFan", fans: [] }, [
            [fanIndex, setting],
          ]);
        return { success: true, error: null };
      }),
    applyCooling: vi
      .spyOn(api, "applyCooling")
      .mockImplementation(
        async ({ gpuId, configuration, fanIndexes, expectedRevision }) => {
          const gpu = response.gpus.find((gpu) => gpu.stableId === gpuId);
          const targets: [number, FanSetting][] =
            configuration.arrangement === "shared"
              ? (gpu?.cooling.fans ?? []).map((fan) => [
                  fan.index,
                  configuration.setting,
                ])
              : configuration.fans.map((fan) => [fan.fanIndex, fan.setting]);
          const problems: string[] = [];
          if (
            gpu &&
            expectedRevision != null &&
            coolingControl(gpu).revision !== expectedRevision
          )
            problems.push(
              "Cooling changed since this draft was opened. Review the current cooling controls before applying again.",
            );
          if (!gpu)
            problems.push(
              "The GPU is absent or its stable identity could not be verified. Refresh the GPU readings and try again.",
            );
          if (
            gpu &&
            fanIndexes &&
            (fanIndexes.length !== gpu.cooling.fans.length ||
              fanIndexes.some(
                (index) => !gpu.cooling.fans.some((fan) => fan.index === index),
              ))
          )
            problems.push(
              "The targeted fans changed. Review the current fan inventory before applying.",
            );
          const seen = new Set<number>();
          for (const [index, setting] of gpu ? targets : []) {
            const fan = gpu!.cooling.fans.find((fan) => fan.index === index);
            if (!fan || seen.has(index))
              problems.push(`Fan ${index + 1} is missing or duplicated.`);
            seen.add(index);
            if (
              fan &&
              setting.mode === "fixed" &&
              (setting.speedPercent < (fan.minSpeedPercent ?? 0) ||
                setting.speedPercent > (fan.maxSpeedPercent ?? 100))
            )
              problems.push(
                `Fan ${index + 1}: speed is outside its supported range.`,
              );
            if (setting.mode === "curve")
              problems.push(
                ...fanCurveProblems(
                  configuration.arrangement === "shared"
                    ? gpu!.cooling.curveSupport!
                    : targetedCurveSupport(gpu!, index),
                  setting.points,
                ).map((problem) => problem.message),
              );
          }
          if (
            gpu?.telemetry.temperatureC == null &&
            targets.some(([, setting]) => setting.mode === "curve")
          )
            problems.push(
              "The GPU's temperature is unavailable, so a curve can't be followed. Refresh the GPU readings and try again.",
            );
          if (problems.length) {
            const error = [...new Set(problems)].join("\n");
            return {
              revision: gpu ? coolingControl(gpu).revision : 0,
              outcome: { status: "unattempted", error },
              fans: targets.map(([fanIndex]) => ({
                fanIndex,
                outcome: { status: "unattempted", error },
              })),
            };
          }
          const application = applyFans(gpu!, configuration, targets);
          if (
            application.outcome.status === "succeeded" &&
            targets.some(([, setting]) => setting.mode === "curve")
          ) {
            const control = coolingControl(gpu!);
            control.rememberedCurve = structuredClone(control.applied!);
          }
          return application;
        },
      ),
    previewFanCurve: vi
      .spyOn(api, "previewFanCurve")
      .mockImplementation(async ({ gpuId, points, fanIndex }) => {
        const gpu = byId(gpuId);
        const support =
          gpu &&
          (fanIndex == null
            ? gpu.cooling.curveSupport
            : targetedCurveSupport(gpu, fanIndex));
        if (!support)
          throw new Error(
            "The GPU is unavailable. Refresh the GPU readings and try again.",
          );
        return {
          problems: fanCurveProblems(support, points),
          ...(fanIndex != null && { support }),
        };
      }),
    setFanModeAuto: vi
      .spyOn(api, "setFanModeAuto")
      .mockImplementation(async (gpuId, fanIndex) => {
        const gpu = byId(gpuId);
        if (gpu)
          applyFans(gpu, { arrangement: "perFan", fans: [] }, [
            [fanIndex, { mode: "automatic" }],
          ]);
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
    startupProfiles,
    startupOutcomes,
    possiblyStale,
    get startupBehavior() {
      return startupBehavior;
    },
    failures,
    fanFailures,
    lastApplied,
    steamApps,
    games,
    get automaticSwitching() {
      return automaticSwitching;
    },
    ...spies,
    ...gameSpies,
    pushDaemonStatus(status: GpuDaemonStatus) {
      for (const listener of daemonListeners) listener(status);
    },
    pushRunningGames(state: RunningGamesState) {
      runningGames = state;
      for (const listener of runningListeners) listener(state);
    },
    pushSwitchingStatus(status: AutomaticSwitchingStatus) {
      switchingStatus = status;
      for (const listener of switchingListeners) listener(status);
    },
    /** Pushes the GPU's tuning state to the app, as the daemon does after another session's change. */
    pushTuningState(gpuId: string) {
      const state = tuningState(owner(gpuId));
      for (const listener of tuningStateListeners) listener(state);
    },
    /**
     * Follows each running curve at the GPU's current temperature, as the daemon's loop does.
     * Rises apply at once; a drop applies only once the caller says demand has stayed lower for
     * the daemon's five-second delay. A missing temperature suspends the curve.
     */
    followCurves({ decreaseDelayElapsed = false } = {}) {
      for (const gpu of response.gpus) {
        const control = gpu.cooling.control;
        const settings = fanSettings(control?.applied ?? null, gpu);
        if (!control?.requested.length) continue;
        if (
          [...settings].some(([fanIndex, setting]) => {
            if (
              setting.mode === "curve" &&
              !control.requested.some(
                (request) => request.fanIndex === fanIndex,
              )
            )
              return false;
            const mode = gpu.cooling.fans.find(
              (fan) => fan.index === fanIndex,
            )?.mode;
            return (
              mode != null &&
              mode !== (setting.mode === "automatic" ? "auto" : "manual")
            );
          })
        ) {
          control.revision += 1;
          control.recovery = null;
          control.suspension = {
            kind: "externalControl",
            reason:
              "The driver reports a fan control mode different from Prism's applied controls. Prism left those controls alone.",
            restoration: [],
          };
          control.requested = [];
          control.curveTemperatureC = null;
          continue;
        }
        if (gpu.telemetry.temperatureC == null) {
          suspendCurve(
            gpu,
            "The GPU's temperature is unavailable, so Prism stopped following its curve.",
          );
          continue;
        }
        for (const request of control.requested) {
          const setting = settings.get(request.fanIndex);
          if (setting?.mode !== "curve") continue;
          const demand = requestedSpeed(
            setting.points,
            gpu.telemetry.temperatureC,
          );
          if (demand < request.speedPercent && !decreaseDelayElapsed) continue;
          request.speedPercent = demand;
          gpu.cooling.fans.find(
            (fan) => fan.index === request.fanIndex,
          )!.currentSpeedPercent = request.speedPercent;
        }
        control.curveTemperatureC = gpu.telemetry.temperatureC;
      }
    },
    /**
     * Restarts the daemon within this host boot. Like the daemon, each running curve resumes at
     * the current temperature, or returns its fans to automatic cooling and waits for a retry
     * when the temperature is unavailable or a fan write fails.
     */
    restartDaemon() {
      for (const gpu of response.gpus) {
        const control = gpu.cooling.control;
        if (!control?.requested.length) continue;
        const settings = fanSettings(control.applied, gpu);
        const temperature = gpu.telemetry.temperatureC;
        control.revision += 1;
        const unavailable =
          temperature == null
            ? "The GPU's temperature is unavailable, so a curve can't be followed. Refresh the GPU readings and try again."
            : null;
        let failure: string | null = unavailable;
        const fans: CoolingApplication["fans"] = control.requested.map(
          ({ fanIndex }) => {
            if (failure)
              return {
                fanIndex,
                outcome: {
                  status: "unattempted",
                  error:
                    unavailable ?? "An earlier fan failed. Retry the curve.",
                },
              };
            const error = fanFailures[fanIndex];
            if (error) {
              failure = `Fan ${fanIndex + 1}: ${error}`;
              return { fanIndex, outcome: { status: "failed", error } };
            }
            return { fanIndex, outcome: succeeded };
          },
        );
        if (failure) {
          suspendCurve(
            gpu,
            `After the Prism daemon restarted, the curve couldn't resume. ${failure}`,
          );
          control.recovery = { status: "retryRequired", fans };
          continue;
        }
        control.revision += 1;
        for (const request of control.requested) {
          const setting = settings.get(request.fanIndex);
          if (setting?.mode !== "curve") continue;
          request.speedPercent = requestedSpeed(setting.points, temperature!);
          Object.assign(
            gpu.cooling.fans.find((fan) => fan.index === request.fanIndex)!,
            { mode: "manual", currentSpeedPercent: request.speedPercent },
          );
        }
        control.curveTemperatureC = temperature;
        control.recovery = { status: "resumed", fans };
      }
    },
    /** Suspends the first GPU's curve after a fault, as the daemon reports it. */
    suspendCurve(reason: string, restoration?: FanRestoration[]) {
      suspendCurve(response.gpus[0], reason, restoration);
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
