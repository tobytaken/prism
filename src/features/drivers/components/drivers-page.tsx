import { useMutation, useQueryClient } from "@tanstack/react-query";

import {
  cancelDriverJob,
  requestHostReboot,
  startDriverAction,
} from "@/features/drivers/api";
import { useDriverStatus } from "@/features/drivers/hooks/use-driver-status";
import type { DriverAction } from "@/features/drivers/types";

import { DriversScreen } from "./drivers-screen";

export function DriversPage() {
  const queryClient = useQueryClient();
  const statusQuery = useDriverStatus();

  const refreshStatus = async () => {
    await queryClient.invalidateQueries({ queryKey: ["driver-status"] });
  };

  const startMutation = useMutation({
    mutationFn: (action: DriverAction) => startDriverAction(action),
    onSuccess: refreshStatus,
  });
  const cancelMutation = useMutation({
    mutationFn: cancelDriverJob,
    onSuccess: refreshStatus,
  });
  const rebootMutation = useMutation({
    mutationFn: requestHostReboot,
  });

  return (
    <DriversScreen
      status={statusQuery.data}
      isLoading={statusQuery.isPending}
      errorMessage={
        statusQuery.error instanceof Error ? statusQuery.error.message : null
      }
      isStarting={startMutation.isPending}
      isCanceling={cancelMutation.isPending}
      isRebooting={rebootMutation.isPending}
      actionError={formatMutationError(
        startMutation.error ?? cancelMutation.error ?? rebootMutation.error,
      )}
      onStartAction={(action) => startMutation.mutate(action)}
      onCancelJob={(jobId) => cancelMutation.mutate(jobId)}
      onRequestReboot={() => rebootMutation.mutate()}
    />
  );
}

function formatMutationError(error: unknown) {
  return error instanceof Error ? error.message : null;
}
