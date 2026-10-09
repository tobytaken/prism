import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  DialogClose,
} from "@/components/ui/dialog";
import type { GamesWorkflow } from "@/features/games/hooks/use-games";
import type { GameEntry } from "@/features/games/types";

export function AddProgramDialog({
  added,
  workflow,
  onOpenChange,
}: {
  added: GameEntry[];
  workflow: GamesWorkflow;
  onOpenChange: (open: boolean) => void;
}) {
  const [filter, setFilter] = useState("");
  const programs = useQuery({
    queryKey: ["running-programs"],
    queryFn: () => window.prism.games.listRunningPrograms(),
    staleTime: 0,
    gcTime: 0,
  });
  const pending =
    workflow.addProgram.isPending || workflow.browseProgram.isPending;
  const shown =
    programs.data?.filter((program) =>
      `${program.name} ${program.path ?? ""}`
        .toLowerCase()
        .includes(filter.trim().toLowerCase()),
    ) ?? [];
  async function add(path: string) {
    try {
      await workflow.addProgram.mutateAsync(path);
      onOpenChange(false);
    } catch {
      /* The mutation error is displayed below. */
    }
  }
  async function browse() {
    try {
      const state = await workflow.browseProgram.mutateAsync();
      if (state) onOpenChange(false);
    } catch {
      /* The mutation error is displayed below. */
    }
  }
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!pending) {
          workflow.addProgram.reset();
          workflow.browseProgram.reset();
          onOpenChange(open);
        }
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add running program</DialogTitle>
          <DialogDescription>
            Your own running programs. Known helpers appear last. Recognizing a
            program depends on how it runs.
          </DialogDescription>
        </DialogHeader>
        <Input
          type="search"
          aria-label="Filter running programs"
          placeholder="Filter by name or location"
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
        />
        {programs.isPending && <p role="status">Reading running programs…</p>}
        {programs.error && <p role="alert">{programs.error.message}</p>}
        {workflow.addProgram.error && (
          <p role="alert">{workflow.addProgram.error.message}</p>
        )}
        {workflow.browseProgram.error && (
          <p role="alert">{workflow.browseProgram.error.message}</p>
        )}
        {programs.isSuccess && shown.length === 0 && (
          <p>No running programs found.</p>
        )}
        <ul className="max-h-80 space-y-2 overflow-y-auto">
          {shown.map((program) => {
            const alreadyAdded = added.some(
              ({ game }) =>
                game.kind === "program" && game.path === program.path,
            );
            return (
              <li key={program.pid} className="space-y-2 rounded-lg border p-3">
                <div className="flex items-center justify-between gap-3">
                  <p className="break-words font-medium">
                    {program.name}{" "}
                    {program.helper && (
                      <span className="text-xs text-muted-foreground">
                        Helper
                      </span>
                    )}
                  </p>
                  <Button
                    size="sm"
                    disabled={pending || !program.path || alreadyAdded}
                    aria-label={`Add ${program.name}`}
                    onClick={() => {
                      if (program.path) void add(program.path);
                    }}
                  >
                    {alreadyAdded ? "Added" : "Add"}
                  </Button>
                </div>
                <p className="break-all text-xs text-muted-foreground">
                  {program.path ?? "Host location unavailable"}
                </p>
                <p className="text-xs text-muted-foreground">
                  PID {program.pid} ·{" "}
                  <span>{program.launcher ?? "Launcher unknown"}</span>
                </p>
                {program.environment.map((tag) => (
                  <p
                    key={tag}
                    className="break-all text-xs text-muted-foreground"
                  >
                    {tag}
                  </p>
                ))}
                {program.recognitionWarning && (
                  <p className="text-sm text-amber-600">
                    {program.recognitionWarning}
                  </p>
                )}
              </li>
            );
          })}
        </ul>
        <DialogFooter>
          <Button disabled={pending} onClick={() => void browse()}>
            Browse for file…
          </Button>
          <DialogClose asChild>
            <Button variant="outline" disabled={pending}>
              Close
            </Button>
          </DialogClose>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
