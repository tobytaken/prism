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
  profileSaveBlocker,
} from "@/features/gpus/lib/tuning-profiles";
import type {
  GpuPerformanceDevice,
  GpuProfileSettings,
  GpuTuningProfile,
  ProfileSettingKey,
} from "@/features/gpus/types";

export function GpuProfilePanel({
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
  onLoad?: (profile: GpuTuningProfile, settings: GpuProfileSettings) => void;
  loadDisabled?: boolean;
}) {
  const id = useId();
  const [name, setName] = useState("");
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
    !!gpu.stableId &&
    !readError &&
    !loadBlocker &&
    !loadDisabled &&
    !profileMutationPending &&
    !workflow.save.isPending &&
    !workflow.query.isPending &&
    !workflow.query.error;
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
            onChange={(event) =>
              gpu.stableId && workflow.select(gpu.stableId, event.target.value)
            }
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
                if (canLoad && selected) onLoad(selected, selected.settings);
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
            if (canSave)
              workflow.save.mutate({
                gpuId: gpu.id,
                name: name.trim(),
                ...(appliedDefaults.length > 0 && { appliedDefaults }),
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
      {selected && (
        <div className="flex flex-wrap gap-3 text-sm">
          {Object.entries(selected.settings).map(([key, setting]) => (
            <Badge key={key} variant="secondary">
              {setting.kind === "default"
                ? "Use default"
                : `${setting.value} ${key === "powerLimitWatts" ? "W" : "MHz"}`}
              <span className="ml-1 text-muted-foreground">
                {settingLabels[key as keyof GpuProfileSettings]}
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
      {workflow.save.error && workflow.save.variables?.gpuId === gpu.id && (
        <p role="alert" className="text-sm text-destructive">
          {workflow.save.error.message}
        </p>
      )}
    </section>
  );
}

const settingLabels = {
  powerLimitWatts: "Power limit",
  graphicsOffsetMHz: "Graphics offset",
  memoryOffsetMHz: "Memory offset",
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
