import { useId, useState, type FormEvent } from "react";
import { LoaderCircle } from "lucide-react";

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
import type { CurrentProfile, GpuTuningProfile } from "@/features/gpus/types";

/** Manages only the caller's profiles for the GPU whose selector opened it. */
export function GpuManageProfilesDialog({
  profiles,
  current,
  currentProfileKnown,
  workflow,
  onOpenChange,
}: {
  profiles: GpuTuningProfile[];
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
  const pending = workflow.rename.isPending || workflow.remove.isPending;
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
    if (!deleting || pending || !currentProfileKnown) return;
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
        {profiles.length === 0 && (
          <p className="text-sm text-muted-foreground">
            No saved profiles for this GPU yet.
          </p>
        )}
        {!currentProfileKnown && (
          <p role="status" className="text-sm text-muted-foreground">
            Deletion is unavailable until this GPU's current profile can be
            read. Try again once the Prism daemon is available.
          </p>
        )}
        <ul className="max-h-80 space-y-3 overflow-y-auto">
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
                    disabled={pending || !currentProfileKnown}
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
                Current tuning values will stay unchanged.
              </DialogDescription>
            </DialogHeader>
            {error && (
              <p role="alert" className="text-sm text-destructive">
                {error}
              </p>
            )}
            {!currentProfileKnown && (
              <p role="status" className="text-sm text-muted-foreground">
                The GPU's current profile could not be read. Try deleting again
                once the Prism daemon is available.
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
                disabled={pending || !currentProfileKnown}
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
