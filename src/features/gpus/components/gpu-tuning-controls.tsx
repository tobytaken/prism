import { TuningOutcomes, tuningSettingLabel } from "./tuning-outcomes";
import { inTuningRange } from "@/features/gpus/lib/tuning-intentions";
import type { TuningMutationHandler } from "@/features/gpus/lib/tuning-mutation";
import { type ReactNode } from "react";
import { Clock3, Gauge, RotateCcw, Wind, Zap } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { CoolingConflictReview } from "./cooling-conflict-review";
import { CoolingControlStatus } from "@/features/gpus/components/cooling-control-status";
import { CoolingArrangementEditor } from "@/features/gpus/components/cooling-arrangement-editor";
import { VendorBadge } from "@/features/gpus/components/vendor-badge";
import {
  useTuningEdits,
  type TuningEditStore,
  type TuningEdits,
} from "@/features/gpus/hooks/use-tuning-edits";
import { cn } from "@/lib/utils";
import type {
  ClockOffsetInfo,
  CoolingConfiguration,
  CoolingInfo,
  ControlSupport,
  FanControlInfo,
  GpuPerformanceDevice,
  PowerControlInfo,
  GpuProfileSettings,
  ProfileRangeCorrection,
  ProfileSettingKey,
} from "@/features/gpus/types";

export type PerformanceMutationAction =
  | {
      kind: "applyProfile";
      gpuId: string;
      stableId: string;
      /** The instructions to apply, which a saved profile's id and corrections must yield. */
      settings: GpuProfileSettings;
      /** Applies the saved profile by id rather than these instructions alone. */
      profileId?: string;
      /** Reviewed range corrections to save to the profile before applying it. */
      corrections?: ProfileRangeCorrection[];
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
      expectedRevision?: number;
      stableId?: string;
      gpuId: string;
      fanIndex: number;
      speedPercent: number;
    }
  | {
      kind: "setFanModeAuto";
      expectedRevision?: number;
      stableId?: string;
      gpuId: string;
      fanIndex: number;
    }
  | {
      kind: "setClockOffsets";
      gpuId: string;
      // "default" requests the default offset, which is zero.
      graphicsOffsetMHz: number | "default" | null;
      memoryOffsetMHz: number | "default" | null;
    }
  | {
      kind: "resetClockOffsets";
      gpuId: string;
    }
  | {
      kind: "applyCooling";
      fanIndexes?: number[];
      /** Batches existing individual Fixed/Automatic drafts through the controller. */
      individualFanEdits?: boolean;
      expectedRevision?: number;
      gpuId: string;
      /** The GPU's UUID, which the daemon's cooling control is keyed by. */
      stableId: string;
      configuration: CoolingConfiguration;
    };

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
  const offsetDraft = (setting: "graphics" | "memory") =>
    setting in edits
      ? typeof edits[setting] === "number"
        ? edits[setting]
        : edits[setting] === "default"
          ? 0
          : null
      : gpu.clocks[setting].currentOffsetMHz;
  const graphicsDraft = offsetDraft("graphics");
  const memoryDraft = offsetDraft("memory");
  // A default choice is shown as applied only while the GPU still reports the resolved default.
  const appliedDefaults = editStore.appliedDefaults(gpu.id);
  const defaultChoice = (
    setting: "power" | "graphics" | "memory",
    key: ProfileSettingKey,
    current: number | null,
    resolved: number | null,
  ): DefaultChoice =>
    setting in edits
      ? edits[setting] === "default"
        ? "unapplied"
        : null
      : appliedDefaults.includes(key) && current != null && current === resolved
        ? "applied"
        : null;
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

  const offsetToApply = (setting: "graphics" | "memory") => {
    const info = gpu.clocks[setting];
    const draft = offsetDraft(setting);
    return setting in edits &&
      canEditOffset(info) &&
      inTuningRange(draft, info.minOffsetMHz, info.maxOffsetMHz)
      ? edits[setting] === "default"
        ? ("default" as const)
        : draft
      : null;
  };
  const graphicsToApply = offsetToApply("graphics");
  const memoryToApply = offsetToApply("memory");
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
        defaultChoice={defaultChoice(
          "power",
          "powerLimitWatts",
          gpu.power.currentLimitWatts,
          gpu.power.defaultLimitWatts,
        )}
        onDraftChange={setPowerDraft}
        onUseDefault={() => editStore.change(gpu.id, "power", "default")}
        onApply={() =>
          edits.power === "default"
            ? onMutationAction({ kind: "resetPowerLimit", gpuId: gpu.id })
            : powerDraft != null &&
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

  const applyCooling = (
    configuration: CoolingConfiguration,
    fanIndexes?: number[],
  ) =>
    gpu.stableId &&
    onMutationAction({
      kind: "applyCooling",
      gpuId: gpu.id,
      stableId: gpu.stableId,
      configuration,
      fanIndexes,
    });

  const hasFanEdits = Object.keys(edits).some((key) => key.startsWith("fan:"));
  const coolingSection = (
    <PerformanceSection title="Cooling" icon={Wind}>
      <CapabilitySummary
        supported={gpu.cooling.isSupported}
        reason={gpu.cooling.unsupportedReason}
      />
      <CoolingSectionContent
        cooling={gpu.cooling}
        readOnly={
          edits.cooling != null ||
          (!hasFanEdits && gpu.cooling.control?.applied != null)
        }
        drafts={fanDrafts}
        edits={edits}
        layout={layout}
        pendingAction={pendingAction}
        onDraftChange={(fanIndex, value) =>
          editStore.change(
            gpu.id,
            `fan:${fanIndex}`,
            value,
            gpu.cooling.control?.revision,
          )
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
      {editStore.coolingNeedsReview(gpu) && gpu.cooling.control && (
        <CoolingConflictReview
          control={gpu.cooling.control}
          fans={gpu.cooling.fans}
          edits={edits}
          busy={pendingAction != null}
          onReview={(revision) => editStore.reviewCooling(gpu.id, revision)}
        />
      )}
      <CoolingControlStatus
        gpu={gpu}
        busy={pendingAction != null || edits.cooling != null || hasFanEdits}
        onApply={applyCooling}
      />
      {hasFanEdits ? (
        <p className="text-xs text-muted-foreground">
          Apply or discard individual fan changes before changing the cooling
          arrangement.
        </p>
      ) : (
        <CoolingArrangementEditor
          key={gpu.id}
          gpu={gpu}
          draft={edits.cooling}
          busy={pendingAction != null}
          onChange={(draft) =>
            editStore.change(
              gpu.id,
              "cooling",
              draft,
              gpu.cooling.control?.revision,
            )
          }
          onApply={applyCooling}
        />
      )}
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
        graphicsUnapplied={"graphics" in edits}
        memoryUnapplied={"memory" in edits}
        graphicsDefaultChoice={defaultChoice(
          "graphics",
          "graphicsOffsetMHz",
          gpu.clocks.graphics.currentOffsetMHz,
          0,
        )}
        memoryDefaultChoice={defaultChoice(
          "memory",
          "memoryOffsetMHz",
          gpu.clocks.memory.currentOffsetMHz,
          0,
        )}
        onGraphicsDraftChange={setGraphicsDraft}
        onMemoryDraftChange={setMemoryDraft}
        onUseDefault={(setting) => editStore.change(gpu.id, setting, "default")}
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
  defaultChoice,
  onDraftChange,
  onUseDefault,
  onApply,
  onReset,
  hasChanges,
}: {
  power: PowerControlInfo;
  hasChanges: boolean;
  draft: number | null;
  layout: "stack" | "rail";
  busy: boolean;
  defaultChoice: DefaultChoice;
  onDraftChange: (value: number | null) => void;
  onUseDefault: () => void;
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
            <UnappliedNote
              pending={hasChanges}
              reported={formatWatts(power.currentLimitWatts)}
            />
            <DefaultChoiceNote choice={defaultChoice} />
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

      <Button
        size="sm"
        variant="ghost"
        aria-label="Use default power limit"
        onClick={onUseDefault}
        disabled={
          !canEdit ||
          !inTuningRange(
            power.defaultLimitWatts,
            power.minLimitWatts,
            power.maxLimitWatts,
          ) ||
          defaultChoice != null ||
          busy
        }
      >
        Use default
      </Button>
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
  readOnly,
  drafts,
  edits,
  layout,
  pendingAction,
  onDraftChange,
  onSetManual,
  onSetAuto,
}: {
  cooling: CoolingInfo;
  readOnly: boolean;
  drafts: Record<number, number | null>;
  edits: TuningEdits;
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
            readOnly={readOnly}
            draft={draft}
            unapplied={`fan:${fan.index}` in edits}
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
  graphicsUnapplied,
  memoryUnapplied,
  graphicsDefaultChoice,
  memoryDefaultChoice,
  onGraphicsDraftChange,
  onMemoryDraftChange,
  onUseDefault,
  onApply,
  onReset,
}: {
  clocks: GpuPerformanceDevice["clocks"];
  graphicsDraft: number | null;
  memoryDraft: number | null;
  layout: "stack" | "rail";
  busy: boolean;
  hasChanges: boolean;
  graphicsUnapplied: boolean;
  memoryUnapplied: boolean;
  graphicsDefaultChoice: DefaultChoice;
  memoryDefaultChoice: DefaultChoice;
  onGraphicsDraftChange: (value: number | null) => void;
  onMemoryDraftChange: (value: number | null) => void;
  onUseDefault: (setting: "graphics" | "memory") => void;
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
          unapplied={graphicsUnapplied}
          defaultChoice={graphicsDefaultChoice}
          onDraftChange={onGraphicsDraftChange}
          onUseDefault={() => onUseDefault("graphics")}
          disabled={!canEditOffset(clocks.graphics) || busy}
        />
        <ClockOffsetEditor
          title="Memory"
          icon={Clock3}
          info={clocks.memory}
          draft={memoryDraft}
          unapplied={memoryUnapplied}
          defaultChoice={memoryDefaultChoice}
          onDraftChange={onMemoryDraftChange}
          onUseDefault={() => onUseDefault("memory")}
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
      aria-label={title}
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
  readOnly,
  current,
  draft,
  unapplied,
  min,
  max,
  busy,
  onDraftChange,
  onSetManual,
  onSetAuto,
}: {
  fan: FanControlInfo;
  readOnly: boolean;
  current: number | null;
  draft: number | null;
  unapplied: boolean;
  min: number;
  max: number;
  busy: boolean;
  onDraftChange: (value: number) => void;
  onSetManual: () => void;
  onSetAuto: () => void;
}) {
  // The driver's mode only: a curve also runs in manual mode without being a fixed speed.
  const reportedMode =
    fan.mode === "manual"
      ? "Driver reports manual mode"
      : fan.mode === "auto"
        ? "Driver reports auto mode"
        : "Driver doesn't report the mode";

  return (
    <div className="space-y-3 border-t border-border/70 pt-4">
      <div className="flex items-center justify-between gap-3">
        <div>
          <p className="text-xs font-medium uppercase tracking-[0.22em] text-muted-foreground">
            Fan {fan.index + 1}
          </p>
          <p className="mt-1 text-xs text-muted-foreground">{reportedMode}</p>
        </div>
        <p className="text-xs font-medium text-muted-foreground tabular-nums">
          {min}-{max}%
        </p>
      </div>

      <p className="text-xs text-muted-foreground">
        Current speed {current == null ? "Unavailable" : `${current}%`}
      </p>
      {!readOnly && (
        <>
          <UnappliedNote
            pending={unapplied}
            reported={current == null ? "Unavailable" : `${current}%`}
          />
          <div className="space-y-3">
            <div className="flex items-center justify-between gap-3">
              <span className="text-xs text-muted-foreground">
                Manual target
              </span>
              <span className="text-sm font-semibold tabular-nums">
                {draft}%
              </span>
            </div>
            <input
              type="range"
              className="h-2 w-full cursor-pointer accent-primary disabled:cursor-not-allowed disabled:opacity-50"
              min={min}
              max={max}
              step={1}
              value={draft ?? ""}
              onChange={(event) =>
                onDraftChange(event.currentTarget.valueAsNumber)
              }
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
        </>
      )}
    </div>
  );
}

function ClockOffsetEditor({
  title,
  icon: Icon,
  info,
  draft,
  unapplied,
  disabled,
  defaultChoice,
  onDraftChange,
  onUseDefault,
}: {
  title: string;
  icon: typeof Gauge;
  info: ClockOffsetInfo;
  draft: number | null;
  unapplied: boolean;
  disabled: boolean;
  defaultChoice: DefaultChoice;
  onDraftChange: (value: number | null) => void;
  onUseDefault: () => void;
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
      <UnappliedNote
        pending={unapplied}
        reported={formatOffset(info.currentOffsetMHz)}
      />
      <DefaultChoiceNote choice={defaultChoice} />
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
          <Button
            size="sm"
            variant="ghost"
            aria-label={`Use default ${title.toLowerCase()} clock offset`}
            onClick={onUseDefault}
            disabled={
              disabled ||
              defaultChoice != null ||
              !inTuningRange(0, info.minOffsetMHz, info.maxOffsetMHz)
            }
          >
            Use default
          </Button>
        </div>
      )}
    </div>
  );
}

type DefaultChoice = "unapplied" | "applied" | null;

function UnappliedNote({
  pending,
  reported,
}: {
  pending: boolean;
  reported: string;
}) {
  if (!pending) return null;
  return (
    <p className="mt-1 text-xs text-amber-500">
      unapplied · reported {reported}
    </p>
  );
}

function DefaultChoiceNote({ choice }: { choice: DefaultChoice }) {
  if (!choice) return null;
  return (
    <p className="mt-1 text-xs text-primary">
      {choice === "unapplied" ? "Use default (not applied)" : "Uses default"}
    </p>
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
