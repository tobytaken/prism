import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import type {
  StartupBehavior,
  SetStartupProfileRequest,
} from "@/features/gpus/types";

export const STARTUP_QUERY_KEY = ["gpu-startup"];

/** Host-wide choices and the caller's undismissed outcomes from this boot. */
export function useStartup() {
  const client = useQueryClient();
  const query = useQuery({
    queryKey: STARTUP_QUERY_KEY,
    queryFn: () => window.prism.gpu.getStartupState(),
    refetchInterval: 1000,
    refetchIntervalInBackground: true,
  });
  const refresh = () =>
    client.invalidateQueries({ queryKey: STARTUP_QUERY_KEY });
  const setBehavior = useMutation({
    mutationFn: async (behavior: StartupBehavior) => {
      const result = await window.prism.gpu.setStartupBehavior({ behavior });
      if (!result.success)
        throw new Error(result.error ?? "Failed to change startup behavior.");
    },
    onSettled: refresh,
  });
  const setProfile = useMutation({
    mutationFn: async (request: SetStartupProfileRequest) => {
      const result = await window.prism.gpu.setStartupProfile(request);
      if (!result.success)
        throw new Error(
          result.error ?? "Failed to change the startup profile.",
        );
    },
    onSettled: refresh,
  });
  const dismiss = useMutation({
    mutationFn: async (gpuId: string) => {
      const result = await window.prism.gpu.dismissStartupOutcome({ gpuId });
      if (!result.success)
        throw new Error(
          result.error ?? "Failed to dismiss the startup outcome.",
        );
    },
    onSettled: refresh,
  });
  return {
    query,
    setBehavior,
    setProfile,
    dismiss,
    refresh,
    profileOf: (gpuId: string) =>
      query.data?.profiles.find((item) => item.gpuId === gpuId)?.profile ??
      null,
  };
}
