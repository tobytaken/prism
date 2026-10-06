import { Button } from "@/components/ui/button";
import type { GpuProfileWorkflow } from "@/features/gpus/hooks/use-gpu-profiles";
import type { GpuPerformanceDevice } from "@/features/gpus/types";
import { TuningOutcomes } from "./tuning-outcomes";

export function StartupOutcomes({
  workflow,
  gpus,
}: {
  workflow: GpuProfileWorkflow;
  gpus: GpuPerformanceDevice[];
}) {
  const { query, dismiss } = workflow.startup;
  if (query.error)
    return (
      <p role="alert" className="text-sm text-destructive">
        Startup outcomes could not be read: {query.error.message}
      </p>
    );
  if (!query.data?.outcomes.length) return null;
  return (
    <section aria-label="Startup outcomes" className="space-y-3">
      <h2 className="text-lg font-semibold">Startup outcomes</h2>
      {query.data.outcomes.map((outcome) => {
        const gpu = gpus.find((gpu) => gpu.stableId === outcome.gpuId);
        const label = `${gpu?.name ?? "GPU"} (${outcome.gpuId})`;
        const application = outcome.application;
        return (
          <div
            key={outcome.gpuId}
            className="flex items-start justify-between gap-4 rounded-lg border p-4 text-sm"
          >
            <div className="space-y-1">
              <h3 className="font-medium">{label}</h3>
              <p>
                {outcome.profile?.name ?? "Startup"} —{" "}
                {outcome.status === "applied"
                  ? "Applied"
                  : outcome.status === "failed"
                    ? "Failed"
                    : "Not attempted"}
              </p>
              {outcome.reason && (
                <p className="text-muted-foreground whitespace-pre-line">
                  {outcome.reason}
                </p>
              )}
              {application && (
                <TuningOutcomes
                  outcomes={{
                    power: application.power ?? undefined,
                    graphics: application.graphics ?? undefined,
                    memory: application.memory ?? undefined,
                  }}
                />
              )}
              {outcome.profile && outcome.status !== "applied" && (
                <p className="text-muted-foreground">
                  You can apply the profile manually from the selector and
                  review any range corrections.
                </p>
              )}
            </div>
            <Button
              type="button"
              variant="outline"
              size="sm"
              aria-label={`Dismiss startup outcome for ${label}`}
              disabled={dismiss.isPending}
              onClick={() => dismiss.mutate(outcome.gpuId)}
            >
              Dismiss
            </Button>
          </div>
        );
      })}
      {dismiss.error && (
        <p role="alert" className="text-sm text-destructive">
          {dismiss.error.message}
        </p>
      )}
    </section>
  );
}
