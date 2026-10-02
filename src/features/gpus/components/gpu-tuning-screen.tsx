import { UnavailableGpuEdits } from "./unavailable-gpu-edits";
import { GpuProfilePanel, GpuProfileAvailability } from "./gpu-profile-panel";
import type { GpuProfileWorkflow } from "@/features/gpus/hooks/use-gpu-profiles";
import { TuningOutcomes, tuningSettingLabel } from "./tuning-outcomes";
import { inTuningRange } from "@/features/gpus/lib/tuning-intentions";
import { unavailableTuningReadback } from "@/features/gpus/lib/tuning-readback";
import type { TuningMutationHandler } from "@/features/gpus/lib/tuning-mutation";
import { type ReactNode } from "react";
import {
  AlertTriangle,
  Clock3,
  Cpu,
  Gauge,
  LoaderCircle,
  RefreshCw,
  RotateCcw,
  ShieldAlert,
  Wind,
  Zap,
} from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { VendorBadge } from "@/features/gpus/components/vendor-badge";
import {
  useTuningEdits,
  type TuningEditStore,
} from "@/features/gpus/hooks/use-tuning-edits";
import { cn } from "@/lib/utils";
import type {
  ClockOffsetInfo,
  CoolingInfo,
  ControlSupport,
  FanControlInfo,
  GpuDaemonStatus,
  GpuPerformanceDevice,
  GpuPerformanceResponse,
  PowerControlInfo,
  NumericProfileSettings,
} from "@/features/gpus/types";

export type PerformanceMutationAction =
  | {
      kind: "applyProfile";
      gpuId: string;
      stableId: string;
      settings: NumericProfileSettings;
    }
  | {
      kind: "setPowerLimit";
      gpuId: string;
      limitWatts: number;
    }
  | {
      kind: "resetPowerLimit";
      gpuId: string;
    }
  | {
      kind: "setFanSpeed";
      gpuId: string;
      fanIndex: number;
      speedPercent: number;
    }
  | {
      kind: "setFanModeAuto";
      gpuId: string;
      fanIndex: number;
    }
  | {
      kind: "setClockOffsets";
      gpuId: string;
      graphicsOffsetMHz: number | null;
      memoryOffsetMHz: number | null;
    }
  | {
      kind: "resetClockOffsets";
      gpuId: string;
    };

const EMPTY_PERFORMANCE_GPUS: GpuPerformanceDevice[] = [];
const EMPTY_WARNINGS: string[] = [];

type GpuTuningScreenProps = {
  profiles?: GpuProfileWorkflow;
  inventory?: GpuPerformanceResponse;
  daemonStatus?: GpuDaemonStatus;
  isLoading: boolean;
  errorMessage: string | null;
  mutationErrorMessage: string | null;
  reapplyPending: boolean;
  reapplyErrorMessage: string | null;
  pendingAction: PerformanceMutationAction | null;
  onReapplySavedSettings: () => void;
  onMutationAction: TuningMutationHandler;
};

export function GpuTuningScreen({
  profiles,
  inventory,
  daemonStatus,
  isLoading,
  errorMessage,
  mutationErrorMessage,
  reapplyPending,
  reapplyErrorMessage,
  pendingAction,
  onReapplySavedSettings,
  onMutationAction,
}: GpuTuningScreenProps) {
  const editStore = useTuningEdits();
  const gpus = inventory?.gpus ?? EMPTY_PERFORMANCE_GPUS;
  const warnings = inventory?.warnings ?? EMPTY_WARNINGS;
  const daemonDisconnected = daemonStatus?.reachable === false;
  const daemonConnected = daemonStatus?.reachable === true;
  const daemonMessage =
    daemonStatus?.message ??
    "Privileged GPU controls require prismd to be installed and reachable.";
  const busy =
    editStore.applyAllPending || pendingAction != null || reapplyPending;
  const hasEdits = Object.values(editStore.devices).some(
    (device) => Object.keys(device.edits).length > 0,
  );

  return (
    <div className="space-y-6">
      <div className="flex flex-col gap-4 md:flex-row md:items-start md:justify-between">
        <h1 className="text-2xl font-semibold tracking-tight">Tuning</h1>
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            size="sm"
            disabled={!daemonConnected || !hasEdits || busy || isLoading}
            onClick={() => void editStore.applyAll(gpus, onMutationAction)}
          >
            {editStore.applyAllPending && (
              <LoaderCircle className="animate-spin" />
            )}
            Apply all
          </Button>
          {daemonConnected && (
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={onReapplySavedSettings}
            >
              {reapplyPending ? (
                <LoaderCircle className="animate-spin" />
              ) : (
                <RefreshCw />
              )}
              Reapply Saved Settings
            </Button>
          )}
        </div>
      </div>

      {daemonDisconnected && daemonStatus && (
        <Alert
          variant="destructive"
          className="border-destructive/50 bg-destructive/12"
        >
          <ShieldAlert className="text-destructive" />
          <AlertTitle>Privileged GPU controls are unavailable</AlertTitle>
          <AlertDescription className="text-destructive/90">
            <div className="space-y-1">
              <p>{daemonMessage}</p>
              <p>
                Controls stay visible so you can inspect what the driver exposes
                on this system, but changes stay locked until prismd is
                reachable.
              </p>
              <p className="font-mono text-xs text-destructive/80">
                {daemonStatus.socketPath}
              </p>
            </div>
          </AlertDescription>
        </Alert>
      )}

      {errorMessage && (
        <Alert variant="destructive">
          <AlertTriangle />
          <AlertTitle>Failed to load tuning controls</AlertTitle>
          <AlertDescription>{errorMessage}</AlertDescription>
        </Alert>
      )}
      {mutationErrorMessage && (
        <Alert variant="destructive">
          <AlertTriangle />
          <AlertTitle>Failed to apply change</AlertTitle>
          <AlertDescription>{mutationErrorMessage}</AlertDescription>
        </Alert>
      )}
      {reapplyErrorMessage && (
        <Alert variant="destructive">
          <AlertTriangle />
          <AlertTitle>Failed to reapply saved settings</AlertTitle>
          <AlertDescription>{reapplyErrorMessage}</AlertDescription>
        </Alert>
      )}
      {warnings.length > 0 && (
        <Alert>
          <AlertTriangle className="text-primary" />
          <AlertTitle>Driver warnings</AlertTitle>
          <AlertDescription className="space-y-1">
            {warnings.map((warning) => (
              <p key={warning}>{warning}</p>
            ))}
          </AlertDescription>
        </Alert>
      )}

      <UnavailableGpuEdits
        devices={editStore.devices}
        gpus={gpus}
        onDiscard={editStore.discard}
        pending={busy}
      />

      {profiles && <GpuProfileAvailability workflow={profiles} gpus={gpus} />}

      {isLoading && <GpuTuningSkeleton />}

      {!isLoading && gpus.length === 0 && !errorMessage && (
        <div className="rounded-xl border border-dashed border-border bg-card/50 px-6 py-16 text-center">
          <Cpu className="mx-auto size-8 text-muted-foreground" />
          <p className="mt-4 text-sm font-medium">No supported GPUs detected</p>
          <p className="mt-1 text-sm text-muted-foreground">
            Tuning controls are currently available for NVIDIA GPUs exposed
            through NVML.
          </p>
        </div>
      )}

      {!isLoading && gpus.length > 0 && (
        <div className="space-y-5">
          {gpus.map((gpu) => (
            <div
              key={gpu.id}
              className="standalone space-y-5 border-y border-border/80 py-6"
            >
              <div className="space-y-1">
                <h2 className="text-lg font-semibold tracking-tight">
                  {gpu.name}
                </h2>
                <p className="text-sm text-muted-foreground">
                  Adjust power, cooling, and clock offsets for this GPU.
                </p>
              </div>
              {profiles && (
                <GpuProfilePanel
                  gpu={gpu}
                  workflow={profiles}
                  edits={editStore.devices[gpu.id]?.edits ?? {}}
                  readError={errorMessage}
                  pendingAction={pendingAction}
                  reapplyPending={reapplyPending}
                  loadDisabled={busy || !daemonConnected}
                  onLoad={(profile, settings) => {
                    void editStore.apply(
                      {
                        kind: "applyProfile",
                        gpuId: gpu.id,
                        stableId: profile.gpuId,
                        settings,
                      },
                      gpu,
                      onMutationAction,
                    );
                  }}
                />
              )}
              <fieldset
                disabled={daemonDisconnected || busy}
                aria-disabled={daemonDisconnected || busy}
                className={cn(
                  "transition-opacity",
                  daemonDisconnected &&
                    "pointer-events-none opacity-45 saturate-50",
                )}
              >
                <GpuTuningControls
                  gpu={
                    errorMessage
                      ? unavailableTuningReadback(gpu, errorMessage)
                      : gpu
                  }
                  showIdentity
                  editStore={editStore}
                  pendingAction={pendingAction}
                  onMutationAction={onMutationAction}
                />
              </fieldset>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export function GpuTuningControls({
  gpu,
  showIdentity = false,
  layout = "stack",
  editStore: suppliedEditStore,
  pendingAction,
  onMutationAction: performMutation,
}: {
  gpu: GpuPerformanceDevice;
  editStore?: TuningEditStore;
  showIdentity?: boolean;
  layout?: "stack" | "rail";
  pendingAction: PerformanceMutationAction | null;
  onMutationAction: TuningMutationHandler;
}) {
  const localEditStore = useTuningEdits();
  const editStore = suppliedEditStore ?? localEditStore;
  const edits = editStore.devices[gpu.id]?.edits ?? {};
  const outcomes = editStore.devices[gpu.id]?.outcomes ?? {};
  const onMutationAction = (action: PerformanceMutationAction) => {
    void editStore.apply(action, gpu, performMutation);
  };
  const powerDraft =
    "power" in edits
      ? typeof edits.power === "number"
        ? edits.power
        : edits.power === "default"
          ? gpu.power.defaultLimitWatts
          : null
      : gpu.power.currentLimitWatts;
  const graphicsDraft =
    "graphics" in edits
      ? typeof edits.graphics === "number"
        ? edits.graphics
        : null
      : gpu.clocks.graphics.currentOffsetMHz;
  const memoryDraft =
    "memory" in edits
      ? typeof edits.memory === "number"
        ? edits.memory
        : null
      : gpu.clocks.memory.currentOffsetMHz;
  const fanDrafts = Object.fromEntries(
    gpu.cooling.fans.map((fan) => [
      fan.index,
      typeof edits[`fan:${fan.index}`] === "number"
        ? Number(edits[`fan:${fan.index}`])
        : (fan.currentSpeedPercent ?? null),
    ]),
  );
  const setPowerDraft = (value: number | null) =>
    editStore.change(gpu.id, "power", value);
  const setGraphicsDraft = (value: number | null) =>
    editStore.change(gpu.id, "graphics", value);
  const setMemoryDraft = (value: number | null) =>
    editStore.change(gpu.id, "memory", value);
  const changeSummary = (
    <div className="space-y-2" aria-live="polite">
      {Object.keys(edits).length > 0 && (
        <Badge variant="outline">Unapplied changes</Badge>
      )}
      <TuningOutcomes outcomes={outcomes} />
      {Object.keys(edits).map((key) => {
        const setting = key as keyof typeof edits;
        const label = tuningSettingLabel(setting).toLowerCase();
        const retry = editStore.retryAction(gpu, setting);
        return (
          <div key={setting} className="flex gap-2">
            {outcomes[setting] && (
              <Button
                size="sm"
                disabled={pendingAction != null || !retry}
                onClick={() => retry && onMutationAction(retry)}
              >
                Retry {label} change
              </Button>
            )}
            <Button
              size="sm"
              variant="outline"
              disabled={pendingAction != null}
              onClick={() => editStore.discard(gpu.id, setting)}
            >
              Discard {label} change
            </Button>
          </div>
        );
      })}
    </div>
  );

  const powerBusy = pendingAction != null;
  const clocksBusy = pendingAction != null;

  const graphicsToApply =
    "graphics" in edits &&
    canEditOffset(gpu.clocks.graphics) &&
    inTuningRange(
      graphicsDraft,
      gpu.clocks.graphics.minOffsetMHz,
      gpu.clocks.graphics.maxOffsetMHz,
    )
      ? graphicsDraft
      : null;
  const memoryToApply =
    "memory" in edits &&
    canEditOffset(gpu.clocks.memory) &&
    inTuningRange(
      memoryDraft,
      gpu.clocks.memory.minOffsetMHz,
      gpu.clocks.memory.maxOffsetMHz,
    )
      ? memoryDraft
      : null;
  const hasClockChanges = graphicsToApply != null || memoryToApply != null;

  const identity = showIdentity ? (
    <div className="flex flex-wrap items-center gap-2">
      <VendorBadge vendor={gpu.vendor} />
      {gpu.architecture && <Badge variant="outline">{gpu.architecture}</Badge>}
      {gpu.busId && (
        <span className="font-mono text-xs text-muted-foreground">
          {gpu.busId}
        </span>
      )}
    </div>
  ) : null;

  const powerSection = (
    <PerformanceSection title="Power" icon={Zap}>
      <TuningAvailability
        support={gpu.power.support}
        reason={gpu.power.reason}
        currentValue={gpu.power.currentLimitWatts}
        currentValueError={gpu.power.currentValueError}
      />
      <PowerSectionContent
        power={gpu.power}
        draft={powerDraft}
        layout={layout}
        busy={powerBusy}
        hasChanges={"power" in edits}
        onDraftChange={setPowerDraft}
        onApply={() =>
          powerDraft != null &&
          onMutationAction({
            kind: "setPowerLimit",
            gpuId: gpu.id,
            limitWatts: powerDraft,
          })
        }
        onReset={() =>
          onMutationAction({
            kind: "resetPowerLimit",
            gpuId: gpu.id,
          })
        }
      />
    </PerformanceSection>
  );

  const coolingSection = (
    <PerformanceSection title="Cooling" icon={Wind}>
      <CapabilitySummary
        supported={gpu.cooling.isSupported}
        reason={gpu.cooling.unsupportedReason}
      />
      <CoolingSectionContent
        cooling={gpu.cooling}
        drafts={fanDrafts}
        layout={layout}
        pendingAction={pendingAction}
        onDraftChange={(fanIndex, value) =>
          editStore.change(gpu.id, `fan:${fanIndex}`, value)
        }
        onSetManual={(fanIndex, speedPercent) =>
          onMutationAction({
            kind: "setFanSpeed",
            gpuId: gpu.id,
            fanIndex,
            speedPercent,
          })
        }
        onSetAuto={(fanIndex) =>
          onMutationAction({
            kind: "setFanModeAuto",
            gpuId: gpu.id,
            fanIndex,
          })
        }
      />
    </PerformanceSection>
  );

  const clockSection = (
    <PerformanceSection title="Clock Offsets" icon={Clock3}>
      <ClockSectionContent
        clocks={gpu.clocks}
        graphicsDraft={graphicsDraft}
        memoryDraft={memoryDraft}
        layout={layout}
        busy={clocksBusy}
        hasChanges={hasClockChanges}
        onGraphicsDraftChange={setGraphicsDraft}
        onMemoryDraftChange={setMemoryDraft}
        onApply={() =>
          onMutationAction({
            kind: "setClockOffsets",
            gpuId: gpu.id,
            graphicsOffsetMHz: graphicsToApply,
            memoryOffsetMHz: memoryToApply,
          })
        }
        onReset={() =>
          onMutationAction({
            kind: "resetClockOffsets",
            gpuId: gpu.id,
          })
        }
      />
    </PerformanceSection>
  );

  if (layout === "rail") {
    return (
      <div data-testid="gpu-tuning-controls" className="space-y-5">
        {identity}
        {changeSummary}
        <div className="grid gap-5 sm:grid-cols-2">
          <div className="space-y-6">
            {powerSection}
            {clockSection}
          </div>
          {coolingSection}
        </div>
      </div>
    );
  }

  return (
    <div data-testid="gpu-tuning-controls" className="space-y-6">
      {identity}
      {changeSummary}
      {powerSection}
      {coolingSection}
      {clockSection}
    </div>
  );
}

function PowerSectionContent({
  power,
  draft,
  layout,
  busy,
  onDraftChange,
  onApply,
  onReset,
  hasChanges,
}: {
  power: PowerControlInfo;
  hasChanges: boolean;
  draft: number | null;
  layout: "stack" | "rail";
  busy: boolean;
  onDraftChange: (value: number | null) => void;
  onApply: () => void;
  onReset: () => void;
}) {
  const canEdit =
    power.support === "supported" &&
    power.minLimitWatts != null &&
    power.maxLimitWatts != null &&
    power.currentLimitWatts != null;

  return (
    <div className={cn("space-y-4", layout === "rail" && "text-sm")}>
      <div className="space-y-3 border-t border-border/70 pt-4">
        <div className="flex items-start justify-between gap-3">
          <div>
            <label className="block text-xs font-medium uppercase tracking-[0.22em] text-muted-foreground">
              Target Limit
            </label>
            <p className="mt-1 text-xs text-muted-foreground">
              {power.minLimitWatts != null && power.maxLimitWatts != null
                ? `${power.minLimitWatts}-${power.maxLimitWatts} W allowed`
                : "Limit range unavailable"}
            </p>
            <p className="mt-1 text-xs text-muted-foreground">
              Current limit {formatWatts(power.currentLimitWatts)}
            </p>
          </div>
          <p className="text-sm font-semibold tabular-nums">
            {formatWatts(power.currentLimitWatts == null ? null : draft)}
          </p>
        </div>
        {canEdit && draft != null && (
          <input
            aria-label="Power limit slider"
            type="range"
            className="h-2 w-full cursor-pointer accent-primary disabled:cursor-not-allowed disabled:opacity-50"
            min={power.minLimitWatts ?? 0}
            max={power.maxLimitWatts ?? 0}
            step={1}
            value={draft}
            onChange={(event) =>
              onDraftChange(event.currentTarget.valueAsNumber)
            }
            disabled={!canEdit || busy}
          />
        )}
        <Input
          type="number"
          aria-label="Power limit"
          className="rounded-none border-x-0 border-t-0 bg-transparent px-0 shadow-none focus-visible:ring-0"
          min={power.minLimitWatts ?? undefined}
          max={power.maxLimitWatts ?? undefined}
          step={1}
          value={draft ?? ""}
          onChange={(event) => {
            const nextValue = event.currentTarget.valueAsNumber;
            if (event.currentTarget.value === "") {
              onDraftChange(null);
            } else if (Number.isFinite(nextValue)) {
              onDraftChange(nextValue);
            }
          }}
          disabled={!canEdit || busy}
        />
      </div>

      <div className="grid grid-cols-2 gap-2">
        <Button
          size="sm"
          onClick={onApply}
          disabled={
            !canEdit ||
            !hasChanges ||
            !inTuningRange(draft, power.minLimitWatts, power.maxLimitWatts) ||
            busy
          }
        >
          Apply
        </Button>
        <Button
          size="sm"
          variant="outline"
          onClick={onReset}
          disabled={
            power.support !== "supported" ||
            power.defaultLimitWatts == null ||
            busy
          }
        >
          <RotateCcw className="mr-1 size-3.5" />
          Reset
        </Button>
      </div>
    </div>
  );
}

function CoolingSectionContent({
  cooling,
  drafts,
  layout,
  pendingAction,
  onDraftChange,
  onSetManual,
  onSetAuto,
}: {
  cooling: CoolingInfo;
  drafts: Record<number, number | null>;
  layout: "stack" | "rail";
  pendingAction: PerformanceMutationAction | null;
  onDraftChange: (fanIndex: number, value: number) => void;
  onSetManual: (fanIndex: number, speedPercent: number) => void;
  onSetAuto: (fanIndex: number) => void;
}) {
  if (!cooling.isSupported || cooling.fans.length === 0) {
    return null;
  }

  return (
    <div
      className={cn(
        layout === "rail"
          ? "grid gap-4 [grid-template-columns:repeat(auto-fit,minmax(min(100%,220px),1fr))]"
          : "space-y-5",
      )}
    >
      {cooling.fans.map((fan) => {
        const min = fan.minSpeedPercent ?? 0;
        const max = fan.maxSpeedPercent ?? 100;
        const draft = drafts[fan.index] ?? null;
        const current = fan.currentSpeedPercent;
        const busy = pendingAction != null;

        return (
          <FanEditor
            key={fan.index}
            fan={fan}
            draft={draft}
            busy={busy}
            onDraftChange={(value) => onDraftChange(fan.index, value)}
            onSetManual={() => draft != null && onSetManual(fan.index, draft)}
            onSetAuto={() => onSetAuto(fan.index)}
            current={current}
            min={min}
            max={max}
          />
        );
      })}
    </div>
  );
}

function ClockSectionContent({
  clocks,
  graphicsDraft,
  memoryDraft,
  layout,
  busy,
  hasChanges,
  onGraphicsDraftChange,
  onMemoryDraftChange,
  onApply,
  onReset,
}: {
  clocks: GpuPerformanceDevice["clocks"];
  graphicsDraft: number | null;
  memoryDraft: number | null;
  layout: "stack" | "rail";
  busy: boolean;
  hasChanges: boolean;
  onGraphicsDraftChange: (value: number | null) => void;
  onMemoryDraftChange: (value: number | null) => void;
  onApply: () => void;
  onReset: () => void;
}) {
  return (
    <div className="space-y-5">
      <div
        className={cn(
          layout === "rail"
            ? "grid gap-4 [grid-template-columns:repeat(auto-fit,minmax(min(100%,220px),1fr))]"
            : "space-y-5",
        )}
      >
        <ClockOffsetEditor
          title="Graphics"
          icon={Zap}
          info={clocks.graphics}
          draft={graphicsDraft}
          onDraftChange={onGraphicsDraftChange}
          disabled={!canEditOffset(clocks.graphics) || busy}
        />
        <ClockOffsetEditor
          title="Memory"
          icon={Clock3}
          info={clocks.memory}
          draft={memoryDraft}
          onDraftChange={onMemoryDraftChange}
          disabled={!canEditOffset(clocks.memory) || busy}
        />
      </div>

      <div className="grid grid-cols-2 gap-2">
        <Button size="sm" onClick={onApply} disabled={!hasChanges || busy}>
          Apply
        </Button>
        <Button
          size="sm"
          variant="outline"
          onClick={onReset}
          disabled={
            (clocks.graphics.support !== "supported" &&
              clocks.memory.support !== "supported") ||
            clocks.graphics.support === "unknown" ||
            clocks.memory.support === "unknown" ||
            busy
          }
        >
          <RotateCcw className="mr-1 size-3.5" />
          Reset
        </Button>
      </div>
    </div>
  );
}

function PerformanceSection({
  title,
  icon: Icon,
  children,
  className,
}: {
  title: string;
  icon: typeof Gauge;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section
      className={cn(
        "min-w-0 space-y-4 border-b border-border/70 pb-6 last:border-b-0 last:pb-0",
        className,
      )}
    >
      <div className="flex items-center gap-2">
        <Icon className="size-4 text-primary" />
        <p className="text-sm font-semibold uppercase tracking-[0.18em] text-foreground/85">
          {title}
        </p>
      </div>
      {children}
    </section>
  );
}

function CapabilitySummary({
  supported,
  reason,
}: {
  supported: boolean;
  reason: string | null;
}) {
  if (supported) {
    return null;
  }

  return (
    <div className="border-l border-border/80 bg-muted/20 px-3 py-2 text-xs text-muted-foreground">
      {reason ?? "Unavailable on this system."}
    </div>
  );
}

function FanEditor({
  fan,
  current,
  draft,
  min,
  max,
  busy,
  onDraftChange,
  onSetManual,
  onSetAuto,
}: {
  fan: FanControlInfo;
  current: number | null;
  draft: number | null;
  min: number;
  max: number;
  busy: boolean;
  onDraftChange: (value: number) => void;
  onSetManual: () => void;
  onSetAuto: () => void;
}) {
  const currentMode =
    fan.mode === "manual" ? "Manual" : fan.mode === "auto" ? "Auto" : "Unknown";

  return (
    <div className="space-y-3 border-t border-border/70 pt-4">
      <div className="flex items-center justify-between gap-3">
        <div>
          <p className="text-xs font-medium uppercase tracking-[0.22em] text-muted-foreground">
            Fan {fan.index + 1}
          </p>
          <p className="mt-1 text-xs text-muted-foreground">
            {currentMode} mode
          </p>
        </div>
        <p className="text-xs font-medium text-muted-foreground tabular-nums">
          {min}-{max}%
        </p>
      </div>

      <p className="text-xs text-muted-foreground">
        Current speed {current == null ? "Unavailable" : `${current}%`}
      </p>
      <div className="space-y-3">
        <div className="flex items-center justify-between gap-3">
          <span className="text-xs text-muted-foreground">Manual target</span>
          <span className="text-sm font-semibold tabular-nums">{draft}%</span>
        </div>
        <input
          type="range"
          className="h-2 w-full cursor-pointer accent-primary disabled:cursor-not-allowed disabled:opacity-50"
          min={min}
          max={max}
          step={1}
          value={draft ?? ""}
          onChange={(event) => onDraftChange(event.currentTarget.valueAsNumber)}
          disabled={busy}
        />
        <Input
          type="number"
          aria-label={`Fan ${fan.index + 1} manual target`}
          className="rounded-none border-x-0 border-t-0 bg-transparent px-0 shadow-none focus-visible:ring-0"
          min={min}
          max={max}
          step={1}
          value={draft ?? ""}
          onChange={(event) => {
            const nextValue = event.currentTarget.valueAsNumber;
            if (event.currentTarget.value === "") {
              onDraftChange(0);
            } else if (Number.isFinite(nextValue)) {
              onDraftChange(nextValue);
            }
          }}
          disabled={busy}
        />
      </div>

      <div className="grid grid-cols-2 gap-2">
        <Button
          size="sm"
          onClick={onSetManual}
          disabled={
            busy ||
            !inTuningRange(draft, min, max) ||
            (draft === current && fan.mode === "manual")
          }
        >
          Set Manual
        </Button>
        <Button
          size="sm"
          variant="outline"
          onClick={onSetAuto}
          disabled={busy || fan.mode === "auto"}
        >
          Restore Auto
        </Button>
      </div>
    </div>
  );
}

function ClockOffsetEditor({
  title,
  icon: Icon,
  info,
  draft,
  disabled,
  onDraftChange,
}: {
  title: string;
  icon: typeof Gauge;
  info: ClockOffsetInfo;
  draft: number | null;
  disabled: boolean;
  onDraftChange: (value: number | null) => void;
}) {
  return (
    <div className="space-y-3 border-t border-border/70 pt-4">
      <div className="flex items-start justify-between gap-3">
        <div>
          <div className="flex items-center gap-2">
            <Icon className="size-4 text-primary" />
            <p className="text-xs font-medium uppercase tracking-[0.22em] text-muted-foreground">
              {title}
            </p>
          </div>
          <p className="mt-1 text-xs text-muted-foreground">
            {info.minOffsetMHz != null && info.maxOffsetMHz != null
              ? `${formatOffset(info.minOffsetMHz)} to ${formatOffset(info.maxOffsetMHz)}`
              : "Offset range unavailable"}
          </p>
        </div>
        <p className="text-sm font-semibold tabular-nums">
          {formatOffset(draft)}
        </p>
      </div>

      <p className="text-xs text-muted-foreground">
        Current offset {formatOffset(info.currentOffsetMHz)}
      </p>
      <TuningAvailability
        support={info.support}
        reason={info.reason}
        currentValue={info.currentOffsetMHz}
        currentValueError={info.currentValueError}
      />
      {info.support === "supported" && (
        <div className="space-y-3">
          {canEditOffset(info) && draft != null && (
            <input
              aria-label={`${title} clock offset slider`}
              type="range"
              className="h-2 w-full cursor-pointer accent-primary disabled:cursor-not-allowed disabled:opacity-50"
              min={info.minOffsetMHz ?? 0}
              max={info.maxOffsetMHz ?? 0}
              step={1}
              value={draft}
              onChange={(event) =>
                onDraftChange(event.currentTarget.valueAsNumber)
              }
              disabled={disabled}
            />
          )}
          <Input
            type="number"
            aria-label={`${title} clock offset`}
            className="rounded-none border-x-0 border-t-0 bg-transparent px-0 shadow-none focus-visible:ring-0"
            min={info.minOffsetMHz ?? undefined}
            max={info.maxOffsetMHz ?? undefined}
            step={1}
            value={draft ?? ""}
            onChange={(event) => {
              const nextValue = event.currentTarget.valueAsNumber;
              if (event.currentTarget.value === "") {
                onDraftChange(null);
              } else if (Number.isFinite(nextValue)) {
                onDraftChange(nextValue);
              }
            }}
            disabled={disabled}
          />
        </div>
      )}
    </div>
  );
}

function GpuTuningSkeleton() {
  return (
    <div
      data-testid="gpu-tuning-skeleton"
      className="space-y-5 overflow-hidden rounded-xl border border-border/80 bg-card/80 p-6"
    >
      <div className="space-y-3">
        <Skeleton className="h-5 w-24 rounded-md" />
        <Skeleton className="h-8 w-80 rounded-md" />
        <Skeleton className="h-4 w-80 rounded-md" />
      </div>
      {Array.from({ length: 3 }).map((_, index) => (
        <Skeleton key={index} className="h-56 rounded-xl" />
      ))}
    </div>
  );
}

function formatOffset(value: number | null) {
  if (value == null) {
    return "Unavailable";
  }

  return `${value >= 0 ? "+" : ""}${value} MHz`;
}

function formatWatts(value: number | null) {
  return value == null ? "Unavailable" : `${value} W`;
}

function canEditOffset(info: ClockOffsetInfo) {
  return (
    info.support === "supported" &&
    info.currentOffsetMHz != null &&
    info.minOffsetMHz != null &&
    info.maxOffsetMHz != null
  );
}

function TuningAvailability({
  support,
  reason,
  currentValue,
  currentValueError,
}: {
  support: ControlSupport;
  reason: string | null;
  currentValue: number | null;
  currentValueError: string | null;
}) {
  if (support === "supported" && currentValue != null) return null;
  return (
    <div className="border-l border-border/80 bg-muted/20 px-3 py-2 text-xs text-muted-foreground">
      <p>
        {support === "unsupported"
          ? "Not supported"
          : support === "supported"
            ? "Supported · current value unavailable"
            : "Support currently unknown"}
      </p>
      {reason && <p>{reason}</p>}
      {currentValueError && <p>{currentValueError}</p>}
    </div>
  );
}
