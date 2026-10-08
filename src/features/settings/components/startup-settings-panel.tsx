import { useId } from "react";

import { Separator } from "@/components/ui/separator";
import { useGpuPerformance } from "@/features/gpus/hooks/use-gpu-performance";
import { useStartup } from "@/features/gpus/hooks/use-startup";

export function StartupSettingsPanel() {
  const id = useId();
  const startup = useStartup();
  const performance = useGpuPerformance();
  const behavior = startup.query.data?.behavior;
  const unconfigured =
    performance.data?.gpus.filter(
      (gpu) => !gpu.stableId || !startup.profileOf(gpu.stableId),
    ) ?? [];
  const possiblyStale = startup.query.data?.possiblyStale ?? [];
  const gpuLabel = (gpuId: string) =>
    `${performance.data?.gpus.find((gpu) => gpu.stableId === gpuId)?.name ?? "GPU"} (${gpuId})`;
  return (
    <section className="max-w-2xl space-y-6" aria-label="Startup settings">
      <div>
        <h2 className="text-xl font-semibold tracking-tight">
          Startup behavior
        </h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Choose what Prism applies at host boot, before login. This setting is
          shared by everyone on this host.
        </p>
      </div>
      <Separator />
      {startup.query.isPending && (
        <p role="status">Reading startup behavior…</p>
      )}
      {startup.query.error && (
        <p role="alert" className="text-sm text-destructive">
          {startup.query.error.message}
        </p>
      )}
      <fieldset
        disabled={!startup.query.isSuccess || startup.setBehavior.isPending}
        className="space-y-3"
      >
        <legend className="sr-only">Startup behavior</legend>
        {(
          [
            ["off", "Off", null],
            ["startupProfiles", "Startup profiles", null],
            [
              "restoreLastApplied",
              "Restore last applied",
              "Restores each setting's most recent manual value, including cooling configurations and changes not saved to a profile. An unstable value could be restored on every boot, so after an unclean shutdown that followed a restore, Prism skips the restore once.",
            ],
          ] as const
        ).map(([value, label, warning]) => (
          <label
            key={value}
            htmlFor={`${id}-${value}`}
            className="flex items-start gap-3 rounded-lg border p-3 text-sm"
          >
            <input
              id={`${id}-${value}`}
              type="radio"
              name={`${id}-behavior`}
              value={value}
              checked={behavior === value}
              aria-describedby={warning ? `${id}-${value}-warning` : undefined}
              onChange={() => startup.setBehavior.mutate(value)}
            />
            <span className="space-y-1">
              <span className="block">{label}</span>
              {warning && (
                <span
                  id={`${id}-${value}-warning`}
                  className="block text-amber-600 dark:text-amber-400"
                >
                  {warning}
                </span>
              )}
            </span>
          </label>
        ))}
      </fieldset>
      {startup.setBehavior.error && (
        <p role="alert" className="text-sm text-destructive">
          {startup.setBehavior.error.message}
        </p>
      )}
      <p className="text-sm text-muted-foreground">
        Changes take effect at the next host boot. Choose each GPU's startup
        profile in Manage profiles on Performance.
      </p>
      {behavior === "off" && (
        <p className="text-sm text-muted-foreground">
          Existing saved startup settings continue to apply while Off.
        </p>
      )}
      {behavior === "restoreLastApplied" && possiblyStale.length > 0 && (
        <div
          role="status"
          aria-label="Possibly stale"
          className="space-y-2 rounded-lg border border-amber-500/50 p-3 text-sm"
        >
          <p className="font-medium">Restore last applied is possibly stale</p>
          <ul className="space-y-1">
            {possiblyStale.map((gpuId) => (
              <li key={gpuId}>
                {gpuLabel(gpuId)} — a recent change was not recorded for
                startup. The next boot may restore older values until a later
                change is recorded.
              </li>
            ))}
          </ul>
        </div>
      )}
      {behavior === "startupProfiles" && (
        <div className="space-y-2 text-sm">
          {performance.isPending && <p role="status">Reading GPUs…</p>}
          {performance.error && (
            <p role="alert">
              The GPU list could not be read: {performance.error.message}
            </p>
          )}
          {unconfigured.length > 0 && (
            <ul className="space-y-2">
              {unconfigured.map((gpu) => (
                <li key={gpu.id}>
                  {gpu.name} ({gpu.stableId ?? gpu.id}) — nothing applied at
                  startup
                  {!gpu.stableId &&
                    ". Profile features need a stable identity (UUID)."}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </section>
  );
}
