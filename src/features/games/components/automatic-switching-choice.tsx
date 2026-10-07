import { useId } from "react";
import type { GamesWorkflow } from "@/features/games/hooks/use-games";

export function AutomaticSwitchingChoice({
  workflow,
}: {
  workflow: GamesWorkflow;
}) {
  const id = useId();
  const state = workflow.query.data;
  return (
    <div className="max-w-2xl space-y-1 text-sm">
      <label
        htmlFor={`${id}-switching`}
        className="flex items-center gap-3 font-medium"
      >
        <input
          id={`${id}-switching`}
          type="checkbox"
          checked={state?.automaticSwitching ?? false}
          disabled={!state || workflow.setAutomaticSwitching.isPending}
          aria-describedby={`${id}-switching-note`}
          onChange={(event) =>
            workflow.setAutomaticSwitching.mutate(event.target.checked)
          }
        />
        Switch profiles automatically while my games run
      </label>
      <p id={`${id}-switching-note`} className="text-muted-foreground">
        Off by default, and only for your own games. While a linked game runs,
        Prism applies its profile to each linked GPU, even with this app closed,
        and restores the values from before the game when it ends.
      </p>
    </div>
  );
}
