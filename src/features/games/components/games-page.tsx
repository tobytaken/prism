import { useId, useState } from "react";
import { Plus } from "lucide-react";
import { Link } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { gameKey, useGames } from "@/features/games/hooks/use-games";
import type { GameEntry } from "@/features/games/types";
import { listGpuProfiles } from "@/features/gpus/api";
import { useGpuPerformance } from "@/features/gpus/hooks/use-gpu-performance";
import { PROFILES_QUERY_KEY } from "@/features/gpus/hooks/use-gpu-profiles";
import type { GpuTuningProfile } from "@/features/gpus/types";
import { AddSteamGameDialog } from "./add-steam-game-dialog";

type GpuChoice = { uuid: string; label: string };

/** Links the caller's games to their GPU tuning profiles, one profile per GPU. */
export function GamesPage() {
  const id = useId();
  const games = useGames();
  const profiles = useQuery({
    queryKey: PROFILES_QUERY_KEY,
    queryFn: listGpuProfiles,
  });
  const performance = useGpuPerformance();
  const [filter, setFilter] = useState("");
  const [adding, setAdding] = useState(false);
  const state = games.query.data;
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
  const changeError =
    games.setProfile.error ??
    games.remove.error ??
    games.setAutomaticSwitching.error;
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

      <div className="max-w-2xl space-y-1 rounded-lg border p-3 text-sm">
        <label
          htmlFor={`${id}-switching`}
          className="flex items-center gap-3 font-medium"
        >
          <input
            id={`${id}-switching`}
            type="checkbox"
            checked={state?.automaticSwitching ?? false}
            disabled={!state || games.setAutomaticSwitching.isPending}
            aria-describedby={`${id}-switching-note`}
            onChange={(event) =>
              games.setAutomaticSwitching.mutate(event.target.checked)
            }
          />
          Switch profiles automatically while my games run
        </label>
        <p id={`${id}-switching-note`} className="text-muted-foreground">
          Off by default, and only for your own games. You can link games while
          it's off. Prism doesn't switch profiles for games yet; your choice and
          links are kept for when it does.
        </p>
      </div>

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

      {games.query.isPending && <p role="status">Reading your games…</p>}
      {games.query.error && (
        <p role="alert" className="text-sm text-destructive">
          {games.query.error.message}
        </p>
      )}
      {changeError && (
        <p role="alert" className="text-sm text-destructive">
          {changeError.message}
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
            gpus={gpus}
            profiles={profiles.data ?? []}
            profilesKnown={profiles.isSuccess}
            games={games}
          />
        ))}
      </div>
      {adding && state && (
        <AddSteamGameDialog
          added={state.games}
          games={games}
          onOpenChange={setAdding}
        />
      )}
    </div>
  );
}

function GameCard({
  game,
  gpus,
  profiles,
  profilesKnown,
  games,
}: {
  game: GameEntry;
  gpus: GpuChoice[];
  profiles: GpuTuningProfile[];
  profilesKnown: boolean;
  games: ReturnType<typeof useGames>;
}) {
  const id = useId();
  const pending = games.setProfile.isPending || games.remove.isPending;
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
          onClick={() => games.remove.mutate(game.game)}
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
                games.setProfile.mutate({
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
  games: GameEntry[],
): GpuChoice[] {
  const choices = new Map<string, string>();
  for (const gpu of present)
    if (gpu.stableId?.trim())
      choices.set(gpu.stableId, `${gpu.name} (${gpu.stableId})`);
  const absent = [
    ...profiles.map((profile) => profile.gpuId),
    ...games.flatMap((game) => Object.keys(game.profiles)),
  ];
  for (const uuid of absent)
    if (!choices.has(uuid)) choices.set(uuid, `absent GPU (${uuid})`);
  return [...choices].map(([uuid, label]) => ({ uuid, label }));
}
