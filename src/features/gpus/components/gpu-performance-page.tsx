import { useMutation, useQueryClient } from "@tanstack/react-query";

import {
  runTuningMutation,
  type TuningMutationResult,
} from "@/features/gpus/lib/tuning-mutation";
import { useGpuDaemonStatus } from "@/features/gpus/hooks/use-gpu-daemon-status";
import { useGpuPerformance } from "@/features/gpus/hooks/use-gpu-performance";

import { GpuPerformanceScreen } from "./gpu-performance-screen";
import type { PerformanceMutationAction } from "./gpu-tuning-screen";

export function GpuPerformancePage() {
  const queryClient = useQueryClient();
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
      inventory={query.data}
      daemonStatus={daemonStatusQuery.data}
      isLoading={query.isPending}
      errorMessage={query.error instanceof Error ? query.error.message : null}
      mutationErrorMessage={mutation.error?.message ?? null}
      pendingAction={mutation.isPending ? mutation.variables : null}
      onMutationAction={(action) => mutation.mutateAsync(action)}
    />
  );
}
