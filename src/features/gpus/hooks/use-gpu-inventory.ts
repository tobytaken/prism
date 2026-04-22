import { useQuery } from "@tanstack/react-query";

import { getGpuInventory } from "@/features/gpus/api";

export function useGpuInventory() {
  return useQuery({
    queryKey: ["gpu-inventory"],
    queryFn: getGpuInventory,
    refetchInterval: 1_000,
    refetchIntervalInBackground: true,
  });
}
