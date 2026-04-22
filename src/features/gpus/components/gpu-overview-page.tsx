import { useGpuInventory } from "@/features/gpus/hooks/use-gpu-inventory";

import { GpuOverviewScreen } from "./gpu-overview-screen";

export function GpuOverviewPage() {
  const inventoryQuery = useGpuInventory();

  return (
    <GpuOverviewScreen
      inventory={inventoryQuery.data}
      isLoading={inventoryQuery.isPending}
      errorMessage={inventoryQuery.error instanceof Error ? inventoryQuery.error.message : null}
    />
  );
}
