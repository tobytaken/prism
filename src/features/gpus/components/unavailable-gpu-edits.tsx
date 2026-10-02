import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import type {
  TuningEditStore,
  TuningSetting,
} from "@/features/gpus/hooks/use-tuning-edits";
import type { GpuPerformanceDevice } from "@/features/gpus/types";
import { TuningOutcomes, tuningSettingLabel } from "./tuning-outcomes";

export function UnavailableGpuEdits({
  devices,
  gpus,
  onDiscard,
  pending = false,
}: {
  devices: TuningEditStore["devices"];
  gpus: readonly Pick<GpuPerformanceDevice, "id">[];
  onDiscard?: (gpuId: string, setting: TuningSetting) => void;
  pending?: boolean;
}) {
  return Object.entries(devices)
    .filter(
      ([gpuId, device]) =>
        Object.keys(device.edits).length > 0 &&
        !gpus.some((gpu) => gpu.id === gpuId),
    )
    .map(([gpuId, device]) => (
      <Alert key={gpuId}>
        <AlertTitle>Unapplied changes</AlertTitle>
        <AlertDescription className="space-y-2" aria-live="polite">
          <p>Edits for {gpuId} are retained while this GPU is unavailable.</p>
          <TuningOutcomes outcomes={device.outcomes} />
          {onDiscard &&
            Object.keys(device.edits).map((key) => {
              const setting = key as TuningSetting;
              return (
                <Button
                  key={setting}
                  size="sm"
                  variant="outline"
                  disabled={pending}
                  onClick={() => onDiscard(gpuId, setting)}
                >
                  Discard {tuningSettingLabel(setting).toLowerCase()} change for{" "}
                  {gpuId}
                </Button>
              );
            })}
        </AlertDescription>
      </Alert>
    ));
}
