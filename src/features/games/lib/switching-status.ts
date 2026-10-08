import type { ShownSwitchingStatus } from "@/features/games/hooks/use-switching-status";

function gpuState(status: ShownSwitchingStatus | undefined, gpuId: string) {
  return status?.gpus.find((entry) => entry.gpuId === gpuId)?.state;
}

/** How a GPU's automatic switching status reads, and its header chip when it isn't ready. */
export function describeGpuSwitching(
  status: ShownSwitchingStatus | undefined,
  gpuId: string,
): { summary: string; chip: string | null } {
  if (!status) return { summary: "Reading status…", chip: null };
  const { condition } = status;
  const state = gpuState(status, gpuId);
  if (state?.status === "recoveryRequired")
    return {
      summary: `Recovery required: ${state.message}`,
      chip: "Recovery required",
    };
  if (condition.status === "unknown")
    return {
      summary: `Status unknown: ${condition.reason}`,
      chip: "Switching status unknown",
    };
  if (condition.status === "blocked")
    return {
      summary: `Blocked: ${condition.reason}`,
      chip: "Switching blocked",
    };
  if (state?.status === "failed")
    return { summary: `Failed: ${state.message}`, chip: "Switching failed" };
  if (condition.status === "recognitionUnavailable")
    return {
      summary:
        state?.status === "active"
          ? `Holding ${state.profile.name} for ${state.gameName}: recognition unavailable.`
          : "Holding current values: recognition unavailable.",
      chip: "Recognition unavailable",
    };
  if (state?.status === "waitingForEdits")
    return {
      summary: `Automatic profile "${state.profile.name}" is waiting for unapplied tuning changes.`,
      chip: "Waiting for edits",
    };
  if (state?.status === "active")
    return {
      summary: `${state.profile.name} applied for ${state.gameName}.`,
      chip: null,
    };
  if (state?.status === "anotherUser")
    return { summary: "Another user's game is running.", chip: null };
  return condition.status === "ready"
    ? { summary: "Waiting for a linked game to run.", chip: null }
    : { summary: "Automatic switching is off.", chip: null };
}

/** The tag for a current profile that a running game applied automatically. */
export function automaticTag(
  status: ShownSwitchingStatus | undefined,
  gpuId: string | null,
  currentProfileId: string | undefined,
): string | null {
  const state = gpuId ? gpuState(status, gpuId) : undefined;
  return state?.status === "active" && state.profile.id === currentProfileId
    ? `auto · ${state.gameName}`
    : null;
}
