import { useQuery } from "@tanstack/react-query";

import { getDriverStatus } from "@/features/drivers/api";

export function useDriverStatus() {
  return useQuery({
    queryKey: ["driver-status"],
    queryFn: getDriverStatus,
    refetchInterval: (query) => {
      const job = query.state.data?.activeJob;
      return job?.status === "queued" || job?.status === "running"
        ? 2_000
        : false;
    },
  });
}
