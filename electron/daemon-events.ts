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
    !hasKeys(value, ["gpuId", "revision", "currentProfile"]) ||
    typeof value.gpuId !== "string" ||
    typeof value.revision !== "string"
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
