import { useId, useState, type FormEvent } from "react";
import { LoaderCircle } from "lucide-react";
import { Link } from "react-router-dom";

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
import { Input } from "@/components/ui/input";
import type { GpuProfileWorkflow } from "@/features/gpus/hooks/use-gpu-profiles";
import { startupProfileWarning } from "@/features/gpus/lib/startup-profiles";
import type {
  GpuPerformanceDevice,
  CurrentProfile,
  GpuTuningProfile,
} from "@/features/gpus/types";

/** Manages only the caller's profiles for the GPU whose selector opened it. */
export function GpuManageProfilesDialog({
  profiles,
  gpuId,
  gpu,
  current,
  currentProfileKnown,
  workflow,
  onOpenChange,
}: {
  profiles: GpuTuningProfile[];
  gpuId: string;
  gpu?: GpuPerformanceDevice;
  current: CurrentProfile | null;
  currentProfileKnown: boolean;
  workflow: GpuProfileWorkflow;
  onOpenChange: (open: boolean) => void;
}) {
  const id = useId();
  const [renaming, setRenaming] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<GpuTuningProfile | null>(null);
  const ownerless =
    workflow.migration.query.data?.profiles.filter(
      (profile) => profile.gpuId === gpuId,
    ) ?? [];
  const startup = workflow.startup;
  const startupProfile = startup.profileOf(gpuId);
  const designated = profiles.find(
    (profile) => profile.id === startupProfile?.id,
  );
  const startupWarning = designated
    ? startupProfileWarning(designated, gpu)
    : null;
  const startupKnown = startup.query.isSuccess;
  const gamesKnown = workflow.games.query.isSuccess;
  const canDelete =
    startupKnown &&
    gamesKnown &&
    (currentProfileKnown || (!gpu && workflow.states.isSuccess));
  const linkedGames = deleting
    ? workflow.games.linkedTo(deleting.id).map((game) => game.name)
    : [];
  const pending =
    workflow.rename.isPending ||
    workflow.remove.isPending ||
    startup.setProfile.isPending;
  async function rename(event: FormEvent) {
    event.preventDefault();
    if (!renaming || !name.trim() || pending) return;
    setError(null);
    try {
      await workflow.rename.mutateAsync({
        profileId: renaming,
        name: name.trim(),
      });
      setRenaming(null);
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    }
  }
  async function remove() {
    if (!deleting || pending || !canDelete) return;
    setError(null);
    try {
      await workflow.remove.mutateAsync({ profileId: deleting.id });
      setDeleting(null);
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    }
  }
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!pending) onOpenChange(open);
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Manage profiles</DialogTitle>
          <DialogDescription>
            Manage your profiles for this GPU.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-2 rounded-lg border p-3">
          <p className="text-sm">
            Startup behavior:{" "}
            {startup.query.data
              ? {
                  off: "Off",
                  startupProfiles: "Startup profiles",
                  restoreLastApplied: "Restore last applied",
                }[startup.query.data.behavior]
              : "Unavailable"}
            .{" "}
            <Link className="underline" to="/settings">
              Settings
            </Link>
          </p>
          <label htmlFor={`${id}-startup`} className="text-sm font-medium">
            Startup profile
          </label>
          <select
            id={`${id}-startup`}
            className="rounded-md border bg-background p-2 text-sm"
            value={startupProfile?.id ?? ""}
            disabled={!startupKnown || pending}
            onChange={(event) =>
              startup.setProfile.mutate({
                gpuId,
                profileId: event.target.value || null,
              })
            }
          >
            <option value="">No startup profile</option>
            {startupProfile &&
              !profiles.some((profile) => profile.id === startupProfile.id) && (
                <option value={startupProfile.id} disabled>
                  {startupProfile.name}{" "}
                  {ownerless.some((profile) => profile.id === startupProfile.id)
                    ? "(read-only, no owner)"
                    : "(another user's profile)"}
                </option>
              )}
            {profiles.map((profile) => (
              <option key={profile.id} value={profile.id}>
                {profile.name}
              </option>
            ))}
          </select>
          <p className="text-xs text-muted-foreground">
            This choice is host-wide; the latest change wins. Changes take
            effect at the next host boot.
          </p>
          <p className="text-sm">
            Link games to your profiles on{" "}
            <Link className="underline" to="/games">
              Games
            </Link>
            .
          </p>
          {startupWarning && (
            <p className="text-sm text-amber-600">{startupWarning}</p>
          )}
          {startup.query.error && (
            <p role="alert" className="text-sm text-destructive">
              {startup.query.error.message}
            </p>
          )}
          {startup.setProfile.error && (
            <p role="alert" className="text-sm text-destructive">
              {startup.setProfile.error.message}
            </p>
          )}
        </div>
        {profiles.length === 0 && ownerless.length === 0 && (
          <p className="text-sm text-muted-foreground">
            No saved profiles for this GPU yet.
          </p>
        )}
        {!canDelete && (
          <p role="status" className="text-sm text-muted-foreground">
            {!startupKnown
              ? "Deletion is unavailable until startup choices can be read. Try again once the Prism daemon is available."
              : !gamesKnown
                ? "Deletion is unavailable until game associations can be read. Try again once the Prism daemon is available."
                : "Deletion is unavailable until this GPU's current profile can be read. Try again once the Prism daemon is available."}
          </p>
        )}
        <ul className="max-h-80 space-y-3 overflow-y-auto">
          {ownerless.map((profile) => (
            <li key={profile.id} className="space-y-2 rounded-lg border p-3">
              <p className="text-sm font-medium">{profile.name}</p>
              <p className="text-xs text-muted-foreground">
                Read-only · This migrated profile has no owner.
              </p>
            </li>
          ))}
          {profiles.map((profile) => (
            <li key={profile.id} className="space-y-2 rounded-lg border p-3">
              <div className="flex items-center justify-between gap-3">
                <div className="min-w-0">
                  <p className="break-words text-sm font-medium">
                    {profile.name}
                  </p>
                  {profile.id === current?.id && (
                    <p className="text-xs text-muted-foreground">
                      Current profile
                    </p>
                  )}
                </div>
                <div className="flex shrink-0 gap-2">
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    disabled={pending}
                    aria-label={`Rename ${profile.name}`}
                    onClick={() => {
                      setRenaming(profile.id);
                      setName(profile.name);
                      setError(null);
                    }}
                  >
                    Rename
                  </Button>
                  <Button
                    type="button"
                    variant="destructive"
                    size="sm"
                    disabled={pending || !canDelete}
                    aria-label={`Delete ${profile.name}`}
                    onClick={() => {
                      setDeleting(profile);
                      setRenaming(null);
                      setError(null);
                    }}
                  >
                    Delete
                  </Button>
                </div>
              </div>
              {renaming === profile.id && (
                <form
                  className="grid gap-2"
                  onSubmit={(event) => void rename(event)}
                >
                  <label htmlFor={`${id}-name`} className="text-sm">
                    Profile name
                  </label>
                  <Input
                    id={`${id}-name`}
                    autoFocus
                    value={name}
                    disabled={pending}
                    onChange={(event) => setName(event.target.value)}
                  />
                  {error && (
                    <p role="alert" className="text-sm text-destructive">
                      {error}
                    </p>
                  )}
                  <div className="flex gap-2">
                    <Button
                      type="submit"
                      size="sm"
                      disabled={pending || !name.trim()}
                    >
                      {pending && <LoaderCircle className="animate-spin" />}Save
                      name
                    </Button>
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      disabled={pending}
                      onClick={() => {
                        setRenaming(null);
                        setError(null);
                      }}
                    >
                      Cancel
                    </Button>
                  </div>
                </form>
              )}
            </li>
          ))}
        </ul>
        <DialogFooter>
          <DialogClose asChild>
            <Button type="button" variant="outline" disabled={pending}>
              Done
            </Button>
          </DialogClose>
        </DialogFooter>
        <Dialog
          open={!!deleting}
          onOpenChange={(open) => {
            if (!open && !pending) {
              setDeleting(null);
              setError(null);
            }
          }}
        >
          <DialogContent role="alertdialog">
            <DialogHeader>
              <DialogTitle>Delete "{deleting?.name}"?</DialogTitle>
              <DialogDescription>
                {deleting?.id === current?.id &&
                  "This is the current profile. After deletion, this GPU will have no current profile. "}
                {deleting?.id === startupProfile?.id &&
                  "This is the startup profile. After deletion, this GPU will have no startup profile. "}
                {!gpu &&
                  "The GPU is absent. Any current-profile designation for this profile will be cleared. "}
                {linkedGames.length > 0 &&
                  `It's linked to ${new Intl.ListFormat("en").format(linkedGames)}. Those links will be removed, and ${linkedGames.length === 1 ? "the game" : "the games"} will leave this GPU unchanged. `}
                Current tuning values will stay unchanged.
              </DialogDescription>
            </DialogHeader>
            {error && (
              <p role="alert" className="text-sm text-destructive">
                {error}
              </p>
            )}
            {!canDelete && (
              <p role="status" className="text-sm text-muted-foreground">
                Profile roles could not be read. Try deleting again once the
                Prism daemon is available.
              </p>
            )}
            <DialogFooter>
              <DialogClose asChild>
                <Button type="button" variant="outline" disabled={pending}>
                  Cancel
                </Button>
              </DialogClose>
              <Button
                type="button"
                variant="destructive"
                disabled={pending || !canDelete}
                onClick={() => void remove()}
              >
                {pending && <LoaderCircle className="animate-spin" />}Delete
                profile
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </DialogContent>
    </Dialog>
  );
}
