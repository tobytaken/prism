import { Button } from "@/components/ui/button";
import type {
  CoolingConfiguration,
  CoolingControl,
  CoolingRecovery,
  FanRestoration,
  FanSetting,
  GpuPerformanceDevice,
} from "@/features/gpus/types";

/**
 * What Prism applied to the GPU's fans and how its curve control is doing. The applied
 * configuration, the speed a curve requests now and the driver's reported fan modes are kept
 * apart: a curve runs through the driver's manual mode without being a fixed speed.
 */
export function CoolingControlStatus({
  gpu,
  busy,
  onApply,
}: {
  gpu: GpuPerformanceDevice;
  busy: boolean;
  onApply: (configuration: CoolingConfiguration) => void;
}) {
  const control = gpu.cooling.control;
  if (!control || !gpu.cooling.isSupported || gpu.cooling.fans.length === 0)
    return null;
  const { applied, suspension, recovery } = control;
  const yielded = suspension?.kind === "externalControl";
  const automatic =
    applied?.arrangement === "shared" && applied.setting.mode === "automatic";

  return (
    <div
      role="group"
      aria-label="Cooling control"
      className="space-y-2 border-t border-border/70 pt-4 text-xs"
    >
      <p className="font-medium">
        {applied
          ? `Applied: ${describeCooling(applied)}`
          : "Prism hasn't applied cooling to this GPU."}
      </p>
      <RequestedSpeeds control={control} />
      {recovery?.status === "resumed" && (
        <p className="text-muted-foreground">
          After the Prism daemon restarted, it resumed this curve from the
          current temperature.
        </p>
      )}
      {control.requested.length > 0 && (
        <p className="text-muted-foreground">
          Prism can detect reported fan control mode changes. Other tools'
          manual speed changes may not be observable.
        </p>
      )}
      {suspension && (
        <div className="space-y-1 border-l border-destructive/70 bg-destructive/5 px-3 py-2">
          <p className="text-destructive">
            Curve control suspended: {suspension.reason}
          </p>
          {recovery?.status === "retryRequired" && (
            <RecoveryOutcomes recovery={recovery} />
          )}
          <ul className="space-y-0.5 text-muted-foreground">
            {suspension.restoration.map((fan) => (
              <li key={fan.fanIndex}>{restorationText(fan)}</li>
            ))}
          </ul>
          <p className="text-muted-foreground">
            {yielded
              ? "Prism kept the curve. Resume only when you want Prism to control cooling again."
              : "Prism kept the curve but won't follow it again until you retry."}
          </p>
          {applied && (
            <Button
              size="sm"
              disabled={busy || !gpu.stableId}
              onClick={() => onApply(applied)}
            >
              {yielded ? "Resume curve" : "Retry curve"}
            </Button>
          )}
        </div>
      )}
      {applied && !automatic && (
        <Button
          size="sm"
          variant="outline"
          disabled={busy || !gpu.stableId}
          onClick={() =>
            onApply({ arrangement: "shared", setting: { mode: "automatic" } })
          }
        >
          Use automatic cooling
        </Button>
      )}
    </div>
  );
}

/** Each interrupted curve fan's resumption by the restarted daemon. */
function RecoveryOutcomes({ recovery }: { recovery: CoolingRecovery }) {
  return (
    <>
      <p className="text-muted-foreground">
        The restarted daemon didn't resume this curve:
      </p>
      <ul className="space-y-0.5 text-muted-foreground">
        {recovery.fans.map(({ fanIndex, outcome }) => (
          <li key={fanIndex}>
            {`Fan ${fanIndex + 1}: `}
            {outcome.status === "succeeded"
              ? "set, then stopped with the other fans"
              : outcome.status === "failed"
                ? `not resumed${outcome.error ? ` — ${outcome.error}` : ""}`
                : "not attempted"}
          </li>
        ))}
      </ul>
    </>
  );
}

function RequestedSpeeds({ control }: { control: CoolingControl }) {
  const { requested, curveTemperatureC } = control;
  if (requested.length === 0) return null;
  const at = curveTemperatureC == null ? "" : ` at ${curveTemperatureC}°C`;
  const speeds = new Set(requested.map((fan) => fan.speedPercent));
  return (
    <p className="text-muted-foreground">
      {speeds.size === 1
        ? `The curve requests ${requested[0].speedPercent}%${at}.`
        : `The curve requests ${requested
            .map((fan) => `${fan.speedPercent}% for fan ${fan.fanIndex + 1}`)
            .join(", ")}${at}.`}
    </p>
  );
}

export function describeCooling(configuration: CoolingConfiguration) {
  if (configuration.arrangement === "shared")
    return `${settingText(configuration.setting)} for all fans`;
  return configuration.fans
    .map((fan) => `fan ${fan.fanIndex + 1} ${settingText(fan.setting)}`)
    .join(", ");
}

function settingText(setting: FanSetting) {
  return setting.mode === "automatic"
    ? "automatic cooling"
    : setting.mode === "fixed"
      ? `fixed ${setting.speedPercent}%`
      : "fan curve";
}

function restorationText(fan: FanRestoration) {
  const label = `Fan ${fan.fanIndex + 1}: automatic cooling`;
  const detail = fan.detail ? ` — ${fan.detail}` : "";
  return fan.status === "restored"
    ? `${label} restored${detail}`
    : fan.status === "unverified"
      ? `${label} not confirmed${detail}`
      : `${label} not restored${detail}`;
}
