import { useEffect, useState, type ReactNode } from "react";
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
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { VendorBadge } from "@/features/gpus/components/vendor-badge";
import { formatNumber } from "@/features/gpus/lib/formatters";
import { cn } from "@/lib/utils";
import type {
  ClockOffsetInfo,
  CoolingInfo,
  FanControlInfo,
  GpuDaemonStatus,
  GpuPerformanceDevice,
  GpuPerformanceResponse,
  PowerControlInfo,
} from "@/features/gpus/types";

export type PerformanceMutationAction =
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
  inventory?: GpuPerformanceResponse;
  daemonStatus?: GpuDaemonStatus;
  isLoading: boolean;
  errorMessage: string | null;
  mutationErrorMessage: string | null;
  reapplyPending: boolean;
  reapplyErrorMessage: string | null;
  pendingAction: PerformanceMutationAction | null;
  onReapplySavedSettings: () => void;
  onMutationAction: (action: PerformanceMutationAction) => void;
};

export function GpuTuningScreen({
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
  const gpus = inventory?.gpus ?? EMPTY_PERFORMANCE_GPUS;
  const warnings = inventory?.warnings ?? EMPTY_WARNINGS;
  const daemonDisconnected = daemonStatus?.reachable === false;
  const daemonConnected = daemonStatus?.reachable === true;
  const daemonMessage =
    daemonStatus?.message ??
    "Privileged GPU controls require prismd to be installed and reachable.";

  return (
    <div className="space-y-6">
      <div className="flex flex-col gap-4 md:flex-row md:items-start md:justify-between">
        <h1 className="text-2xl font-semibold tracking-tight">Tuning</h1>
        {daemonConnected && (
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={reapplyPending}
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
        <fieldset
          disabled={daemonDisconnected}
          aria-disabled={daemonDisconnected}
          className={cn(
            "space-y-5 transition-opacity",
            daemonDisconnected && "pointer-events-none opacity-45 saturate-50",
          )}
        >
          {gpus.map((gpu) => (
            <GpuTuningCard
              key={gpu.id}
              gpu={gpu}
              pendingAction={pendingAction}
              onMutationAction={onMutationAction}
            />
          ))}
        </fieldset>
      )}
    </div>
  );
}

function GpuTuningCard({
  gpu,
  pendingAction,
  onMutationAction,
}: {
  gpu: GpuPerformanceDevice;
  pendingAction: PerformanceMutationAction | null;
  onMutationAction: (action: PerformanceMutationAction) => void;
}) {
  const [powerDraft, setPowerDraft] = useState(
    gpu.power.currentLimitWatts ?? gpu.power.defaultLimitWatts ?? 0,
  );
  const [fanDrafts, setFanDrafts] = useState<Record<number, number>>({});
  const [graphicsDraft, setGraphicsDraft] = useState(
    gpu.clocks.graphics.currentOffsetMHz ?? 0,
  );
  const [memoryDraft, setMemoryDraft] = useState(
    gpu.clocks.memory.currentOffsetMHz ?? 0,
  );

  useEffect(() => {
    setPowerDraft(
      gpu.power.currentLimitWatts ?? gpu.power.defaultLimitWatts ?? 0,
    );
  }, [gpu.id, gpu.power.currentLimitWatts, gpu.power.defaultLimitWatts]);

  useEffect(() => {
    setFanDrafts(
      Object.fromEntries(
        gpu.cooling.fans.map((fan) => [
          fan.index,
          fan.currentSpeedPercent ?? fan.minSpeedPercent ?? 0,
        ]),
      ),
    );
  }, [gpu.cooling.fans, gpu.id]);

  useEffect(() => {
    setGraphicsDraft(gpu.clocks.graphics.currentOffsetMHz ?? 0);
    setMemoryDraft(gpu.clocks.memory.currentOffsetMHz ?? 0);
  }, [
    gpu.clocks.graphics.currentOffsetMHz,
    gpu.clocks.memory.currentOffsetMHz,
    gpu.id,
  ]);

  const powerBusy =
    pendingAction?.gpuId === gpu.id &&
    (pendingAction.kind === "setPowerLimit" ||
      pendingAction.kind === "resetPowerLimit");
  const clocksBusy =
    pendingAction?.gpuId === gpu.id &&
    (pendingAction.kind === "setClockOffsets" ||
      pendingAction.kind === "resetClockOffsets");

  const normalizedPowerDraft = normalizeDraft(
    powerDraft,
    gpu.power.minLimitWatts,
    gpu.power.maxLimitWatts,
  );
  const normalizedGraphicsDraft = normalizeDraft(
    graphicsDraft,
    gpu.clocks.graphics.minOffsetMHz,
    gpu.clocks.graphics.maxOffsetMHz,
  );
  const normalizedMemoryDraft = normalizeDraft(
    memoryDraft,
    gpu.clocks.memory.minOffsetMHz,
    gpu.clocks.memory.maxOffsetMHz,
  );
  const hasClockChanges =
    (gpu.clocks.graphics.isSupported &&
      normalizedGraphicsDraft !==
        (gpu.clocks.graphics.currentOffsetMHz ?? 0)) ||
    (gpu.clocks.memory.isSupported &&
      normalizedMemoryDraft !== (gpu.clocks.memory.currentOffsetMHz ?? 0));

  return (
    <Card className="border-border/80 bg-card/80 py-0">
      <CardHeader className="border-b border-border/80 py-5">
        <div className="flex flex-wrap items-center gap-2">
          <VendorBadge vendor={gpu.vendor} />
          {gpu.architecture && <Badge variant="outline">{gpu.architecture}</Badge>}
          {gpu.busId && (
            <span className="font-mono text-xs text-muted-foreground">
              {gpu.busId}
            </span>
          )}
        </div>
        <div className="space-y-1">
          <CardTitle>{gpu.name}</CardTitle>
          <p className="text-sm text-muted-foreground">
            Adjust power, cooling, and clock offsets for this GPU.
          </p>
        </div>
      </CardHeader>

      <CardContent className="py-6">
        <div data-testid="gpu-tuning-controls" className="space-y-4">
          <PerformanceSection title="Power" icon={Gauge}>
            <CapabilitySummary
              supported={gpu.power.isSupported}
              reason={gpu.power.unsupportedReason}
            />
            <PowerSectionContent
              power={gpu.power}
              draft={powerDraft}
              busy={powerBusy}
              onDraftChange={setPowerDraft}
              onApply={() =>
                onMutationAction({
                  kind: "setPowerLimit",
                  gpuId: gpu.id,
                  limitWatts: normalizedPowerDraft,
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

          <PerformanceSection title="Cooling" icon={Wind}>
            <CapabilitySummary
              supported={gpu.cooling.isSupported}
              reason={gpu.cooling.unsupportedReason}
            />
            <CoolingSectionContent
              gpuId={gpu.id}
              cooling={gpu.cooling}
              drafts={fanDrafts}
              pendingAction={pendingAction}
              onDraftChange={(fanIndex, value) =>
                setFanDrafts((current) => ({
                  ...current,
                  [fanIndex]: value,
                }))
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

          <PerformanceSection title="Clock Offsets" icon={Clock3}>
            <CapabilitySummary
              supported={gpu.clocks.isSupported}
              reason={gpu.clocks.unsupportedReason}
            />
            <ClockSectionContent
              clocks={gpu.clocks}
              graphicsDraft={graphicsDraft}
              memoryDraft={memoryDraft}
              busy={clocksBusy}
              hasChanges={hasClockChanges}
              onGraphicsDraftChange={setGraphicsDraft}
              onMemoryDraftChange={setMemoryDraft}
              onApply={() =>
                onMutationAction({
                  kind: "setClockOffsets",
                  gpuId: gpu.id,
                  graphicsOffsetMHz: gpu.clocks.graphics.isSupported
                    ? normalizedGraphicsDraft
                    : null,
                  memoryOffsetMHz: gpu.clocks.memory.isSupported
                    ? normalizedMemoryDraft
                    : null,
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
        </div>
      </CardContent>
    </Card>
  );
}

function PowerSectionContent({
  power,
  draft,
  busy,
  onDraftChange,
  onApply,
  onReset,
}: {
  power: PowerControlInfo;
  draft: number;
  busy: boolean;
  onDraftChange: (value: number) => void;
  onApply: () => void;
  onReset: () => void;
}) {
  const canEdit =
    power.isSupported &&
    power.minLimitWatts != null &&
    power.maxLimitWatts != null &&
    power.currentLimitWatts != null;
  const normalizedDraft = normalizeDraft(
    draft,
    power.minLimitWatts,
    power.maxLimitWatts,
  );
  const hasChanges =
    normalizedDraft !== (power.currentLimitWatts ?? normalizedDraft);

  return (
    <div className="space-y-4">
      <div className="grid grid-cols-2 gap-3">
        <InlineMetric
          label="Current"
          value={formatNumber(power.currentDrawWatts, "W")}
        />
        <InlineMetric
          label="Default"
          value={formatNumber(power.defaultLimitWatts, "W")}
        />
        <InlineMetric
          label="Enforced"
          value={formatNumber(power.enforcedLimitWatts, "W")}
        />
        <InlineMetric
          label="Range"
          value={
            power.minLimitWatts != null && power.maxLimitWatts != null
              ? `${power.minLimitWatts}-${power.maxLimitWatts} W`
              : "Unavailable"
          }
        />
      </div>

      <div className="space-y-3">
        <label className="block text-xs font-medium uppercase tracking-[0.22em] text-muted-foreground">
          Target limit
        </label>
        <input
          type="range"
          className="h-2 w-full cursor-pointer accent-primary disabled:cursor-not-allowed disabled:opacity-50"
          min={power.minLimitWatts ?? 0}
          max={power.maxLimitWatts ?? 0}
          step={1}
          value={normalizedDraft}
          onChange={(event) => onDraftChange(event.currentTarget.valueAsNumber)}
          disabled={!canEdit || busy}
        />
        <Input
          type="number"
          min={power.minLimitWatts ?? undefined}
          max={power.maxLimitWatts ?? undefined}
          step={1}
          value={normalizedDraft}
          onChange={(event) => {
            const nextValue = event.currentTarget.valueAsNumber;
            if (Number.isFinite(nextValue)) {
              onDraftChange(nextValue);
            }
          }}
          disabled={!canEdit || busy}
        />
      </div>

      <div className="flex gap-2">
        <Button
          size="sm"
          className="flex-1"
          onClick={onApply}
          disabled={!canEdit || !hasChanges || busy}
        >
          Apply
        </Button>
        <Button
          size="sm"
          variant="outline"
          className="flex-1"
          onClick={onReset}
          disabled={!power.isSupported || busy}
        >
          <RotateCcw className="mr-1 size-3.5" />
          Reset
        </Button>
      </div>
    </div>
  );
}

function CoolingSectionContent({
  gpuId,
  cooling,
  drafts,
  pendingAction,
  onDraftChange,
  onSetManual,
  onSetAuto,
}: {
  gpuId: string;
  cooling: CoolingInfo;
  drafts: Record<number, number>;
  pendingAction: PerformanceMutationAction | null;
  onDraftChange: (fanIndex: number, value: number) => void;
  onSetManual: (fanIndex: number, speedPercent: number) => void;
  onSetAuto: (fanIndex: number) => void;
}) {
  if (!cooling.isSupported || cooling.fans.length === 0) {
    return null;
  }

  return (
    <div className="space-y-4">
      {cooling.fans.map((fan) => {
        const min = fan.minSpeedPercent ?? 0;
        const max = fan.maxSpeedPercent ?? 100;
        const draft = normalizeDraft(
          drafts[fan.index] ?? fan.currentSpeedPercent ?? min,
          min,
          max,
        );
        const current = fan.currentSpeedPercent ?? draft;
        const busy =
          pendingAction?.gpuId === gpuId &&
          ((pendingAction.kind === "setFanSpeed" &&
            pendingAction.fanIndex === fan.index) ||
            (pendingAction.kind === "setFanModeAuto" &&
              pendingAction.fanIndex === fan.index));

        return (
          <FanEditor
            key={fan.index}
            fan={fan}
            draft={draft}
            busy={busy}
            onDraftChange={(value) => onDraftChange(fan.index, value)}
            onSetManual={() => onSetManual(fan.index, draft)}
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
  busy,
  hasChanges,
  onGraphicsDraftChange,
  onMemoryDraftChange,
  onApply,
  onReset,
}: {
  clocks: GpuPerformanceDevice["clocks"];
  graphicsDraft: number;
  memoryDraft: number;
  busy: boolean;
  hasChanges: boolean;
  onGraphicsDraftChange: (value: number) => void;
  onMemoryDraftChange: (value: number) => void;
  onApply: () => void;
  onReset: () => void;
}) {
  return (
    <div className="space-y-4">
      <ClockOffsetEditor
        title="Graphics"
        icon={Zap}
        info={clocks.graphics}
        draft={graphicsDraft}
        onDraftChange={onGraphicsDraftChange}
        disabled={!clocks.graphics.isSupported || busy}
      />
      <ClockOffsetEditor
        title="Memory"
        icon={Clock3}
        info={clocks.memory}
        draft={memoryDraft}
        onDraftChange={onMemoryDraftChange}
        disabled={!clocks.memory.isSupported || busy}
      />

      <div className="flex gap-2">
        <Button
          size="sm"
          className="flex-1"
          onClick={onApply}
          disabled={!clocks.isSupported || !hasChanges || busy}
        >
          Apply
        </Button>
        <Button
          size="sm"
          variant="outline"
          className="flex-1"
          onClick={onReset}
          disabled={!clocks.isSupported || busy}
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
        "min-w-0 space-y-4 rounded-xl border border-border bg-background/45 p-4",
        className,
      )}
    >
      <div className="flex items-center gap-2">
        <Icon className="size-4 text-primary" />
        <CardTitle className="text-sm uppercase tracking-[0.18em] text-foreground/85">
          {title}
        </CardTitle>
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
    <div className="rounded-lg border border-border bg-muted/50 px-3 py-2 text-xs text-muted-foreground">
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
  current: number;
  draft: number;
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
    <div className="rounded-lg border border-border bg-background/60 p-3">
      <div className="flex items-center justify-between gap-3">
        <div>
          <p className="text-sm font-medium">Fan {fan.index + 1}</p>
          <p className="mt-1 text-xs text-muted-foreground">
            Current {formatPercent(fan.currentSpeedPercent)} · Mode {currentMode}
          </p>
        </div>
        <Badge variant="outline">
          {min}-{max}%
        </Badge>
      </div>

      <div className="mt-3 space-y-3">
        <input
          type="range"
          className="h-2 w-full cursor-pointer accent-primary disabled:cursor-not-allowed disabled:opacity-50"
          min={min}
          max={max}
          step={1}
          value={draft}
          onChange={(event) => onDraftChange(event.currentTarget.valueAsNumber)}
          disabled={busy}
        />
        <Input
          type="number"
          min={min}
          max={max}
          step={1}
          value={draft}
          onChange={(event) => {
            const nextValue = event.currentTarget.valueAsNumber;
            if (Number.isFinite(nextValue)) {
              onDraftChange(nextValue);
            }
          }}
          disabled={busy}
        />
      </div>

      <div className="mt-3 flex gap-2">
        <Button
          size="sm"
          className="flex-1"
          onClick={onSetManual}
          disabled={busy || draft === current}
        >
          Set Manual
        </Button>
        <Button
          size="sm"
          variant="outline"
          className="flex-1"
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
  draft: number;
  disabled: boolean;
  onDraftChange: (value: number) => void;
}) {
  const normalizedDraft = normalizeDraft(
    draft,
    info.minOffsetMHz,
    info.maxOffsetMHz,
  );

  return (
    <div className="rounded-lg border border-border bg-background/60 p-3">
      <div className="flex items-center gap-2">
        <Icon className="size-4 text-primary" />
        <div>
          <p className="text-sm font-medium">{title}</p>
          <p className="mt-1 text-xs text-muted-foreground">
            Current {formatOffset(info.currentOffsetMHz)} · Range{" "}
            {info.minOffsetMHz != null && info.maxOffsetMHz != null
              ? `${formatOffset(info.minOffsetMHz)} to ${formatOffset(info.maxOffsetMHz)}`
              : "Unavailable"}
          </p>
        </div>
      </div>

      {info.isSupported ? (
        <div className="mt-3 space-y-3">
          <input
            type="range"
            className="h-2 w-full cursor-pointer accent-primary disabled:cursor-not-allowed disabled:opacity-50"
            min={info.minOffsetMHz ?? 0}
            max={info.maxOffsetMHz ?? 0}
            step={1}
            value={normalizedDraft}
            onChange={(event) =>
              onDraftChange(event.currentTarget.valueAsNumber)
            }
            disabled={disabled}
          />
          <Input
            type="number"
            min={info.minOffsetMHz ?? undefined}
            max={info.maxOffsetMHz ?? undefined}
            step={1}
            value={normalizedDraft}
            onChange={(event) => {
              const nextValue = event.currentTarget.valueAsNumber;
              if (Number.isFinite(nextValue)) {
                onDraftChange(nextValue);
              }
            }}
            disabled={disabled}
          />
        </div>
      ) : (
        <p className="mt-3 rounded-lg border border-border bg-muted/50 px-3 py-2 text-xs text-muted-foreground">
          {info.unsupportedReason ?? "Unavailable on this system."}
        </p>
      )}
    </div>
  );
}

function InlineMetric({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border border-border bg-background/60 p-3">
      <p className="text-xs uppercase tracking-[0.18em] text-muted-foreground">
        {label}
      </p>
      <p className="mt-2 text-sm font-medium tabular-nums">{value}</p>
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

function normalizeDraft(value: number, min: number | null, max: number | null) {
  if (!Number.isFinite(value)) {
    return min ?? max ?? 0;
  }

  if (min != null && value < min) {
    return min;
  }

  if (max != null && value > max) {
    return max;
  }

  return value;
}

function formatPercent(value: number | null) {
  return value == null ? "Unavailable" : `${value}%`;
}

function formatOffset(value: number | null) {
  if (value == null) {
    return "Unavailable";
  }

  return `${value >= 0 ? "+" : ""}${value} MHz`;
}
