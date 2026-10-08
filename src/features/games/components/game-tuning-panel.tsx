import { Button } from "@/components/ui/button";
import { Link } from "react-router-dom";
import { gameKey } from "@/features/games/hooks/use-games";
import { useRunningGames } from "@/features/games/hooks/use-running-games";
import { describeGpuSwitching } from "@/features/games/lib/switching-status";
import { AutomaticSwitchingChoice } from "./automatic-switching-choice";
import { StartupOutcomes } from "@/features/gpus/components/startup-outcomes";
import { MigrationNotice } from "@/features/gpus/components/migration-notice";
import type { GpuProfileWorkflow } from "@/features/gpus/hooks/use-gpu-profiles";
import type { GpuPerformanceDevice } from "@/features/gpus/types";

export function GameTuningPanel({
  workflow,
  gpus,
  hasEdits = false,
  busy = false,
  applyDisabled = false,
  onApply,
  onDiscard,
}: {
  workflow: GpuProfileWorkflow;
  gpus: GpuPerformanceDevice[];
  hasEdits?: boolean;
  busy?: boolean;
  applyDisabled?: boolean;
  onApply?: () => void;
  onDiscard?: () => void;
}) {
  const running = useRunningGames();
  const games = workflow.games;
  return (
    <section
      aria-label="Game tuning"
      className="space-y-4 rounded-lg border bg-card/80 p-4"
    >
      <div className="flex items-center justify-between gap-4">
        <h2 className="text-lg font-semibold">Game tuning</h2>
        <Link className="text-sm underline" to="/games">
          Games
        </Link>
      </div>
      {!running && (
        <p role="status" className="text-sm">
          Reading running games…
        </p>
      )}
      {running?.status === "unavailable" && (
        <p role="status" className="text-sm text-muted-foreground">
          Recognition unavailable. {running.reason}
        </p>
      )}
      {running?.status === "available" &&
        (running.games.length === 0 ? (
          <p className="text-sm text-muted-foreground">No running games.</p>
        ) : (
          <ul
            aria-label="Running games"
            className="flex flex-wrap gap-2 text-sm"
          >
            {running.games.map(({ game }) => (
              <li key={gameKey(game)} className="rounded-md bg-muted px-3 py-1">
                {games.query.data?.games.find(
                  (entry) => gameKey(entry.game) === gameKey(game),
                )?.name ?? `Steam app ${game.appId}`}
              </li>
            ))}
          </ul>
        ))}
      <AutomaticSwitchingChoice workflow={games} />
      {gpus.length > 0 && (
        <ul
          aria-label="Automatic switching status"
          className="space-y-1 text-sm"
        >
          {gpus.map((gpu) => (
            <li key={gpu.id} className="text-muted-foreground">
              {`${gpu.name}: ${
                gpu.stableId
                  ? describeGpuSwitching(workflow.switching, gpu.stableId)
                      .summary
                  : "Unavailable: this GPU reports no stable identity (UUID)."
              }`}
            </li>
          ))}
        </ul>
      )}
      {workflow.switching?.gpus.some(
        (gpu) => gpu.state.status === "waitingForEdits",
      ) && (
        <div className="flex gap-2">
          <Button
            size="sm"
            disabled={!hasEdits || busy || applyDisabled}
            onClick={onApply}
          >
            Apply my changes
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={!hasEdits || busy}
            onClick={onDiscard}
          >
            Discard my changes
          </Button>
          {!hasEdits && (
            <p className="text-sm text-muted-foreground">
              Apply or discard the edits in the Prism session where they were
              made.
            </p>
          )}
        </div>
      )}
      {(games.query.error || games.changeError) && (
        <p role="alert" className="text-sm text-destructive">
          {(games.query.error ?? games.changeError)?.message}
        </p>
      )}
      <MigrationNotice workflow={workflow} />
      <StartupOutcomes workflow={workflow} gpus={gpus} />
    </section>
  );
}
