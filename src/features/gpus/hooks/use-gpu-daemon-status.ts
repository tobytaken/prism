import { useQuery } from "@tanstack/react-query";

import { getGpuDaemonStatus } from "@/features/gpus/api";

export function useGpuDaemonStatus() {
  return useQuery({
    queryKey: ["gpu-daemon-status"],
    queryFn: getGpuDaemonStatus,
    refetchInterval: 2_000,
    refetchIntervalInBackground: true,
  });
}
