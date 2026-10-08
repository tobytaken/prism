import type {
  AutomaticSwitchingStatus,
  RunningGamesState,
} from "@/features/games/types";
import type {
  GpuDaemonStatus,
  GpuTuningState,
  TuningStoreHealth,
} from "@/features/gpus/types";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasKeys(value: Record<string, unknown>, keys: string[]) {
  return (
    Object.keys(value).length === keys.length &&
    keys.every((key) => key in value)
  );
}

function isStoreHealth(value: unknown): value is TuningStoreHealth {
  if (!isRecord(value)) return false;
  if (value.status === "healthy") return hasKeys(value, ["status"]);
  return (
    value.status === "unhealthy" &&
    hasKeys(value, ["status", "problem", "path", "detail"]) &&
    (value.problem === "unreadable" || value.problem === "newerVersion") &&
    typeof value.path === "string" &&
    typeof value.detail === "string"
  );
}

export function isDaemonStatus(value: unknown): value is GpuDaemonStatus {
  return (
    isRecord(value) &&
    hasKeys(value, [
      "reachable",
      "managedByDaemon",
      "socketPath",
      "message",
      "tuningStore",
    ]) &&
    typeof value.reachable === "boolean" &&
    typeof value.managedByDaemon === "boolean" &&
    typeof value.socketPath === "string" &&
    (value.message === null || typeof value.message === "string") &&
    (value.tuningStore === null || isStoreHealth(value.tuningStore))
  );
}

export function daemonStatusNotification(
  value: unknown,
): GpuDaemonStatus | undefined {
  if (
    isRecord(value) &&
    hasKeys(value, ["method", "params"]) &&
    value.method === "daemon.statusChanged" &&
    isDaemonStatus(value.params)
  ) {
    return value.params;
  }
}

export function isTuningState(value: unknown): value is GpuTuningState {
  if (
    !isRecord(value) ||
    !hasKeys(value, ["gpuId", "revision", "reported", "currentProfile"]) ||
    typeof value.gpuId !== "string" ||
    typeof value.revision !== "string" ||
    !isRecord(value.reported) ||
    !hasKeys(value.reported, [
      "powerLimitWatts",
      "graphicsOffsetMHz",
      "memoryOffsetMHz",
    ]) ||
    !Object.values(value.reported).every(
      (reading) => reading === null || Number.isSafeInteger(reading),
    )
  )
    return false;
  const current = value.currentProfile;
  return (
    current === null ||
    (isRecord(current) &&
      hasKeys(current, ["id", "name"]) &&
      typeof current.id === "string" &&
      typeof current.name === "string")
  );
}

export function tuningStateNotification(
  value: unknown,
): GpuTuningState | undefined {
  if (
    isRecord(value) &&
    hasKeys(value, ["method", "params"]) &&
    value.method === "gpu.tuningStateChanged" &&
    isTuningState(value.params)
  ) {
    return value.params;
  }
}

export function isRunningGamesState(
  value: unknown,
): value is RunningGamesState {
  if (!isRecord(value)) return false;
  if (value.status === "unavailable")
    return (
      hasKeys(value, ["status", "reason"]) && typeof value.reason === "string"
    );
  return (
    value.status === "available" &&
    hasKeys(value, ["status", "games"]) &&
    Array.isArray(value.games) &&
    value.games.every(
      (entry: unknown) =>
        isRecord(entry) &&
        hasKeys(entry, ["game", "pid", "startTimeTicks"]) &&
        isSteamGame(entry.game) &&
        positiveU32(entry.pid) &&
        Number.isSafeInteger(entry.startTimeTicks) &&
        typeof entry.startTimeTicks === "number" &&
        entry.startTimeTicks >= 0,
    )
  );
}

function positiveU32(value: unknown): value is number {
  return (
    typeof value === "number" &&
    Number.isInteger(value) &&
    value > 0 &&
    value <= 0xffffffff
  );
}

export function runningGamesNotification(
  value: unknown,
): RunningGamesState | undefined {
  if (
    isRecord(value) &&
    hasKeys(value, ["method", "params"]) &&
    value.method === "games.runningChanged" &&
    isRunningGamesState(value.params)
  )
    return value.params;
}

function isSteamGame(value: unknown) {
  return (
    isRecord(value) &&
    hasKeys(value, ["kind", "appId"]) &&
    value.kind === "steam" &&
    positiveU32(value.appId)
  );
}

function isSwitchingCondition(value: unknown) {
  if (!isRecord(value)) return false;
  if (value.status === "ready" || value.status === "off")
    return hasKeys(value, ["status"]);
  return (
    (value.status === "blocked" || value.status === "recognitionUnavailable") &&
    hasKeys(value, ["status", "reason"]) &&
    typeof value.reason === "string"
  );
}

function isGpuSwitchingState(value: unknown) {
  if (!isRecord(value)) return false;
  switch (value.status) {
    case "active":
      return (
        hasKeys(value, ["status", "game", "gameName", "profile"]) &&
        isSteamGame(value.game) &&
        typeof value.gameName === "string" &&
        isRecord(value.profile) &&
        hasKeys(value.profile, ["id", "name"]) &&
        typeof value.profile.id === "string" &&
        typeof value.profile.name === "string"
      );
    case "waitingForEdits":
      return (
        hasKeys(value, ["status", "profile"]) &&
        isRecord(value.profile) &&
        hasKeys(value.profile, ["id", "name"]) &&
        typeof value.profile.id === "string" &&
        typeof value.profile.name === "string"
      );
    case "anotherUser":
      return hasKeys(value, ["status"]);
    case "failed":
    case "recoveryRequired":
      return (
        hasKeys(value, ["status", "message"]) &&
        typeof value.message === "string"
      );
    default:
      return false;
  }
}

export function isAutomaticSwitchingStatus(
  value: unknown,
): value is AutomaticSwitchingStatus {
  return (
    isRecord(value) &&
    hasKeys(value, ["condition", "gpus"]) &&
    isSwitchingCondition(value.condition) &&
    Array.isArray(value.gpus) &&
    value.gpus.every(
      (entry: unknown) =>
        isRecord(entry) &&
        hasKeys(entry, ["gpuId", "state"]) &&
        typeof entry.gpuId === "string" &&
        isGpuSwitchingState(entry.state),
    )
  );
}

export function switchingStatusNotification(
  value: unknown,
): AutomaticSwitchingStatus | undefined {
  if (
    isRecord(value) &&
    hasKeys(value, ["method", "params"]) &&
    value.method === "games.switchingStatusChanged" &&
    isAutomaticSwitchingStatus(value.params)
  )
    return value.params;
}
