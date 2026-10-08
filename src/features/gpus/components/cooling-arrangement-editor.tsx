import { useQueries } from "@tanstack/react-query";
import { fanCurvePreviewOptions } from "@/features/gpus/lib/fan-curve-preview";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import { FanCurveSection, useFanCurveSupport } from "./fan-curve-section";
import {
  coolingConfiguration,
  coolingDraft,
  type CoolingDraft,
  type FanSettingDraft,
} from "@/features/gpus/hooks/use-tuning-edits";
import type {
  CoolingConfiguration,
  GpuPerformanceDevice,
} from "@/features/gpus/types";

/** Arrangement changes are edits: only Apply hands a complete configuration to the daemon. */
export function CoolingArrangementEditor({
  gpu,
  draft,
  busy,
  onChange,
  onApply,
}: {
  gpu: GpuPerformanceDevice;
  draft?: CoolingDraft;
  busy: boolean;
  onChange: (draft: CoolingDraft) => void;
  onApply: (configuration: CoolingConfiguration, fanIndexes?: number[]) => void;
}) {
  const [joining, setJoining] = useState(false);
  const applied = gpu.cooling.control?.applied;
  const remembered = gpu.cooling.control?.rememberedCurve;
  const recall = remembered
    ? () =>
        onChange({
          ...coolingDraft(remembered),
          stableId: gpu.stableId,
          fanIndexes:
            remembered.arrangement === "perFan"
              ? remembered.fans.map((fan) => fan.fanIndex)
              : gpu.cooling.fans.map((fan) => fan.index),
        })
    : undefined;
  const current =
    draft ?? (applied ? coolingDraft(applied) : { mode: "automatic" as const });
  const indexes = draft?.fanIndexes ?? gpu.cooling.fans.map((fan) => fan.index);
  const identityChanged =
    draft?.stableId != null && draft.stableId !== gpu.stableId;
  const fansChanged =
    indexes.length !== gpu.cooling.fans.length ||
    indexes.some(
      (index) => !gpu.cooling.fans.some((fan) => fan.index === index),
    );
  const complete = coolingConfiguration(current);
  const configurations =
    current.mode === "perFan"
      ? current.fans
      : [{ fanIndex: undefined, setting: current }];
  const completeFans =
    complete?.arrangement === "perFan"
      ? complete.fans
      : complete
        ? [{ fanIndex: undefined, setting: complete.setting }]
        : [];
  const previews = useQueries({
    queries: completeFans.flatMap(({ fanIndex, setting }) =>
      setting.mode === "curve"
        ? [fanCurvePreviewOptions(gpu, setting.points, fanIndex)]
        : [],
    ),
  });
  const invalidFixed = configurations.some(
    ({ fanIndex, setting }) =>
      setting.mode === "fixed" &&
      gpu.cooling.fans
        .filter((fan) => fanIndex == null || fan.index === fanIndex)
        .some(
          (fan) =>
            setting.speedPercent == null ||
            setting.speedPercent < (fan.minSpeedPercent ?? 0) ||
            setting.speedPercent > (fan.maxSpeedPercent ?? 100),
        ),
  );
  const invalid =
    !complete ||
    invalidFixed ||
    previews.some(
      (preview) => !preview.isSuccess || preview.data.problems.length > 0,
    );
  const blocked =
    busy ||
    !gpu.stableId ||
    !gpu.cooling.isSupported ||
    indexes.length === 0 ||
    identityChanged ||
    fansChanged;
  const change = (next: CoolingDraft) =>
    onChange({
      ...next,
      stableId: draft?.stableId ?? gpu.stableId,
      fanIndexes: indexes,
    });
  const apply = (next: CoolingDraft) => {
    const configuration = coolingConfiguration(next);
    if (configuration && !blocked) onApply(configuration, indexes);
  };
  const join = (setting: FanSettingDraft) => {
    change(setting);
    setJoining(false);
  };
  const separate = current.mode === "perFan";
  // A partial application may only establish settings for some fans. Unknown fans need an
  // explicit mode choice; they must not acquire an invented applied configuration.
  const entries = separate ? current.fans : [];
  return (
    <div className="space-y-4">
      <div className="flex flex-wrap gap-2">
        <Button
          variant="outline"
          size="sm"
          disabled={blocked}
          onClick={() => {
            if (current.mode !== "perFan") {
              change({
                mode: "perFan",
                fans: indexes.map((fanIndex) => ({
                  fanIndex,
                  setting: structuredClone(current),
                })),
              });
            } else if (
              current.fans.length &&
              current.fans.every(
                (fan) =>
                  JSON.stringify(fan.setting) ===
                  JSON.stringify(current.fans[0].setting),
              )
            ) {
              join(current.fans[0].setting);
            } else setJoining(true);
          }}
        >
          {separate ? "Share one configuration" : "Separate fans"}
        </Button>
      </div>
      {remembered && (
        <p className="text-xs text-muted-foreground">
          {separate
            ? "Selecting Curve recalls that fan's last manually applied curve when available. Other fan edits stay unchanged."
            : "Selecting Curve recalls your last manually applied cooling arrangement as a draft."}{" "}
          Review it before applying.
        </p>
      )}
      {(identityChanged || fansChanged) && (
        <p role="alert">
          The GPU or targeted fans changed. Discard this cooling draft and
          review the current hardware before applying.
        </p>
      )}
      {separate ? (
        indexes.map((fanIndex) => {
          const entry = entries.find((fan) => fan.fanIndex === fanIndex);
          return (
            <div
              key={fanIndex}
              role="group"
              aria-label={`Fan ${fanIndex + 1} configuration`}
              className="space-y-2 border-t pt-3"
            >
              <p className="text-sm font-medium">Fan {fanIndex + 1}</p>
              {!entry && (
                <p className="text-xs text-muted-foreground">
                  No applied configuration is known for this fan. Choose a mode
                  to include it.
                </p>
              )}
              <SettingEditor
                gpu={gpu}
                fanIndex={fanIndex}
                setting={entry?.setting ?? { mode: "automatic" }}
                known={entry != null}
                busy={blocked}
                draft={draft != null}
                canApply={!invalid}
                onChange={(setting) =>
                  change({
                    mode: "perFan",
                    fans: [
                      ...entries.filter((fan) => fan.fanIndex !== fanIndex),
                      { fanIndex, setting },
                    ].sort((a, b) => a.fanIndex - b.fanIndex),
                  })
                }
                onApply={(setting) =>
                  apply({
                    ...current,
                    fans: [
                      ...entries.filter((fan) => fan.fanIndex !== fanIndex),
                      { fanIndex, setting },
                    ],
                  })
                }
              />
            </div>
          );
        })
      ) : (
        <SettingEditor
          gpu={gpu}
          setting={current}
          busy={blocked}
          draft={draft != null}
          onRecall={recall}
          onChange={change}
          onApply={apply}
        />
      )}
      {(separate || current.mode !== "curve") && (
        <Button
          size="sm"
          disabled={blocked || !draft || invalid}
          onClick={() => apply(current)}
        >
          Apply cooling
        </Button>
      )}
      {invalidFixed && (
        <p role="alert">
          Enter a fixed target within every targeted fan's supported speed
          range.
        </p>
      )}
      <Dialog open={joining} onOpenChange={setJoining}>
        <DialogContent>
          <DialogTitle>Choose a fan configuration</DialogTitle>
          <DialogDescription>
            The fans have different settings. Choose which configuration all
            fans should share. This stays unapplied until Apply.
          </DialogDescription>
          {entries.map((fan) => (
            <Button
              key={fan.fanIndex}
              variant="outline"
              onClick={() => join(fan.setting)}
            >
              Keep Fan {fan.fanIndex + 1}
            </Button>
          ))}
          <Button variant="ghost" onClick={() => setJoining(false)}>
            Cancel
          </Button>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function SettingEditor({
  gpu,
  fanIndex,
  setting,
  busy,
  draft = true,
  known = true,
  canApply = true,
  onRecall,
  onChange,
  onApply,
}: {
  gpu: GpuPerformanceDevice;
  fanIndex?: number;
  setting: FanSettingDraft;
  busy: boolean;
  draft?: boolean;
  known?: boolean;
  canApply?: boolean;
  onRecall?: () => void;
  onChange: (setting: FanSettingDraft) => void;
  onApply: (setting: FanSettingDraft) => void;
}) {
  const support = useFanCurveSupport(gpu, fanIndex);
  const remembered = gpu.cooling.control?.rememberedCurve;
  const rememberedSetting =
    remembered?.arrangement === "shared"
      ? remembered.setting
      : remembered?.fans.find((fan) => fan.fanIndex === fanIndex)?.setting;
  const rememberedPoints =
    rememberedSetting?.mode === "curve" ? rememberedSetting.points : undefined;
  return (
    <div className="space-y-3">
      <label className="flex items-center gap-2 text-sm">
        Control mode
        <select
          aria-label="Control mode"
          value={known ? setting.mode : ""}
          disabled={busy}
          onChange={(event) => {
            if (event.target.value === "automatic")
              onChange({ mode: "automatic" });
            if (event.target.value === "fixed")
              onChange({ mode: "fixed", speedPercent: null });
            if (event.target.value === "curve") {
              if (onRecall) onRecall();
              else
                onChange({
                  mode: "curve",
                  points:
                    rememberedPoints ??
                    (support?.status === "available" ? support.starter : []),
                });
            }
          }}
        >
          {!known && (
            <option value="" disabled>
              Choose mode
            </option>
          )}
          <option value="automatic">Automatic</option>
          <option value="fixed">Fixed</option>
          <option
            value="curve"
            disabled={
              !onRecall && !rememberedPoints && support?.status !== "available"
            }
          >
            Curve
          </option>
        </select>
      </label>
      {setting.mode === "fixed" && (
        <Input
          type="number"
          aria-label="Fixed target (%)"
          step={1}
          value={setting.speedPercent ?? ""}
          disabled={busy}
          onChange={(event) =>
            onChange({
              mode: "fixed",
              speedPercent:
                event.currentTarget.value === ""
                  ? null
                  : event.currentTarget.valueAsNumber,
            })
          }
        />
      )}
      {(setting.mode === "curve" ||
        (fanIndex == null && setting.mode === "automatic")) && (
        <FanCurveSection
          gpu={gpu}
          fanIndex={fanIndex}
          draft={setting.mode === "curve" && draft ? setting.points : null}
          busy={busy || !canApply}
          onDraftChange={(points) => {
            if (points) onChange({ mode: "curve", points });
          }}
          onApply={(points) => onApply({ mode: "curve", points })}
        />
      )}
    </div>
  );
}
