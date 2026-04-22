import { useGpuPerformance } from "@/features/gpus/hooks/use-gpu-performance";

import { GpuPerformanceScreen } from "./gpu-performance-screen";

export function GpuPerformancePage() {
  const query = useGpuPerformance();

  return (
    <GpuPerformanceScreen
      inventory={query.data}
      isLoading={query.isPending}
      errorMessage={query.error instanceof Error ? query.error.message : null}
    />
  );
}
