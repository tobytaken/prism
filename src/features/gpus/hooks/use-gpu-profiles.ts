import { useEffect } from "react";
import { useMigration } from "./use-migration";
import { useStartup } from "./use-startup";
import { useGames } from "@/features/games/hooks/use-games";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  getGpuTuningStates,
  listGpuProfiles,
  saveGpuProfile,
  saveGpuProfileToCurrent,
  renameGpuProfile,
  deleteGpuProfile,
} from "@/features/gpus/api";
import { PROFILE_SETTING_KEYS } from "@/features/gpus/lib/tuning-profiles";
import type {
  GpuPerformanceDevice,
  GpuTuningState,
} from "@/features/gpus/types";

export const PROFILES_QUERY_KEY = ["gpu-profiles"];
export const TUNING_STATES_QUERY_KEY = ["gpu-tuning-states"];

/** The caller's profiles, and each GPU's current profile and state revision from the daemon. */
export function useGpuProfiles() {
  const queryClient = useQueryClient();
  const startup = useStartup();
  const migration = useMigration();
  const games = useGames();
  const query = useQuery({
    queryKey: PROFILES_QUERY_KEY,
    queryFn: listGpuProfiles,
  });
  // Polled at the readings' pace and updated by pushes, so it keeps up with the values on screen.
  const states = useQuery({
    queryKey: TUNING_STATES_QUERY_KEY,
    queryFn: getGpuTuningStates,
    refetchInterval: 500,
    refetchIntervalInBackground: true,
  });
  useEffect(
    () =>
      window.prism.gpu.onTuningStateChanged((state) => {
        queryClient.setQueryData<GpuTuningState[]>(
          TUNING_STATES_QUERY_KEY,
          (current) => [
            ...(current ?? []).filter(({ gpuId }) => gpuId !== state.gpuId),
            state,
          ],
        );
      }),
    [queryClient],
  );
  const refresh = () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: PROFILES_QUERY_KEY }),
      queryClient.invalidateQueries({ queryKey: TUNING_STATES_QUERY_KEY }),
      startup.refresh(),
      games.refresh(),
    ]);
  // The revision a save carries must cover the values on screen. A pushed or polled state can
  // be newer than the readings shown, and its revision would let unseen values be saved.
  const revisionFor = (gpu: GpuPerformanceDevice) => {
    const state = states.data?.find(({ gpuId }) => gpuId === gpu.stableId);
    if (!state)
      throw new Error("The GPU's profile state is unavailable. Try again.");
    const shown = {
      powerLimitWatts: gpu.power.currentLimitWatts,
      graphicsOffsetMHz: gpu.clocks.graphics.currentOffsetMHz,
      memoryOffsetMHz: gpu.clocks.memory.currentOffsetMHz,
    };
    if (PROFILE_SETTING_KEYS.some((key) => state.reported[key] !== shown[key]))
      throw new Error(STALE_SAVE);
    return { gpuId: state.gpuId, revision: state.revision };
  };
  // A rejected save may be stale, so it refreshes what the app shows as well.
  const saveAsNew = useMutation({
    mutationFn: ({ gpu, name }: { gpu: GpuPerformanceDevice; name: string }) =>
      saveGpuProfile({ ...revisionFor(gpu), name }),
    onSettled: refresh,
  });
  const saveToCurrent = useMutation({
    mutationFn: ({ gpu }: { gpu: GpuPerformanceDevice }) =>
      saveGpuProfileToCurrent(revisionFor(gpu)),
    onSettled: refresh,
  });
  const rename = useMutation({
    mutationFn: renameGpuProfile,
    onSettled: refresh,
  });
  const remove = useMutation({
    mutationFn: deleteGpuProfile,
    onSettled: refresh,
  });
  return {
    migration,
    startup,
    games,
    query,
    states,
    saveAsNew,
    saveToCurrent,
    rename,
    remove,
    stateOf(gpuId: string) {
      return states.data?.find((state) => state.gpuId === gpuId);
    },
    /** Reloads profiles and tuning states after the daemon changed them, such as by applying. */
    refresh,
  };
}

export type GpuProfileWorkflow = ReturnType<typeof useGpuProfiles>;

/** The daemon's rejection of a stale save, also given when the app sees one itself. */
const STALE_SAVE =
  "The GPU's values changed since you last looked. Review them and save again.";
