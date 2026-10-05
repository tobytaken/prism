import { useEffect } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  getGpuTuningStates,
  listGpuProfiles,
  saveGpuProfile,
  saveGpuProfileToCurrent,
} from "@/features/gpus/api";
import type { GpuTuningState } from "@/features/gpus/types";

const PROFILES = ["gpu-profiles"];
const TUNING_STATES = ["gpu-tuning-states"];

/** The caller's profiles, and each GPU's current profile and state revision from the daemon. */
export function useGpuProfiles() {
  const queryClient = useQueryClient();
  const query = useQuery({ queryKey: PROFILES, queryFn: listGpuProfiles });
  // Polled with the readings, so a save carries the revision of the values on screen.
  const states = useQuery({
    queryKey: TUNING_STATES,
    queryFn: getGpuTuningStates,
    refetchInterval: 500,
    refetchIntervalInBackground: true,
  });
  useEffect(
    () =>
      window.prism.gpu.onTuningStateChanged((state) => {
        queryClient.setQueryData<GpuTuningState[]>(TUNING_STATES, (current) => [
          ...(current ?? []).filter(({ gpuId }) => gpuId !== state.gpuId),
          state,
        ]);
      }),
    [queryClient],
  );
  const refresh = () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: PROFILES }),
      queryClient.invalidateQueries({ queryKey: TUNING_STATES }),
    ]);
  const revisionOf = (gpuId: string) => {
    const revision = states.data?.find(
      (state) => state.gpuId === gpuId,
    )?.revision;
    if (revision == null)
      throw new Error("The GPU's profile state is unavailable. Try again.");
    return revision;
  };
  // A rejected save may be stale, so it refreshes what the app shows as well.
  const saveAsNew = useMutation({
    mutationFn: ({ gpuId, name }: { gpuId: string; name: string }) =>
      saveGpuProfile({ gpuId, name, revision: revisionOf(gpuId) }),
    onSettled: refresh,
  });
  const saveToCurrent = useMutation({
    mutationFn: ({ gpuId }: { gpuId: string }) =>
      saveGpuProfileToCurrent({ gpuId, revision: revisionOf(gpuId) }),
    onSettled: refresh,
  });
  return {
    query,
    states,
    saveAsNew,
    saveToCurrent,
    stateOf(gpuId: string) {
      return states.data?.find((state) => state.gpuId === gpuId);
    },
    /** Reloads profiles and tuning states after the daemon changed them, such as by applying. */
    refresh,
  };
}

export type GpuProfileWorkflow = ReturnType<typeof useGpuProfiles>;
