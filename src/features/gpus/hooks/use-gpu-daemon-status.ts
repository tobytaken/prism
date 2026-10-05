import { useEffect } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import { getGpuDaemonStatus } from "@/features/gpus/api";

export function useGpuDaemonStatus() {
  const queryClient = useQueryClient();
  useEffect(
    () =>
      window.prism.gpu.onDaemonStatusChanged((status) => {
        void queryClient.cancelQueries({ queryKey: ["gpu-daemon-status"] });
        queryClient.setQueryData(["gpu-daemon-status"], status);
      }),
    [queryClient],
  );
  return useQuery({
    queryKey: ["gpu-daemon-status"],
    queryFn: getGpuDaemonStatus,
  });
}
