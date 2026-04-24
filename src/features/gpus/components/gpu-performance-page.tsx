import { useMutation, useQueryClient } from "@tanstack/react-query";

import {
  applyGpuConfig,
  resetGpuClockOffsets,
  resetGpuPowerLimit,
  setGpuClockOffsets,
  setGpuFanModeAuto,
  setGpuFanSpeed,
  setGpuPowerLimit,
} from "@/features/gpus/api";
import { useGpuDaemonStatus } from "@/features/gpus/hooks/use-gpu-daemon-status";
import { useGpuPerformance } from "@/features/gpus/hooks/use-gpu-performance";

import { GpuPerformanceScreen } from "./gpu-performance-screen";
import type { PerformanceMutationAction } from "./gpu-tuning-screen";

export function GpuPerformancePage() {
  const queryClient = useQueryClient();
  const query = useGpuPerformance();
  const daemonStatusQuery = useGpuDaemonStatus();
  const mutation = useMutation<void, Error, PerformanceMutationAction>({
    mutationFn: runPerformanceMutation,
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["gpu-performance"] });
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
    <GpuPerformanceScreen
      inventory={query.data}
      daemonStatus={daemonStatusQuery.data}
      isLoading={query.isPending}
      errorMessage={query.error instanceof Error ? query.error.message : null}
      mutationErrorMessage={mutation.error?.message ?? null}
      reapplyPending={reapplyMutation.isPending}
      reapplyErrorMessage={
        reapplyMutation.error instanceof Error
          ? reapplyMutation.error.message
          : reapplyMutation.data?.error?.message ?? null
      }
      pendingAction={mutation.isPending ? mutation.variables : null}
      onReapplySavedSettings={() => {
        reapplyMutation.mutate();
      }}
      onMutationAction={(action) => {
        mutation.mutate(action);
      }}
    />
  );
}

async function runPerformanceMutation(action: PerformanceMutationAction) {
  try {
    switch (action.kind) {
      case "setPowerLimit":
        await setGpuPowerLimit(action.gpuId, action.limitWatts);
        return;
      case "resetPowerLimit":
        await resetGpuPowerLimit(action.gpuId);
        return;
      case "setFanSpeed":
        await setGpuFanSpeed(action.gpuId, action.fanIndex, action.speedPercent);
        return;
      case "setFanModeAuto":
        await setGpuFanModeAuto(action.gpuId, action.fanIndex);
        return;
      case "setClockOffsets":
        await setGpuClockOffsets(
          action.gpuId,
          action.graphicsOffsetMHz,
          action.memoryOffsetMHz,
        );
        return;
      case "resetClockOffsets":
        await resetGpuClockOffsets(action.gpuId);
        return;
    }
  } catch (error) {
    throw error instanceof Error ? error : new Error(String(error));
  }
}
