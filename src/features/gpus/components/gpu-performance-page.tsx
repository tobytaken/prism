import { useMutation, useQueryClient } from "@tanstack/react-query";

import {
  mutationErrorMessage,
  runTuningMutation,
  type TuningMutationResult,
} from "@/features/gpus/lib/tuning-mutation";
import { useGpuDaemonStatus } from "@/features/gpus/hooks/use-gpu-daemon-status";
import { useGpuPerformance } from "@/features/gpus/hooks/use-gpu-performance";
import { useGpuProfiles } from "@/features/gpus/hooks/use-gpu-profiles";

import { GpuPerformanceScreen } from "./gpu-performance-screen";
import type { PerformanceMutationAction } from "./gpu-tuning-screen";

export function GpuPerformancePage() {
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
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: ["gpu-performance"] });
    },
  });

  return (
    <GpuPerformanceScreen
      profiles={profiles}
      inventory={query.data}
      daemonStatus={daemonStatusQuery.data}
      isLoading={query.isPending}
      errorMessage={query.error instanceof Error ? query.error.message : null}
      mutationErrorMessage={mutationErrorMessage(
        mutation.error,
        mutation.variables,
      )}
      pendingAction={mutation.isPending ? mutation.variables : null}
      onMutationAction={(action) => mutation.mutateAsync(action)}
    />
  );
}
