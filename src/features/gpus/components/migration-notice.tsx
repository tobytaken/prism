import { Button } from "@/components/ui/button";
import type { GpuProfileWorkflow } from "@/features/gpus/hooks/use-gpu-profiles";

export function MigrationNotice({
  workflow,
}: {
  workflow: GpuProfileWorkflow;
}) {
  const { query, adopt, dismiss } = workflow.migration;
  if (query.error)
    return (
      <p role="alert">
        Previous startup settings could not be read: {query.error.message}
      </p>
    );
  const state = query.data;
  if (!state || (!state.notice && !(state.canAdopt && state.profiles.length)))
    return null;
  return (
    <section
      aria-label="Previous startup settings"
      className="space-y-3 rounded-lg border p-4 text-sm"
    >
      <h2 className="text-lg font-semibold">Previous startup settings</h2>
      {state.notice && (
        <>
          <p>
            Your previous power limits and clock offsets have been migrated to
            startup profiles. Saved clock locks are no longer restored.
          </p>
          {state.notice.dropped.length > 0 && (
            <>
              <p>These saved configurations could not be migrated:</p>
              <ul className="space-y-2">
                {state.notice.dropped.map((entry) => (
                  <li key={entry.key}>
                    <p className="font-medium">{entry.key}</p>
                    <p>{entry.reason}</p>
                    <dl>
                      {Object.entries(entry.values)
                        .filter(([, value]) => value != null)
                        .map(([key, value]) => (
                          <div key={key}>
                            <dt className="inline">{settingLabel(key)}: </dt>
                            <dd className="inline">{describeValue(value)}</dd>
                          </div>
                        ))}
                    </dl>
                  </li>
                ))}
              </ul>
            </>
          )}
          <Button
            variant="outline"
            size="sm"
            disabled={dismiss.isPending}
            onClick={() => dismiss.mutate()}
            aria-label="Dismiss migration notice"
          >
            Dismiss
          </Button>
        </>
      )}
      {state.canAdopt &&
        state.profiles.map((profile) => (
          <div key={profile.id} className="flex flex-wrap items-center gap-3">
            <span>
              {profile.name} · {profile.gpuId}
            </span>
            <Button
              variant="outline"
              size="sm"
              disabled={adopt.isPending}
              onClick={() => adopt.mutate(profile.id)}
              aria-label={`Add to my profiles (${profile.gpuId})`}
            >
              Add to my profiles
            </Button>
          </div>
        ))}
      {(adopt.error || dismiss.error) && (
        <p role="alert" className="text-destructive">
          {adopt.error?.message ?? dismiss.error?.message}
        </p>
      )}
    </section>
  );
}

function settingLabel(key: string) {
  return (
    (
      {
        powerLimitWatts: "Power limit (W)",
        graphicsOffsetMHz: "Graphics offset (MHz)",
        memoryOffsetMHz: "Memory offset (MHz)",
        gpuClockLock: "Graphics clock lock (MHz)",
        memoryClockLock: "Memory clock lock (MHz)",
      } as Record<string, string>
    )[key] ?? key
  );
}
function describeValue(value: unknown): string {
  if (
    typeof value === "object" &&
    value !== null &&
    "minMhz" in value &&
    "maxMhz" in value
  )
    return `${value.minMhz}–${value.maxMhz}`;
  return typeof value === "object" ? JSON.stringify(value) : String(value);
}
