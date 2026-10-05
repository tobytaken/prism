import { useMutation, useQueryClient } from "@tanstack/react-query";

import { applyGpuConfig } from "@/features/gpus/api";
import {
  mutationErrorMessage,
  runTuningMutation,
  type TuningMutationResult,
} from "@/features/gpus/lib/tuning-mutation";
import { useGpuDaemonStatus } from "@/features/gpus/hooks/use-gpu-daemon-status";
import { useGpuPerformance } from "@/features/gpus/hooks/use-gpu-performance";
import {
  TUNING_STATES_QUERY_KEY,
  useGpuProfiles,
} from "@/features/gpus/hooks/use-gpu-profiles";

import {
  GpuTuningScreen,
  type PerformanceMutationAction,
} from "./gpu-tuning-screen";

export function GpuTuningPage() {
  const queryClient = useQueryClient();
  const profiles = useGpuProfiles();
  const query = useGpuPerformance();
  const daemonStatusQuery = useGpuDaemonStatus();
  const mutation = useMutation<
    TuningMutationResult,
    Error,
    PerformanceMutationAction
  >({
    mutationFn: runTuningMutation,
    // A write can change the GPU's current profile and the revision a save must carry.
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["gpu-performance"] }),
        queryClient.invalidateQueries({ queryKey: TUNING_STATES_QUERY_KEY }),
      ]);
    },
  });
  const reapplyMutation = useMutation({
    mutationFn: () => applyGpuConfig({ reapplySaved: true }),
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["gpu-performance"] }),
        queryClient.invalidateQueries({ queryKey: ["gpu-daemon-status"] }),
        queryClient.invalidateQueries({ queryKey: ["gpu-inventory"] }),
      ]);
    },
  });

  return (
    <GpuTuningScreen
      profiles={profiles}
      inventory={query.data}
      daemonStatus={daemonStatusQuery.data}
      isLoading={query.isPending}
      errorMessage={query.error instanceof Error ? query.error.message : null}
      mutationErrorMessage={mutationErrorMessage(
        mutation.error,
        mutation.variables,
      )}
      reapplyPending={reapplyMutation.isPending}
      reapplyErrorMessage={
        reapplyMutation.error instanceof Error
          ? reapplyMutation.error.message
          : (reapplyMutation.data?.error?.message ?? null)
      }
      pendingAction={mutation.isPending ? mutation.variables : null}
      onReapplySavedSettings={() => {
        reapplyMutation.mutate();
      }}
      onMutationAction={(action) => mutation.mutateAsync(action)}
    />
  );
}
