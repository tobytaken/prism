// PROTOTYPE (#7) Variant C — "Game tuning console + Games page".
// Automatic switching status for every GPU is gathered in one console at the top of
// Performance. Each GPU's rail has a compact profile select with an overflow menu.
// The Profiles page becomes a Games page: the association grid only.
import { useState } from "react";
import { Gamepad2, Gauge, MoreHorizontal } from "lucide-react";
import { Link } from "react-router-dom";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import {
  actions,
  gameById,
  hasAnyEdits,
  profileById,
  profilesFor,
  type Gpu,
  type State,
} from "./profile-prototype-store";
import {
  autoStatus,
  OutcomeList,
  StatusActions,
  startupSaveNote,
  toneDot,
  TuningEditor,
  useProfileActions,
  valuesSummary,
} from "./profile-prototype-shared";
import { FakeTelemetry, Headline, PageTitle } from "./profile-prototype-chrome";

export function VariantC({ s }: { s: State }) {
  return (
    <div className="space-y-8">
      <PageTitle title="Performance" />
      <GameTuningConsole s={s} />
      {s.gpus.map((gpu) => (
        <GpuPanelC key={gpu.id} s={s} gpu={gpu} />
      ))}
    </div>
  );
}

function GameTuningConsole({ s }: { s: State }) {
  const running = s.running.map((r) => gameById(s, r.gameId)!).filter(Boolean);
  const outcomes = s.gpus.filter((g) => g.startupOutcome);
  return (
    <section className="rounded-lg border border-border/80 bg-card/55">
      <div className="flex flex-wrap items-center gap-3 border-b border-border/70 px-4 py-3">
        <Gamepad2 className="size-4 text-primary" />
        <span className="font-semibold">Game tuning</span>
        <span className="text-xs text-muted-foreground">
          {running.length ? `Running: ${running.map((g) => g.name).join(", ")}` : "No games running"}
        </span>
        <label className="ml-auto flex items-center gap-2 text-sm">
          <input type="checkbox" checked={s.autoEnabled} onChange={(e) => actions.setAutoEnabled(e.target.checked)} />
          Switch automatically
        </label>
        <Link to="/profiles" className="text-xs text-primary underline">
          Games →
        </Link>
      </div>
      <div className="divide-y divide-border/60">
        {s.gpus.map((gpu) => {
          const status = autoStatus(s, gpu);
          return (
            <div key={gpu.id} className="space-y-2 px-4 py-2.5">
              <div className="flex flex-wrap items-center gap-3 text-sm">
                <span className={cn("size-2 shrink-0 rounded-full", toneDot[status.tone])} />
                <span className="w-48 shrink-0 truncate font-medium">{gpu.name}</span>
                <span className={cn("flex-1", status.tone === "danger" && "text-destructive", status.tone === "warning" && "text-amber-200")}>
                  {status.title}
                </span>
                <StatusActions gpu={gpu} status={status} size="xs" />
              </div>
              {status.tone !== "neutral" && <p className="pl-[13.5rem] text-xs text-muted-foreground">{status.detail}</p>}
              {status.outcomes && (
                <div className="pl-[13.5rem]">
                  <OutcomeList status={status} />
                </div>
              )}
            </div>
          );
        })}
        {outcomes.map((gpu) => (
          <div key={`${gpu.id}-startup`} className="flex items-center gap-3 px-4 py-2.5 text-sm">
            <span className={cn("size-2 rounded-full", gpu.startupOutcome!.ok ? "bg-primary" : "bg-destructive")} />
            <span className="w-48 shrink-0 truncate font-medium">{gpu.name}</span>
            <span className="flex-1">
              Last startup: {gpu.startupOutcome!.summary}
              <span className="block text-xs text-muted-foreground">{gpu.startupOutcome!.lines.join(" · ")}</span>
            </span>
            <Button size="xs" variant="ghost" onClick={() => actions.dismissStartupOutcome(gpu.id)}>
              Dismiss
            </Button>
          </div>
        ))}
      </div>
    </section>
  );
}

function GpuPanelC({ s, gpu }: { s: State; gpu: Gpu }) {
  const p = useProfileActions(s, gpu);
  const [menu, setMenu] = useState(false);
  const note = startupSaveNote(gpu);
  const status = autoStatus(s, gpu);
  const automaticSource = gpu.currentIsAutomatic && gpu.auto.kind === "active" ? gameById(s, gpu.auto.gameId)?.name : null;

  return (
    <details open className="group/gpu rounded-lg border border-border/80 bg-card/55 shadow-sm">
      <summary className="flex cursor-pointer list-none items-center justify-between gap-4 border-b border-border/70 px-5 py-4 [&::-webkit-details-marker]:hidden">
        <div className="min-w-0">
          <h2 className="truncate text-lg font-semibold tracking-tight">{gpu.name}</h2>
          <span className="font-mono text-xs text-muted-foreground">{gpu.busId}</span>
        </div>
        <div className="flex items-center gap-3">
          {status.tone !== "neutral" && (
            <Badge variant="outline" className="gap-1.5">
              <span className={cn("size-1.5 rounded-full", toneDot[status.tone])} />
              {status.short}
            </Badge>
          )}
          <Headline />
        </div>
      </summary>
      <section className="flex flex-col gap-8 px-5 py-7 xl:flex-row">
        <aside className="w-full shrink-0 space-y-4 xl:w-[460px] xl:border-r xl:border-border/70 xl:pr-5">
          <div className="flex items-center gap-2">
            <Gauge className="size-4 text-primary" />
            <span className="text-lg font-semibold tracking-tight">Overclocking</span>
          </div>

          <div className="space-y-1.5">
            <div className="flex items-center gap-2">
              <select
                aria-label="Profile"
                className="h-8 flex-1 rounded-sm border border-input bg-background px-2 text-sm disabled:opacity-60"
                value={gpu.currentProfileId ?? ""}
                disabled={!!p.editBlock}
                onChange={(e) => e.target.value && p.choose(e.target.value)}
              >
                {!gpu.currentProfileId && <option value="">No current profile</option>}
                {profilesFor(s, gpu.id).map((pr) => (
                  <option key={pr.id} value={pr.id}>
                    {pr.name}
                    {gpu.startupProfileId === pr.id ? "  ★ startup" : ""}
                  </option>
                ))}
              </select>
              <div className="relative">
                <Button size="icon-sm" variant="outline" aria-label="Profile actions" onClick={() => setMenu((m) => !m)}>
                  <MoreHorizontal />
                </Button>
                {menu && (
                  <>
                    <div className="fixed inset-0 z-30" onClick={() => setMenu(false)} />
                    <div className="absolute right-0 top-full z-40 mt-1 w-64 rounded-md border border-border bg-popover p-1 text-sm shadow-xl">
                      {[
                        p.current && {
                          label: `Save to "${p.current.name}"`,
                          hint: note,
                          disabled: !!p.saveBlock || p.unsaved.length === 0,
                          run: p.save,
                        },
                        { label: "Save as new…", disabled: !!p.saveBlock, run: p.saveAsNew },
                        p.current && { label: `Rename "${p.current.name}"…`, run: () => p.rename(p.current!.id) },
                        p.current && {
                          label: gpu.startupProfileId === p.current.id ? "Stop using at startup" : "Use at startup",
                          hint: gpu.startupProfileId && gpu.startupProfileId !== p.current.id ? `Replaces "${profileById(s, gpu.startupProfileId)?.name}"` : null,
                          run: () => p.toggleStartup(p.current!.id),
                        },
                        p.current && { label: `Delete "${p.current.name}"…`, danger: true, run: () => p.remove(p.current!.id) },
                      ]
                        .filter((x): x is Exclude<typeof x, null | undefined | false> => !!x)
                        .map((item) => (
                          <button
                            key={item.label}
                            type="button"
                            disabled={"disabled" in item && item.disabled}
                            onClick={() => {
                              item.run();
                              setMenu(false);
                            }}
                            className={cn(
                              "w-full rounded-sm px-2 py-1.5 text-left hover:bg-muted disabled:opacity-50",
                              "danger" in item && item.danger && "text-destructive",
                            )}
                          >
                            {item.label}
                            {"hint" in item && item.hint && <span className="block text-xs text-amber-200">{item.hint}</span>}
                          </button>
                        ))}
                      <p className="px-2 pt-1 text-[11px] text-muted-foreground">
                        Startup behavior is in{" "}
                        <Link className="underline" to="/settings">
                          Settings
                        </Link>
                        .
                      </p>
                    </div>
                  </>
                )}
              </div>
            </div>
            <p className="text-xs text-muted-foreground">
              {p.editBlock ? (
                <span className="text-amber-200">{p.editBlock}</span>
              ) : !p.current ? (
                "Reported values aren't saved in any profile."
              ) : automaticSource ? (
                <>Applied automatically for {automaticSource}.</>
              ) : p.unsaved.length > 0 ? (
                <span className="text-amber-200">
                  Changes not saved to "{p.current.name}" · saved {valuesSummary(p.current.values)}
                </span>
              ) : (
                <>Reported values match "{p.current.name}".</>
              )}
            </p>
            {gpu.auto.kind === "waiting" && hasAnyEdits(gpu) && (
              <p className="rounded-sm border border-amber-400/40 bg-amber-400/10 px-2 py-1.5 text-xs text-amber-100">{status.title}</p>
            )}
          </div>

          <TuningEditor s={s} gpu={gpu} />
        </aside>
        <FakeTelemetry />
      </section>
      {p.dialog}
    </details>
  );
}

// ---------------------------------------------------------------------------
// Games page (replaces the Profiles placeholder)

export function ProfilesPageC({ s }: { s: State }) {
  const [filter, setFilter] = useState("");
  const games = s.games.filter((g) => g.name.toLowerCase().includes(filter.toLowerCase()));
  return (
    <div className="space-y-6">
      <PageTitle title="Games">
        <span className="text-xs text-muted-foreground">(sidebar entry would read "Games")</span>
      </PageTitle>
      <div className="flex flex-wrap items-center gap-3">
        <label className="flex items-center gap-2 text-sm">
          <input type="checkbox" checked={s.autoEnabled} onChange={(e) => actions.setAutoEnabled(e.target.checked)} />
          Switch profiles automatically while my games run
        </label>
        <input
          className="ml-auto h-8 w-56 rounded-sm border border-input bg-background px-2 text-sm"
          placeholder="Filter games"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        <Button size="sm" variant="outline" onClick={() => actions.addGame("Hollow Knight: Silksong", "steam", "Steam app 1030300")}>
          Add Steam game…
        </Button>
        <Button size="sm" variant="outline" onClick={() => actions.addGame("RetroArch", "manual", "/usr/bin/retroarch")}>
          Add running program…
        </Button>
      </div>
      <div className="grid gap-3 md:grid-cols-2">
        {games.map((game) => {
          const running = s.running.some((r) => r.gameId === game.id);
          return (
            <div key={game.id} className={cn("rounded-lg border bg-card/55 p-4", running ? "border-primary/50" : "border-border/80")}>
              <div className="flex items-start justify-between gap-2">
                <div>
                  <p className="font-semibold">
                    {game.name} {running && <Badge>Running</Badge>} {!game.installed && <Badge variant="destructive">Not installed</Badge>}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {game.source === "steam" ? "Steam" : "Program"} · {game.detail}
                  </p>
                </div>
                <Button size="xs" variant="ghost" onClick={() => actions.removeGame(game.id)}>
                  Remove
                </Button>
              </div>
              <div className="mt-3 space-y-1.5">
                {s.gpus.map((gpu) => (
                  <label key={gpu.id} className="flex items-center justify-between gap-3 text-sm">
                    <span className="truncate text-muted-foreground">{gpu.name}</span>
                    <select
                      className="h-7 w-40 rounded-sm border border-input bg-background px-2 text-xs"
                      value={game.links[gpu.id] ?? ""}
                      onChange={(e) => actions.link(game.id, gpu.id, e.target.value || null)}
                    >
                      <option value="">Leave unchanged</option>
                      {profilesFor(s, gpu.id).map((pr) => (
                        <option key={pr.id} value={pr.id}>
                          {pr.name}
                        </option>
                      ))}
                    </select>
                  </label>
                ))}
              </div>
            </div>
          );
        })}
      </div>
      <p className="text-xs text-muted-foreground">
        Profiles themselves are created and edited beside the tuning controls on Performance.
      </p>
    </div>
  );
}
