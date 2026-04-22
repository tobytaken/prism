import { useParams } from "react-router-dom";

import { useGpuInventory } from "@/features/gpus/hooks/use-gpu-inventory";

import { GpuDetailScreen } from "./gpu-detail-screen";

export function GpuDetailPage() {
  const { gpuId } = useParams();
  const inventoryQuery = useGpuInventory();
  const gpu = inventoryQuery.data?.gpus.find((candidate) => candidate.id === gpuId);

  return (
    <GpuDetailScreen
      gpu={gpu}
      isLoading={inventoryQuery.isPending}
      errorMessage={inventoryQuery.error instanceof Error ? inventoryQuery.error.message : null}
    />
  );
}
