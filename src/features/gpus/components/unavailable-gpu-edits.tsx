import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import type { TuningEditStore } from "@/features/gpus/hooks/use-tuning-edits";
import type { GpuPerformanceDevice } from "@/features/gpus/types";

export function UnavailableGpuEdits({
  devices,
  gpus,
}: {
  devices: TuningEditStore["devices"];
  gpus: readonly Pick<GpuPerformanceDevice, "id">[];
}) {
  return Object.entries(devices)
    .filter(
      ([gpuId, device]) =>
        Object.keys(device.edits).length > 0 &&
        !gpus.some((gpu) => gpu.id === gpuId),
    )
    .map(([gpuId]) => (
      <Alert key={gpuId}>
        <AlertTitle>Unapplied changes</AlertTitle>
        <AlertDescription>
          Edits for {gpuId} are retained while this GPU is unavailable.
        </AlertDescription>
      </Alert>
    ));
}
