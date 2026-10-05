import { useId, useState, type FormEvent } from "react";
import { Bookmark, ChevronDown, LoaderCircle } from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import type { GpuProfileWorkflow } from "@/features/gpus/hooks/use-gpu-profiles";
import type { TuningEdits } from "@/features/gpus/hooks/use-tuning-edits";
import {
  compareProfile,
  correctedSettings,
  PROFILE_SETTING_KEYS,
  profileSaveBlocker,
} from "@/features/gpus/lib/tuning-profiles";
import type {
  GpuPerformanceDevice,
  GpuProfileSettings,
  GpuTuningProfile,
  ProfileRangeCorrection,
  ProfileSettingKey,
} from "@/features/gpus/types";
import type { PerformanceMutationAction } from "./gpu-tuning-screen";

/**
 * Applies the saved profile, first saving any reviewed corrections to it. Resolves with proposed
 * range corrections when the profile needs review before applying.
 */
type ApplyProfile = (
  profile: GpuTuningProfile,
  corrections?: ProfileRangeCorrection[],
) => Promise<ProfileRangeCorrection[] | undefined>;

/**
 * The state behind one GPU's profile selector, shared by the selector in the GPU's header and
 * the feedback shown above its controls. Null without a profile workflow.
 */
export function useGpuProfileSelector({
  gpu,
  workflow,
  edits,
  appliedDefaults,
  readError,
  pendingAction,
  profileWritePending = false,
  applyDisabled = false,
  onApply,
}: {
  gpu: GpuPerformanceDevice;
  workflow?: GpuProfileWorkflow;
  edits: TuningEdits;
  appliedDefaults: ProfileSettingKey[];
  readError: string | null;
  pendingAction: PerformanceMutationAction | null;
  /** A write to the GPU's profile settings is in progress outside the pending action. */
  profileWritePending?: boolean;
  applyDisabled?: boolean;
  onApply?: ApplyProfile;
}) {
  const [review, setReview] = useState<{
    profile: GpuTuningProfile;
    corrections: ProfileRangeCorrection[];
  } | null>(null);
  const [correctedNotice, setCorrectedNotice] = useState<string | null>(null);
  const [adjusting, setAdjusting] = useState(false);
  const [adjustError, setAdjustError] = useState<string | null>(null);
  const [switchRequest, setSwitchRequest] = useState<{
    profile: GpuTuningProfile;
    current: GpuTuningProfile;
    gpu: GpuPerformanceDevice;
  } | null>(null);
  const [switching, setSwitching] = useState(false);
  const [switchError, setSwitchError] = useState<string | null>(null);
  if (!workflow) return null;
  const uuid = gpu.stableId?.trim() ? gpu.stableId : null;
  const profiles =
    workflow.query.data?.filter((profile) => profile.gpuId === uuid) ?? [];
  const current = (uuid && workflow.stateOf(uuid)?.currentProfile) || null;
  // Another user's current profile is shown by name only; its values aren't visible.
  const ownCurrent = profiles.find((profile) => profile.id === current?.id);
  const comparison =
    ownCurrent && !readError
      ? compareProfile(gpu, ownCurrent, appliedDefaults)
      : null;
  const profileMutationPending =
    profileWritePending ||
    (pendingAction?.gpuId === gpu.id &&
      pendingAction.kind !== "setFanSpeed" &&
      pendingAction.kind !== "setFanModeAuto");
  const saving =
    workflow.saveAsNew.isPending || workflow.saveToCurrent.isPending;
  const profilesReady = !workflow.query.isPending && !workflow.query.error;
  const saveBlocker =
    readError ??
    (["power", "graphics", "memory"].some((key) => key in edits)
      ? "Apply or discard unapplied power-limit or clock-offset changes before saving a profile."
      : profileSaveBlocker(gpu)) ??
    (uuid && !workflow.stateOf(uuid)
      ? "Saving is unavailable until the GPU's profile state is read from the Prism daemon."
      : null);
  const applyBlocker =
    Object.keys(edits).length > 0
      ? "Apply or discard outstanding tuning changes before applying a profile."
      : null;
  const applicationAvailable =
    !!onApply &&
    !review &&
    !readError &&
    !applyBlocker &&
    !applyDisabled &&
    !profileMutationPending &&
    !saving &&
    profilesReady;
  const canApply = applicationAvailable && !switchRequest && !switching;
  const canSwitch =
    applicationAvailable &&
    !!switchRequest &&
    current?.id === switchRequest.current.id &&
    !switching;
  const canSave =
    !saveBlocker && !profileMutationPending && !saving && profilesReady;
  const canAdjust =
    !!review &&
    Object.keys(edits).length === 0 &&
    !applyDisabled &&
    !profileMutationPending &&
    !adjusting;

  async function applyProfile(profile: GpuTuningProfile) {
    if (!onApply) return;
    setCorrectedNotice(null);
    const corrections = await onApply(profile);
    void workflow!.refresh();
    if (corrections?.length) setReview({ profile, corrections });
  }
  async function choose(profile: GpuTuningProfile) {
    if (!canApply) return;
    if (ownCurrent && comparison?.changed && profile.id !== ownCurrent.id) {
      setSwitchError(null);
      setSwitchRequest({ profile, current: ownCurrent, gpu });
      return;
    }
    await applyProfile(profile);
  }
  async function switchProfile(save: boolean) {
    if (!canSwitch || !switchRequest || (save && !canSave)) return;
    setSwitching(true);
    setSwitchError(null);
    try {
      if (save)
        await workflow!.saveToCurrent.mutateAsync({ gpu: switchRequest.gpu });
      await applyProfile(switchRequest.profile);
      setSwitchRequest(null);
    } catch (error) {
      setSwitchError(error instanceof Error ? error.message : String(error));
    } finally {
      setSwitching(false);
    }
  }
  // Approved corrections are saved first and stay saved whatever the GPU later reports.
  async function adjustAndApply() {
    if (!canAdjust || !review || !onApply) return;
    setAdjusting(true);
    setAdjustError(null);
    let next: ProfileRangeCorrection[] | undefined;
    try {
      next = await onApply(review.profile, review.corrections);
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
    void workflow!.refresh();
    setReview(next?.length ? { profile: corrected, corrections: next } : null);
    setCorrectedNotice(
      `Saved the approved corrections to "${corrected.name}". The saved profile does not confirm GPU settings; check each setting's application outcome.`,
    );
  }

  return {
    gpu,
    uuid,
    workflow,
    profiles,
    current,
    ownCurrent,
    comparison,
    saveBlocker,
    applyBlocker,
    canApply,
    canSave,
    canAdjust,
    canSwitch,
    switchRequest,
    switching,
    switchError,
    switchProfile,
    cancelSwitch() {
      if (switching) return;
      setSwitchRequest(null);
      setSwitchError(null);
    },
    review,
    adjusting,
    adjustError,
    correctedNotice,
    edits,
    choose,
    adjustAndApply,
    closeReview() {
      setReview(null);
      setAdjustError(null);
    },
  };
}

export type GpuProfileSelectorState = NonNullable<
  ReturnType<typeof useGpuProfileSelector>
>;

/** The profile selector for a GPU's header: its current profile, and a menu to apply or save. */
export function GpuProfileSelector({
  selector,
}: {
  selector: GpuProfileSelectorState;
}) {
  const { gpu, uuid } = selector;
  const label = `${gpu.name} profiles (${uuid ?? gpu.id})`;
  // The selector sits inside the panel's <summary>; its clicks must not fold the panel.
  const keepPanelOpen = (event: React.MouseEvent) => {
    if (event.currentTarget.contains(event.target as Node))
      event.preventDefault();
  };
  if (!uuid)
    return (
      <div role="group" aria-label={label} className="max-w-xs">
        <p role="status" className="text-xs text-muted-foreground">
          Profiles unavailable: this GPU reports no stable identity (UUID), so a
          profile can't be tied to it.
        </p>
      </div>
    );
  return (
    <div role="group" aria-label={label} onClick={keepPanelOpen}>
      <ProfileMenu selector={selector} />
    </div>
  );
}

function ProfileMenu({ selector }: { selector: GpuProfileSelectorState }) {
  const [saveAsOpen, setSaveAsOpen] = useState(false);
  const {
    gpu,
    workflow,
    profiles,
    current,
    ownCurrent,
    comparison,
    saveBlocker,
    applyBlocker,
    canApply,
    canSave,
  } = selector;
  return (
    <>
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="max-w-56"
          >
            <Bookmark />
            <span className="sr-only">Profile: </span>
            <span className="truncate">
              {current?.name ?? "No current profile"}
            </span>
            {comparison?.changed && (
              <>
                <span
                  aria-hidden
                  title="Changes not saved to profile"
                  className="size-2 shrink-0 rounded-full bg-amber-500"
                />
                <span className="sr-only">Changes not saved to profile</span>
              </>
            )}
            <ChevronDown />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-80">
          <DropdownMenuLabel>Profiles for this GPU</DropdownMenuLabel>
          {current && !ownCurrent && (
            <MenuNote>
              The current profile "{current.name}" belongs to another user.
            </MenuNote>
          )}
          {profiles.length === 0 && (
            <MenuNote>No saved profiles for this GPU yet.</MenuNote>
          )}
          {profiles.length > 0 && applyBlocker && (
            <MenuNote role="status">{applyBlocker}</MenuNote>
          )}
          <DropdownMenuRadioGroup value={ownCurrent?.id ?? ""}>
            {profiles.map((profile) => (
              <DropdownMenuRadioItem
                key={profile.id}
                value={profile.id}
                disabled={!canApply}
                onSelect={() => void selector.choose(profile)}
                className="flex-col items-start gap-0.5"
              >
                <span className="font-medium">{profile.name}</span>{" "}
                <span className="text-xs text-muted-foreground">
                  {describeSettings(profile.settings)}
                </span>
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
          {ownCurrent && (!comparison || comparison.unavailable) && (
            <MenuNote>
              Profile comparison is unavailable until current settings can be
              read.
            </MenuNote>
          )}
          <DropdownMenuSeparator />
          {saveBlocker && <MenuNote role="status">{saveBlocker}</MenuNote>}
          {ownCurrent && (
            <DropdownMenuItem
              disabled={!canSave}
              onSelect={() => workflow.saveToCurrent.mutate({ gpu })}
            >
              Save to "{ownCurrent.name}"
            </DropdownMenuItem>
          )}
          <DropdownMenuItem
            disabled={!canSave}
            onSelect={() => setSaveAsOpen(true)}
          >
            Save as new…
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <SaveAsNewDialog
        selector={selector}
        open={saveAsOpen}
        onOpenChange={setSaveAsOpen}
      />
      <SaveBeforeSwitchDialog selector={selector} />
    </>
  );
}

function SaveBeforeSwitchDialog({
  selector,
}: {
  selector: GpuProfileSelectorState;
}) {
  const { switchRequest, switching, switchError, canSwitch, canSave } =
    selector;
  return (
    <Dialog
      open={!!switchRequest}
      onOpenChange={(open) => !open && selector.cancelSwitch()}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Switch profile?</DialogTitle>
          <DialogDescription>
            There are changes not saved to "{switchRequest?.current.name}". Save
            them before applying "{switchRequest?.profile.name}"?
          </DialogDescription>
        </DialogHeader>
        {selector.saveBlocker && (
          <p role="status" className="text-sm text-muted-foreground">
            {selector.saveBlocker}
          </p>
        )}
        {switchRequest && selector.current?.id !== switchRequest.current.id && (
          <p role="status" className="text-sm text-muted-foreground">
            The current profile changed. Cancel and choose a profile again.
          </p>
        )}
        {switchError && (
          <p role="alert" className="text-sm text-destructive">
            {switchError}
          </p>
        )}
        <DialogFooter className="sm:flex-wrap">
          <Button
            type="button"
            variant="outline"
            disabled={switching}
            onClick={selector.cancelSwitch}
          >
            Cancel
          </Button>
          <Button
            type="button"
            variant="outline"
            disabled={!canSwitch}
            onClick={() => void selector.switchProfile(false)}
          >
            Switch without saving
          </Button>
          <Button
            type="button"
            disabled={!canSwitch || !canSave}
            onClick={() => void selector.switchProfile(true)}
          >
            {switching && <LoaderCircle className="animate-spin" />}
            Save to "{switchRequest?.current.name}" and switch
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function MenuNote({
  children,
  role,
}: {
  children: React.ReactNode;
  role?: "status";
}) {
  return (
    <p role={role} className="px-2 py-1.5 text-xs text-muted-foreground">
      {children}
    </p>
  );
}

function SaveAsNewDialog({
  selector,
  open,
  onOpenChange,
}: {
  selector: GpuProfileSelectorState;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const id = useId();
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const { gpu, uuid, workflow, canSave } = selector;
  const pending = workflow.saveAsNew.isPending;
  function changeOpen(next: boolean) {
    if (!next) {
      setName("");
      setError(null);
    }
    onOpenChange(next);
  }
  async function save(event: FormEvent) {
    event.preventDefault();
    if (!name.trim() || !canSave || !uuid) return;
    setError(null);
    try {
      await workflow.saveAsNew.mutateAsync({ gpu, name: name.trim() });
      changeOpen(false);
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    }
  }
  return (
    <Dialog open={open} onOpenChange={changeOpen}>
      <DialogContent>
        <form className="grid gap-4" onSubmit={(event) => void save(event)}>
          <DialogHeader>
            <DialogTitle>Save as new profile</DialogTitle>
            <DialogDescription>
              Saves the power limit and clock offsets applied to {gpu.name} as a
              new profile, which becomes its current profile.
            </DialogDescription>
          </DialogHeader>
          <div className="grid gap-2">
            <label htmlFor={`${id}-name`} className="text-sm">
              Profile name
            </label>
            <Input
              id={`${id}-name`}
              placeholder="e.g. Quiet"
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          </div>
          {selector.saveBlocker && (
            <p role="status" className="text-sm text-muted-foreground">
              {selector.saveBlocker}
            </p>
          )}
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
          <DialogFooter>
            <DialogClose asChild>
              <Button type="button" variant="outline">
                Cancel
              </Button>
            </DialogClose>
            <Button type="submit" disabled={!name.trim() || !canSave}>
              {pending && <LoaderCircle className="animate-spin" />}
              Save
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** What choosing or saving a profile reports, shown above the GPU's tuning controls. */
export function GpuProfileFeedback({
  selector,
}: {
  selector: GpuProfileSelectorState;
}) {
  const { review, uuid, workflow, ownCurrent, comparison } = selector;
  const saveError =
    workflow.saveToCurrent.variables?.gpu.stableId === uuid
      ? workflow.saveToCurrent.error
      : null;
  if (
    !review &&
    !selector.correctedNotice &&
    !saveError &&
    !comparison?.reportedChanged
  )
    return null;
  return (
    <div className="space-y-3">
      {ownCurrent && comparison?.reportedChanged && (
        <section
          aria-label="Current profile saved values"
          className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-amber-500/40 bg-amber-500/5 p-4"
        >
          <p className="text-sm">
            Saved in "{ownCurrent.name}":{" "}
            {describeSettings(ownCurrent.settings)}
          </p>
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={!selector.canSave}
            onClick={() => workflow.saveToCurrent.mutate({ gpu: selector.gpu })}
          >
            Save to "{ownCurrent.name}"
          </Button>
        </section>
      )}
      {saveError && (
        <Alert variant="destructive">
          <AlertTitle>Failed to save the profile</AlertTitle>
          <AlertDescription>{saveError.message}</AlertDescription>
        </Alert>
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
              const { label, unit } = SETTING_DISPLAY[correction.setting];
              return (
                <li key={correction.setting}>
                  {`${label}: saved ${correction.savedValue} ${unit} → current ${correction.bound} ${correction.proposedValue} ${unit}`}
                </li>
              );
            })}
          </ul>
          {Object.keys(selector.edits).length > 0 && (
            <p role="status" className="text-sm text-muted-foreground">
              Apply or discard outstanding tuning changes before adjusting the
              profile.
            </p>
          )}
          {selector.adjustError && (
            <p role="alert" className="text-sm text-destructive">
              The corrected profile could not be saved, so no GPU settings were
              changed. {selector.adjustError}
            </p>
          )}
          <div className="flex flex-wrap gap-2">
            <Button
              type="button"
              disabled={!selector.canAdjust}
              onClick={() => void selector.adjustAndApply()}
            >
              {selector.adjusting && <LoaderCircle className="animate-spin" />}
              Adjust profile and apply
            </Button>
            <Button
              type="button"
              variant="outline"
              disabled={selector.adjusting}
              onClick={selector.closeReview}
            >
              Cancel
            </Button>
          </div>
        </section>
      )}
      {selector.correctedNotice && (
        <p role="status" className="text-sm text-muted-foreground">
          {selector.correctedNotice}
        </p>
      )}
    </div>
  );
}

const SETTING_DISPLAY = {
  powerLimitWatts: { label: "Power limit", unit: "W" },
  graphicsOffsetMHz: { label: "Graphics clock offset", unit: "MHz" },
  memoryOffsetMHz: { label: "Memory clock offset", unit: "MHz" },
};

const SHORT_LABEL = {
  powerLimitWatts: "Power limit",
  graphicsOffsetMHz: "Graphics offset",
  memoryOffsetMHz: "Memory offset",
};

function describeSettings(settings: GpuProfileSettings) {
  return PROFILE_SETTING_KEYS.flatMap((key) => {
    const setting = settings[key];
    if (!setting) return [];
    const value =
      setting.kind === "default"
        ? "Use default"
        : `${setting.value} ${SETTING_DISPLAY[key].unit}`;
    return [`${SHORT_LABEL[key]} ${value}`];
  }).join(" · ");
}

/** Profile loading problems, and profiles whose owning GPU is absent. */
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
              <Button disabled>Apply profile</Button>
            </div>
          ))}
        </section>
      )}
    </>
  );
}
