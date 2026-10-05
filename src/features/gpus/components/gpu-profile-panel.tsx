import { useId, useState } from "react";
import { Bookmark, LoaderCircle } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import type { GpuProfileWorkflow } from "@/features/gpus/hooks/use-gpu-profiles";
import type { TuningEdits } from "@/features/gpus/hooks/use-tuning-edits";
import type { PerformanceMutationAction } from "./gpu-tuning-screen";
import {
  compareProfile,
  correctedSettings,
  profileSaveBlocker,
} from "@/features/gpus/lib/tuning-profiles";
import type {
  GpuPerformanceDevice,
  GpuProfileSettings,
  GpuTuningProfile,
  ProfileRangeCorrection,
  ProfileSettingKey,
} from "@/features/gpus/types";

export function GpuProfilePanel(
  props: Parameters<typeof GpuProfileControls>[0],
) {
  const { gpu } = props;
  if (gpu.stableId?.trim()) return <GpuProfileControls {...props} />;
  return (
    <section
      aria-label={`${gpu.name} profiles (${gpu.id})`}
      className="rounded-xl border border-dashed border-border p-5 space-y-2"
    >
      <div className="flex items-center gap-2 text-sm font-semibold">
        <Bookmark className="size-4 text-muted-foreground" />
        GPU tuning profiles unavailable
      </div>
      <p role="status" className="text-sm text-muted-foreground">
        This GPU reports no stable identity (UUID), so a profile can't be tied
        to it. Profiles can't be saved or applied for this GPU.
      </p>
    </section>
  );
}

function GpuProfileControls({
  gpu,
  workflow,
  edits,
  appliedDefaults,
  readError,
  pendingAction,
  reapplyPending = false,
  onLoad,
  loadDisabled = false,
}: {
  gpu: GpuPerformanceDevice;
  workflow: GpuProfileWorkflow;
  edits: TuningEdits;
  appliedDefaults: ProfileSettingKey[];
  readError: string | null;
  pendingAction: PerformanceMutationAction | null;
  reapplyPending?: boolean;
  /**
   * Applies the saved profile, first saving any reviewed corrections to it. Resolves with
   * proposed range corrections when the profile needs review before applying.
   */
  onLoad?: (
    profile: GpuTuningProfile,
    corrections?: ProfileRangeCorrection[],
  ) => Promise<ProfileRangeCorrection[] | undefined>;
  loadDisabled?: boolean;
}) {
  const id = useId();
  const [name, setName] = useState("");
  const [review, setReview] = useState<{
    profile: GpuTuningProfile;
    corrections: ProfileRangeCorrection[];
  } | null>(null);
  const [correctedNotice, setCorrectedNotice] = useState<string | null>(null);
  const [adjusting, setAdjusting] = useState(false);
  const [adjustError, setAdjustError] = useState<string | null>(null);
  const profiles =
    workflow.query.data?.filter((profile) => profile.gpuId === gpu.stableId) ??
    [];
  const selected = profiles.find(
    (profile) => profile.id === workflow.selectedIds[gpu.stableId ?? ""],
  );
  const comparison =
    selected && !readError
      ? compareProfile(gpu, selected, appliedDefaults)
      : null;
  const profileMutationPending =
    reapplyPending ||
    (pendingAction?.gpuId === gpu.id &&
      pendingAction.kind !== "setFanSpeed" &&
      pendingAction.kind !== "setFanModeAuto");
  const blocker =
    readError ??
    (["power", "graphics", "memory"].some((key) => key in edits)
      ? "Apply or discard unapplied power-limit or clock-offset changes before saving a profile."
      : profileSaveBlocker(gpu));
  const canSave =
    !!name.trim() &&
    !blocker &&
    !profileMutationPending &&
    !workflow.save.isPending &&
    !workflow.query.isPending &&
    !workflow.query.error;
  const loadBlocker =
    Object.keys(edits).length > 0
      ? "Apply or discard outstanding tuning changes before loading a profile."
      : null;
  const canLoad =
    !!selected &&
    !review &&
    !!gpu.stableId &&
    !readError &&
    !loadBlocker &&
    !loadDisabled &&
    !profileMutationPending &&
    !workflow.save.isPending &&
    !workflow.query.isPending &&
    !workflow.query.error;
  const canAdjust =
    !!review &&
    Object.keys(edits).length === 0 &&
    !loadDisabled &&
    !profileMutationPending &&
    !adjusting;
  async function load(profile: GpuTuningProfile) {
    const corrections = await onLoad?.(profile);
    if (corrections?.length) setReview({ profile, corrections });
  }
  // Approved corrections are saved first and stay saved whatever the GPU later reports.
  async function adjustAndApply() {
    if (!canAdjust || !review || !onLoad) return;
    setAdjusting(true);
    setAdjustError(null);
    let next: ProfileRangeCorrection[] | undefined;
    try {
      next = await onLoad(review.profile, review.corrections);
    } catch (error) {
      setAdjustError(error instanceof Error ? error.message : String(error));
      return;
    } finally {
      setAdjusting(false);
    }
    const corrected = {
      ...review.profile,
      settings: correctedSettings(review.profile.settings, review.corrections),
    };
    void workflow.refresh();
    setReview(next?.length ? { profile: corrected, corrections: next } : null);
    setCorrectedNotice(
      `Saved the approved corrections to "${corrected.name}". The saved profile does not confirm GPU settings; check each setting's application outcome.`,
    );
  }
  function closeReview() {
    setReview(null);
    setAdjustError(null);
  }
  return (
    <section
      aria-label={`${gpu.name} profiles (${gpu.stableId ?? gpu.id})`}
      className="rounded-xl border border-border bg-card/60 p-5 space-y-4"
    >
      <div className="flex items-center gap-2 text-sm font-semibold">
        <Bookmark className="size-4 text-primary" />
        GPU tuning profiles
      </div>
      <p className="text-sm text-muted-foreground">
        Save current power and clock offsets for this GPU.
      </p>
      <div className="grid gap-4 md:grid-cols-2">
        <div className="space-y-2">
          <label htmlFor={`${id}-select`} className="text-sm">
            Saved profile
          </label>
          <select
            id={`${id}-select`}
            className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
            value={selected?.id ?? ""}
            onChange={(event) => {
              closeReview();
              setCorrectedNotice(null);
              if (gpu.stableId)
                workflow.select(gpu.stableId, event.target.value);
            }}
          >
            <option value="">Select a profile</option>
            {profiles.map((profile) => (
              <option key={profile.id} value={profile.id}>
                {profile.name}
              </option>
            ))}
          </select>
          {onLoad && (
            <Button
              type="button"
              disabled={!canLoad}
              onClick={() => {
                if (!canLoad || !selected) return;
                setCorrectedNotice(null);
                void load(selected);
              }}
            >
              Load profile
            </Button>
          )}
        </div>
        <form
          className="space-y-2"
          onSubmit={(event) => {
            event.preventDefault();
            if (canSave && gpu.stableId)
              workflow.save.mutate({
                gpuId: gpu.stableId,
                name: name.trim(),
              });
          }}
        >
          <label htmlFor={`${id}-name`} className="text-sm">
            Profile name
          </label>
          <div className="flex gap-2">
            <Input
              id={`${id}-name`}
              placeholder="e.g. Quiet"
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
            <Button type="submit" disabled={!canSave}>
              {workflow.save.isPending && (
                <LoaderCircle className="animate-spin" />
              )}
              Save profile
            </Button>
          </div>
        </form>
      </div>
      {blocker && (
        <p className="text-sm text-muted-foreground" role="status">
          {blocker}
        </p>
      )}
      {onLoad && selected && loadBlocker && (
        <p role="status" className="text-sm text-muted-foreground">
          {loadBlocker}
        </p>
      )}
      {review && (
        <section
          aria-label="Review profile corrections"
          className="space-y-3 rounded-lg border border-primary/40 bg-primary/5 p-4"
        >
          <h3 className="text-sm font-semibold">Review profile corrections</h3>
          <p className="text-sm text-muted-foreground">
            Some values saved in "{review.profile.name}" are outside this GPU's
            current allowed ranges. Adjusting saves these values to the profile
            and then applies it. Cancel leaves the profile and GPU unchanged.
          </p>
          <ul className="list-disc space-y-1 pl-5 text-sm">
            {review.corrections.map((correction) => {
              const { label, unit } = settingDisplay[correction.setting];
              return (
                <li key={correction.setting}>
                  {`${label}: saved ${correction.savedValue} ${unit} → current ${correction.bound} ${correction.proposedValue} ${unit}`}
                </li>
              );
            })}
          </ul>
          {Object.keys(edits).length > 0 && (
            <p role="status" className="text-sm text-muted-foreground">
              Apply or discard outstanding tuning changes before adjusting the
              profile.
            </p>
          )}
          {adjustError && (
            <p role="alert" className="text-sm text-destructive">
              The corrected profile could not be saved, so no GPU settings were
              changed. {adjustError}
            </p>
          )}
          <div className="flex flex-wrap gap-2">
            <Button
              type="button"
              disabled={!canAdjust}
              onClick={() => void adjustAndApply()}
            >
              {adjusting && <LoaderCircle className="animate-spin" />}
              Adjust profile and apply
            </Button>
            <Button
              type="button"
              variant="outline"
              disabled={adjusting}
              onClick={closeReview}
            >
              Cancel
            </Button>
          </div>
        </section>
      )}
      {correctedNotice && (
        <p role="status" className="text-sm text-muted-foreground">
          {correctedNotice}
        </p>
      )}
      {selected && (
        <div className="flex flex-wrap gap-3 text-sm">
          {Object.entries(selected.settings).map(([key, setting]) => (
            <Badge key={key} variant="secondary">
              {setting.kind === "default"
                ? "Use default"
                : `${setting.value} ${settingDisplay[key as keyof GpuProfileSettings].unit}`}
              <span className="ml-1 text-muted-foreground">
                {settingDisplay[key as keyof GpuProfileSettings].shortLabel}
              </span>
            </Badge>
          ))}
        </div>
      )}
      {selected && (
        <div aria-live="polite" className="space-y-2">
          {comparison?.changed && (
            <Badge variant="outline">Changes not saved to profile</Badge>
          )}
          {(readError || comparison?.unavailable) && (
            <p className="text-sm text-muted-foreground">
              Profile comparison is unavailable until current settings can be
              read.
            </p>
          )}
        </div>
      )}
      {workflow.save.error &&
        workflow.save.variables?.gpuId === gpu.stableId && (
          <p role="alert" className="text-sm text-destructive">
            {workflow.save.error.message}
          </p>
        )}
    </section>
  );
}

const settingDisplay = {
  powerLimitWatts: {
    label: "Power limit",
    shortLabel: "Power limit",
    unit: "W",
  },
  graphicsOffsetMHz: {
    label: "Graphics clock offset",
    shortLabel: "Graphics offset",
    unit: "MHz",
  },
  memoryOffsetMHz: {
    label: "Memory clock offset",
    shortLabel: "Memory offset",
    unit: "MHz",
  },
};

export function GpuProfileAvailability({
  workflow,
  gpus,
}: {
  workflow: GpuProfileWorkflow;
  gpus: GpuPerformanceDevice[];
}) {
  const unavailable =
    workflow.query.data?.filter(
      (profile) => !gpus.some((gpu) => gpu.stableId === profile.gpuId),
    ) ?? [];
  return (
    <>
      {workflow.query.error && (
        <Alert variant="destructive">
          <AlertTitle>Failed to load GPU tuning profiles</AlertTitle>
          <AlertDescription>{workflow.query.error.message}</AlertDescription>
        </Alert>
      )}
      {unavailable.length > 0 && (
        <section
          aria-label="Unavailable GPU profiles"
          className="rounded-xl border border-dashed border-border p-5 space-y-3"
        >
          <h2 className="text-sm font-semibold">Unavailable GPU profiles</h2>
          {unavailable.map((profile) => (
            <div
              key={profile.id}
              className="flex flex-wrap items-center justify-between gap-2 text-sm"
            >
              <span className="font-medium">{profile.name}</span>
              <span className="text-muted-foreground">
                Owning GPU absent · {profile.gpuId}
              </span>
              <Button disabled>Load profile</Button>
            </div>
          ))}
        </section>
      )}
    </>
  );
}
