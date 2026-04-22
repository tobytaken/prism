import { useQuery } from "@tanstack/react-query";

import { getGpuPerformance } from "@/features/gpus/api";

export function useGpuPerformance() {
  return useQuery({
    queryKey: ["gpu-performance"],
    queryFn: getGpuPerformance,
    refetchInterval: 500,
    refetchIntervalInBackground: true,
  });
}
