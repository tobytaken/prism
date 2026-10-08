import { useContext } from "react";
import { UNSAFE_DataRouterContext, useBlocker } from "react-router-dom";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

interface Props {
  affected: { id: string; name: string; waiting: string | null }[];
  busy: boolean;
  onDiscard: () => void;
}

export function DiscardTuningDialog(props: Props) {
  // Standalone screens have no route to leave; the app's data router owns navigation.
  const router = useContext(UNSAFE_DataRouterContext);
  return router ? <NavigationDialog {...props} /> : null;
}

function NavigationDialog({ affected, busy, onDiscard }: Props) {
  const blocker = useBlocker(
    ({ currentLocation, nextLocation }) =>
      affected.length > 0 && currentLocation.pathname !== nextLocation.pathname,
  );
  return (
    <Dialog
      open={blocker.state === "blocked"}
      onOpenChange={(open) => {
        if (!open && blocker.state === "blocked") blocker.reset();
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Discard unapplied tuning changes?</DialogTitle>
          <DialogDescription>
            Leaving Performance discards your unapplied tuning changes on these
            GPUs.
          </DialogDescription>
        </DialogHeader>
        <ul className="space-y-2 text-sm">
          {affected.map((gpu) => (
            <li key={gpu.id}>
              <p className="font-medium">{gpu.name}</p>
              {gpu.waiting && (
                <p className="text-amber-600 dark:text-amber-400">
                  {gpu.waiting}
                </p>
              )}
            </li>
          ))}
        </ul>
        <DialogFooter>
          <Button
            variant="outline"
            onClick={() => blocker.state === "blocked" && blocker.reset()}
          >
            Cancel
          </Button>
          <Button
            disabled={busy}
            onClick={() => {
              onDiscard();
              if (blocker.state === "blocked") blocker.proceed();
            }}
          >
            Continue
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
