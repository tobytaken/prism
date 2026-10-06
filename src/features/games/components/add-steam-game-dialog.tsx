import { LoaderCircle } from "lucide-react";
import { useQuery } from "@tanstack/react-query";

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
import type { GamesWorkflow } from "@/features/games/hooks/use-games";
import type { GameEntry } from "@/features/games/types";

/** Lists the caller's installed Steam apps that aren't on the Games page yet. */
export function AddSteamGameDialog({
  added,
  games,
  onOpenChange,
}: {
  added: GameEntry[];
  games: GamesWorkflow;
  onOpenChange: (open: boolean) => void;
}) {
  const apps = useQuery({
    queryKey: ["steam-apps"],
    queryFn: () => window.prism.games.listSteamApps(),
    staleTime: 0,
    gcTime: 0,
  });
  const available =
    apps.data?.filter(
      (app) => !added.some((game) => game.game.appId === app.appId),
    ) ?? [];
  const pending = games.addSteamGame.isPending;
  async function add(appId: number) {
    if (pending) return;
    try {
      await games.addSteamGame.mutateAsync(appId);
      onOpenChange(false);
    } catch {
      // The mutation's error is shown in the dialog.
    }
  }
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!pending) {
          games.addSteamGame.reset();
          onOpenChange(open);
        }
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add Steam game</DialogTitle>
          <DialogDescription>
            Fully installed games from your Steam libraries. Proton, the Steam
            Linux Runtime and redistributables aren't listed.
          </DialogDescription>
        </DialogHeader>
        {apps.isPending && (
          <p role="status" className="text-sm">
            Reading your Steam libraries…
          </p>
        )}
        {apps.error && (
          <p role="alert" className="text-sm text-destructive">
            {apps.error.message}
          </p>
        )}
        {apps.data?.length === 0 && (
          <p className="text-sm text-muted-foreground">
            No installed Steam games were found in your Steam libraries.
          </p>
        )}
        {!!apps.data?.length && available.length === 0 && (
          <p className="text-sm text-muted-foreground">
            Every installed Steam game is already on your Games page.
          </p>
        )}
        {games.addSteamGame.error && (
          <p role="alert" className="text-sm text-destructive">
            {games.addSteamGame.error.message}
          </p>
        )}
        {available.length > 0 && (
          <ul className="max-h-80 space-y-2 overflow-y-auto">
            {available.map((app) => (
              <li
                key={app.appId}
                className="flex items-center justify-between gap-3 rounded-lg border p-3"
              >
                <div className="min-w-0">
                  <p className="break-words text-sm font-medium">{app.name}</p>
                  <p className="text-xs text-muted-foreground">
                    Steam app {app.appId}
                  </p>
                </div>
                <Button
                  type="button"
                  size="sm"
                  disabled={pending}
                  aria-label={`Add ${app.name}`}
                  onClick={() => void add(app.appId)}
                >
                  {pending && games.addSteamGame.variables === app.appId && (
                    <LoaderCircle className="animate-spin" />
                  )}
                  Add
                </Button>
              </li>
            ))}
          </ul>
        )}
        <DialogFooter>
          <DialogClose asChild>
            <Button type="button" variant="outline" disabled={pending}>
              Close
            </Button>
          </DialogClose>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
