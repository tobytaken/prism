// PROTOTYPE (#7) Variant B — "Profile list in the tuning rail + Profiles library".
// Profiles are a visible radio list directly above the sliders; sliders mark the
// current profile's saved value. A dedicated Profiles page remains as the library:
// rename/delete, startup designation and behavior, game associations.
import { Gamepad2, Gauge, Star } from "lucide-react";
import { Link } from "react-router-dom";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import {
  actions,
  gameById,
  gamesLinkedTo,
  profilesFor,
  type Gpu,
  type State,
} from "./profile-prototype-store";
import {
  autoStatus,
  fmt,
  OutcomeList,
  StatusActions,
  startupSaveNote,
  toneClass,
  TuningEditor,
  useProfileActions,
} from "./profile-prototype-shared";
import { FakeTelemetry, Headline, PageTitle } from "./profile-prototype-chrome";
import { startupLabel } from "./profile-prototype-variant-a";

export function VariantB({ s }: { s: State }) {
  return (
    <div className="space-y-8">
      <PageTitle title="Performance" />
      {s.gpus.map((gpu) => (
        <GpuPanelB key={gpu.id} s={s} gpu={gpu} />
      ))}
    </div>
  );
}

function GpuPanelB({ s, gpu }: { s: State; gpu: Gpu }) {
  const status = autoStatus(s, gpu);
  const p = useProfileActions(s, gpu);
  const note = startupSaveNote(gpu);
  const big = status.tone !== "neutral" && status.tone !== "muted";

  return (
    <details open className="group/gpu rounded-lg border border-border/80 bg-card/55 shadow-sm">
      <summary className="flex cursor-pointer list-none items-center justify-between gap-4 border-b border-border/70 px-5 py-4 [&::-webkit-details-marker]:hidden">
        <div className="min-w-0">
          <h2 className="truncate text-lg font-semibold tracking-tight">{gpu.name}</h2>
          <span className="font-mono text-xs text-muted-foreground">{gpu.busId}</span>
        </div>
        <Headline />
      </summary>

      <div className="space-y-4 px-5 pt-5">
        {gpu.startupOutcome && (
          <div className={cn("flex items-start gap-3 rounded-md border px-4 py-3 text-sm", gpu.startupOutcome.ok ? toneClass.neutral : toneClass.danger)}>
            <div className="flex-1">
              <p className="font-medium">Last startup: {gpu.startupOutcome.summary}</p>
              <ul className="mt-1 text-xs opacity-80">
                {gpu.startupOutcome.lines.map((l) => (
                  <li key={l}>{l}</li>
                ))}
              </ul>
            </div>
            <Button size="xs" variant="ghost" onClick={() => actions.dismissStartupOutcome(gpu.id)}>
              Dismiss
            </Button>
          </div>
        )}
        {big && (
          <div className={cn("space-y-3 rounded-md border px-4 py-3", toneClass[status.tone])}>
            <div className="flex flex-wrap items-start gap-3">
              <Gamepad2 className="mt-0.5 size-5 shrink-0" />
              <div className="min-w-0 flex-1">
                <p className="font-semibold">{status.title}</p>
                <p className="mt-0.5 text-sm opacity-85">{status.detail}</p>
              </div>
              <div className="flex gap-2">
                <StatusActions gpu={gpu} status={status} />
              </div>
            </div>
            <OutcomeList status={status} />
          </div>
        )}
      </div>

      <section className="flex flex-col gap-8 px-5 py-7 xl:flex-row">
        <aside className="w-full shrink-0 space-y-5 xl:w-[460px] xl:border-r xl:border-border/70 xl:pr-5">
          <div className="flex items-center gap-2">
            <Gauge className="size-4 text-primary" />
            <span className="text-lg font-semibold tracking-tight">Overclocking</span>
          </div>

          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <span className="text-[11px] font-medium tracking-[0.18em] text-muted-foreground uppercase">Profiles</span>
              <span className="flex items-center gap-2 text-xs text-muted-foreground">
                {!big && <span>{status.short}</span>}
                {!big && <StatusActions gpu={gpu} status={status} size="xs" />}
              </span>
            </div>
            {p.editBlock && <p className="text-xs text-amber-200">{p.editBlock}</p>}
            <div role="radiogroup" className="grid gap-1.5">
              {profilesFor(s, gpu.id).map((profile) => {
                const isCurrent = profile.id === gpu.currentProfileId;
                const games = gamesLinkedTo(s, profile.id);
                const appliedFor =
                  isCurrent && gpu.currentIsAutomatic && gpu.auto.kind === "active" ? gameById(s, gpu.auto.gameId)?.name : null;
                return (
                  <button
                    key={profile.id}
                    type="button"
                    role="radio"
                    aria-checked={isCurrent}
                    disabled={!!p.editBlock && !isCurrent}
                    onClick={() => p.choose(profile.id)}
                    className={cn(
                      "flex items-center gap-3 rounded-md border px-3 py-2 text-left text-sm transition-colors disabled:opacity-50",
                      isCurrent ? "border-primary/60 bg-primary/10" : "border-border/70 hover:bg-muted/30",
                    )}
                  >
                    <span className={cn("size-3 shrink-0 rounded-full border-2", isCurrent ? "border-primary bg-primary" : "border-muted-foreground/50")} />
                    <span className="min-w-0 flex-1">
                      <span className="flex flex-wrap items-center gap-1.5 font-medium">
                        {profile.name}
                        {gpu.startupProfileId === profile.id && (
                          <Badge variant="outline" className="gap-1">
                            <Star className="fill-amber-300 text-amber-300" /> Startup
                          </Badge>
                        )}
                        {appliedFor && <Badge>Auto · {appliedFor}</Badge>}
                        {isCurrent && p.unsaved.length > 0 && (
                          <Badge variant="outline" className="border-amber-300/50 text-amber-200">
                            Changes not saved
                          </Badge>
                        )}
                      </span>
                      <span className="block text-xs text-muted-foreground tabular-nums">
                        {fmt("power", profile.values.power)} · {fmt("graphics", profile.values.graphics)} · {fmt("memory", profile.values.memory)}
                      </span>
                    </span>
                    {games.length > 0 && (
                      <span className="text-xs text-muted-foreground" title={games.map((g) => g.name).join(", ")}>
                        🎮 {games.length}
                      </span>
                    )}
                  </button>
                );
              })}
              {!p.current && (
                <p className="rounded-md border border-dashed border-border px-3 py-2 text-xs text-muted-foreground">
                  No current profile. The values below aren't saved in any profile.
                </p>
              )}
            </div>
            <div className="flex flex-wrap items-center gap-2 pt-1">
              {p.current && (
                <Button size="sm" variant="outline" disabled={!!p.saveBlock || p.unsaved.length === 0} onClick={p.save}>
                  Save to "{p.current.name}"
                </Button>
              )}
              <Button size="sm" variant="outline" disabled={!!p.saveBlock} onClick={p.saveAsNew}>
                Save as new…
              </Button>
              <Link to="/profiles" className="ml-auto text-xs text-primary underline">
                Manage in Profiles →
              </Link>
            </div>
            {note && p.unsaved.length > 0 && <p className="text-xs text-amber-200">{note}</p>}
          </div>

          <div className="h-px bg-border/70" />
          <TuningEditor s={s} gpu={gpu} showSavedMarkers />
        </aside>
        <FakeTelemetry />
      </section>
      {p.dialog}
    </details>
  );
}

// ---------------------------------------------------------------------------
// Profiles library page

export function ProfilesPageB({ s }: { s: State }) {
  return (
    <div className="space-y-8">
      <PageTitle title="Profiles" />

      <section className="grid gap-4 md:grid-cols-2">
        <div className="rounded-lg border border-border/80 bg-card/55 p-4">
          <p className="text-sm font-semibold">Automatic switching</p>
          <label className="mt-2 flex items-center gap-2 text-sm">
            <input type="checkbox" checked={s.autoEnabled} onChange={(e) => actions.setAutoEnabled(e.target.checked)} />
            Switch profiles automatically while my games run
          </label>
          <p className="mt-1 text-xs text-muted-foreground">Your setting. Other users on this computer choose for themselves.</p>
        </div>
        <div className="rounded-lg border border-border/80 bg-card/55 p-4">
          <p className="text-sm font-semibold">At startup (all users)</p>
          <select
            className="mt-2 h-8 w-full rounded-sm border border-input bg-background px-2 text-sm"
            value={s.startupBehavior}
            onChange={(e) => actions.setStartupBehavior(e.target.value as State["startupBehavior"])}
          >
            <option value="off">Off</option>
            <option value="startupProfiles">Apply each GPU's startup profile</option>
            <option value="restoreLast">Restore last applied values</option>
          </select>
          {s.startupBehavior === "restoreLast" && (
            <p className="mt-1 text-xs text-amber-200">An unstable value could be restored on every boot.</p>
          )}
          {s.startupBehavior === "startupProfiles" &&
            s.gpus
              .filter((g) => !g.startupProfileId)
              .map((g) => (
                <p key={g.id} className="mt-1 text-xs text-muted-foreground">
                  {g.name}: nothing applied at startup
                </p>
              ))}
        </div>
      </section>

      {s.gpus.map((gpu) => (
        <LibraryGpuSection key={gpu.id} s={s} gpu={gpu} />
      ))}

      <section className="space-y-3">
        <div className="flex items-center justify-between">
          <h2 className="text-lg font-semibold">Games</h2>
          <div className="flex gap-2">
            <Button size="sm" variant="outline" onClick={() => actions.addGame("Hollow Knight: Silksong", "steam", "Steam app 1030300")}>
              Add Steam game…
            </Button>
            <Button size="sm" variant="outline" onClick={() => actions.addGame("RetroArch", "manual", "/usr/bin/retroarch")}>
              Add running program…
            </Button>
          </div>
        </div>
        <table className="w-full text-sm">
          <thead className="text-left text-xs text-muted-foreground">
            <tr className="border-b border-border">
              <th className="py-2 font-medium">Game</th>
              {s.gpus.map((g) => (
                <th key={g.id} className="py-2 font-medium">
                  {g.name}
                </th>
              ))}
              <th />
            </tr>
          </thead>
          <tbody>
            {s.games.map((game) => (
              <tr key={game.id} className="border-b border-border/60">
                <td className="py-2">
                  <p className="font-medium">
                    {game.name}{" "}
                    {s.running.some((r) => r.gameId === game.id) && <Badge>Running</Badge>}{" "}
                    {!game.installed && <Badge variant="destructive">Not installed</Badge>}
                  </p>
                  <p className="text-xs text-muted-foreground">{game.detail}</p>
                </td>
                {s.gpus.map((gpu) => (
                  <td key={gpu.id} className="py-2">
                    <select
                      className="h-7 rounded-sm border border-input bg-background px-2 text-xs"
                      value={game.links[gpu.id] ?? ""}
                      onChange={(e) => actions.link(game.id, gpu.id, e.target.value || null)}
                    >
                      <option value="">—</option>
                      {profilesFor(s, gpu.id).map((pr) => (
                        <option key={pr.id} value={pr.id}>
                          {pr.name}
                        </option>
                      ))}
                    </select>
                  </td>
                ))}
                <td className="py-2 text-right">
                  <Button size="xs" variant="ghost" onClick={() => actions.removeGame(game.id)}>
                    Remove
                  </Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
    </div>
  );
}

function LibraryGpuSection({ s, gpu }: { s: State; gpu: Gpu }) {
  const p = useProfileActions(s, gpu);
  return (
    <section className="space-y-2">
      <h2 className="text-lg font-semibold">
        {gpu.name} <span className="font-mono text-xs font-normal text-muted-foreground">{gpu.busId}</span>
      </h2>
      <table className="w-full text-sm">
        <thead className="text-left text-xs text-muted-foreground">
          <tr className="border-b border-border">
            <th className="py-2 font-medium">Profile</th>
            <th className="py-2 font-medium">Power</th>
            <th className="py-2 font-medium">Graphics</th>
            <th className="py-2 font-medium">Memory</th>
            <th className="py-2 font-medium">Startup</th>
            <th className="py-2 font-medium">Games</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {profilesFor(s, gpu.id).map((profile) => (
            <tr key={profile.id} className="border-b border-border/60">
              <td className="py-2 font-medium">
                {profile.name} {gpu.currentProfileId === profile.id && <Badge variant="secondary">Current</Badge>}
              </td>
              <td className="py-2 tabular-nums">{fmt("power", profile.values.power)}</td>
              <td className="py-2 tabular-nums">{fmt("graphics", profile.values.graphics)}</td>
              <td className="py-2 tabular-nums">{fmt("memory", profile.values.memory)}</td>
              <td className="py-2">
                <input type="radio" name={`${gpu.id}-startup-b`} checked={gpu.startupProfileId === profile.id} onChange={() => p.toggleStartup(profile.id)} />
              </td>
              <td className="py-2 text-xs text-muted-foreground">{gamesLinkedTo(s, profile.id).map((g) => g.name).join(", ") || "—"}</td>
              <td className="py-2 text-right">
                <Button size="xs" variant="ghost" onClick={() => p.rename(profile.id)}>
                  Rename
                </Button>
                <Button size="xs" variant="destructive" onClick={() => p.remove(profile.id)}>
                  Delete
                </Button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="text-xs text-muted-foreground">
        Startup behavior: {startupLabel(s)}.{" "}
        <button type="button" className="underline" onClick={() => actions.setStartup(gpu.id, null)}>
          Clear startup profile
        </button>
      </p>
      {p.dialog}
    </section>
  );
}
