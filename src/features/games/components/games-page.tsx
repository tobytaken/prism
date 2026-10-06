import { AutomaticSwitchingChoice } from "./automatic-switching-choice";
import { useRunningGames } from "@/features/games/hooks/use-running-games";
import { useId, useState } from "react";
import { Plus } from "lucide-react";
import { Link } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  gameKey,
  useGames,
  type GamesWorkflow,
} from "@/features/games/hooks/use-games";
import type { GameEntry } from "@/features/games/types";
import { listGpuProfiles } from "@/features/gpus/api";
import { useGpuPerformance } from "@/features/gpus/hooks/use-gpu-performance";
import { PROFILES_QUERY_KEY } from "@/features/gpus/hooks/use-gpu-profiles";
import type { GpuTuningProfile } from "@/features/gpus/types";
import { AddSteamGameDialog } from "./add-steam-game-dialog";

type GpuChoice = { uuid: string; label: string };

/** Links the caller's games to their GPU tuning profiles, one profile per GPU. */
export function GamesPage() {
  const workflow = useGames();
  const running = useRunningGames();
  const profiles = useQuery({
    queryKey: PROFILES_QUERY_KEY,
    queryFn: listGpuProfiles,
  });
  const performance = useGpuPerformance();
  const [filter, setFilter] = useState("");
  const [adding, setAdding] = useState(false);
  const state = workflow.query.data;
  const gpus = gpuChoices(
    performance.data?.gpus ?? [],
    profiles.data ?? [],
    state?.games ?? [],
  );
  const term = filter.trim().toLowerCase();
  const shown =
    state?.games.filter(
      (game) =>
        !term ||
        game.name.toLowerCase().includes(term) ||
        String(game.game.appId).includes(term),
    ) ?? [];
  return (
    <div className="space-y-6">
      <div className="space-y-1">
        <h1 className="text-2xl font-semibold tracking-tight">Games</h1>
        <p className="text-sm text-muted-foreground">
          Link your Steam games to your GPU tuning profiles, one profile per
          GPU. Create profiles on{" "}
          <Link className="underline" to="/performance">
            Performance
          </Link>
          .
        </p>
      </div>

      <div className="max-w-2xl rounded-lg border p-3">
        <AutomaticSwitchingChoice workflow={workflow} />
      </div>
      {running?.status === "unavailable" && (
        <p role="status" className="text-sm text-muted-foreground">
          Recognition unavailable. {running.reason}
        </p>
      )}

      <div className="flex flex-wrap items-center gap-3">
        <Input
          type="search"
          aria-label="Filter games"
          placeholder="Filter by name or app ID"
          className="max-w-xs"
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
        />
        <Button type="button" disabled={!state} onClick={() => setAdding(true)}>
          <Plus />
          Add Steam game…
        </Button>
      </div>

      {workflow.query.isPending && <p role="status">Reading your games…</p>}
      {workflow.query.error && (
        <p role="alert" className="text-sm text-destructive">
          {workflow.query.error.message}
        </p>
      )}
      {workflow.changeError && (
        <p role="alert" className="text-sm text-destructive">
          {workflow.changeError.message}
        </p>
      )}
      {profiles.error && (
        <p role="alert" className="text-sm text-destructive">
          Your profiles could not be read: {profiles.error.message}
        </p>
      )}
      {state && state.games.length === 0 && (
        <p className="text-sm text-muted-foreground">
          No games yet. Add a Steam game to link it to your profiles.
        </p>
      )}
      {state && state.games.length > 0 && shown.length === 0 && (
        <p className="text-sm text-muted-foreground">
          No games match "{filter.trim()}".
        </p>
      )}
      <div className="grid gap-4 md:grid-cols-2">
        {shown.map((game) => (
          <GameCard
            key={gameKey(game.game)}
            game={game}
            running={
              running?.status === "available" &&
              running.games.some(
                (entry) => gameKey(entry.game) === gameKey(game.game),
              )
            }
            gpus={gpus}
            profiles={profiles.data ?? []}
            profilesKnown={profiles.isSuccess}
            workflow={workflow}
          />
        ))}
      </div>
      {adding && state && (
        <AddSteamGameDialog
          added={state.games}
          workflow={workflow}
          onOpenChange={setAdding}
        />
      )}
    </div>
  );
}

function GameCard({
  game,
  running,
  gpus,
  profiles,
  profilesKnown,
  workflow,
}: {
  game: GameEntry;
  running: boolean;
  gpus: GpuChoice[];
  profiles: GpuTuningProfile[];
  profilesKnown: boolean;
  workflow: GamesWorkflow;
}) {
  const id = useId();
  const pending = workflow.setProfile.isPending || workflow.remove.isPending;
  return (
    <article
      aria-labelledby={`${id}-name`}
      className="space-y-3 rounded-lg border bg-card/80 p-4"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 space-y-1">
          <h2 id={`${id}-name`} className="break-words font-medium">
            {game.name}
          </h2>
          <p className="text-xs text-muted-foreground">
            Steam app {game.game.appId}
          </p>
          {running && <Badge>Running</Badge>}
          {!game.installed && (
            <Badge
              variant="outline"
              title="The game isn't in your Steam libraries. Its links are kept until you remove it."
            >
              Not installed
            </Badge>
          )}
        </div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={pending}
          aria-label={`Remove ${game.name}`}
          onClick={() => workflow.remove.mutate(game.game)}
        >
          Remove
        </Button>
      </div>
      {gpus.length === 0 && (
        <p className="text-sm text-muted-foreground">
          No GPU profiles yet. Save a profile on Performance to link this game.
        </p>
      )}
      {gpus.map((gpu) => {
        const own = profiles.filter((profile) => profile.gpuId === gpu.uuid);
        const linked = game.profiles[gpu.uuid] ?? "";
        return (
          <div key={gpu.uuid} className="grid gap-1 text-sm">
            <label htmlFor={`${id}-${gpu.uuid}`}>Profile on {gpu.label}</label>
            <select
              id={`${id}-${gpu.uuid}`}
              className="rounded-md border bg-background p-2"
              value={linked}
              disabled={pending || !profilesKnown}
              onChange={(event) =>
                workflow.setProfile.mutate({
                  game: game.game,
                  gpuId: gpu.uuid,
                  profileId: event.target.value || null,
                })
              }
            >
              <option value="">Leave unchanged</option>
              {linked &&
                profilesKnown &&
                !own.some((profile) => profile.id === linked) && (
                  <option value={linked} disabled>
                    Unavailable profile
                  </option>
                )}
              {own.map((profile) => (
                <option key={profile.id} value={profile.id}>
                  {profile.name}
                </option>
              ))}
            </select>
          </div>
        );
      })}
    </article>
  );
}

/**
 * Every GPU a game can be linked on: present GPUs with a stable identity, and absent GPUs that
 * still have the caller's profiles or links.
 */
function gpuChoices(
  present: { name: string; stableId: string | null }[],
  profiles: GpuTuningProfile[],
  associations: GameEntry[],
): GpuChoice[] {
  const choices = new Map<string, string>();
  for (const gpu of present)
    if (gpu.stableId?.trim())
      choices.set(gpu.stableId, `${gpu.name} (${gpu.stableId})`);
  const absent = [
    ...profiles.map((profile) => profile.gpuId),
    ...associations.flatMap((game) => Object.keys(game.profiles)),
  ];
  for (const uuid of absent)
    if (!choices.has(uuid)) choices.set(uuid, `absent GPU (${uuid})`);
  return [...choices].map(([uuid, label]) => ({ uuid, label }));
}
