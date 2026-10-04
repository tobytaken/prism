// PROTOTYPE (#7) Variant A — "Header menu, no Profiles page".
// The profile selector lives in each GPU panel's header. Rename/delete/startup and
// game associations live in a per-GPU "Manage" dialog. The Profiles page is removed.
import { useState } from "react";
import { Bookmark, Check, ChevronDown, Gamepad2, Gauge, Star } from "lucide-react";
import { Link } from "react-router-dom";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import {
  actions,
  gamesLinkedTo,
  profilesFor,
  type Gpu,
  type State,
} from "./profile-prototype-store";
import {
  autoStatus,
  Modal,
  OutcomeList,
  StatusActions,
  startupSaveNote,
  toneClass,
  toneDot,
  TuningEditor,
  useProfileActions,
  valuesSummary,
} from "./profile-prototype-shared";
import { FakeTelemetry, Headline, PageTitle } from "./profile-prototype-chrome";

export function VariantA({ s }: { s: State }) {
  return (
    <div className="space-y-8">
      <PageTitle title="Performance" />
      {s.gpus.map((gpu) => (
        <GpuPanelA key={gpu.id} s={s} gpu={gpu} />
      ))}
    </div>
  );
}

function GpuPanelA({ s, gpu }: { s: State; gpu: Gpu }) {
  const status = autoStatus(s, gpu);
  const p = useProfileActions(s, gpu);
  const [menuOpen, setMenuOpen] = useState(false);
  const [manageOpen, setManageOpen] = useState(false);
  const showStrip = status.tone !== "neutral";

  return (
    <details open className="group/gpu rounded-lg border border-border/80 bg-card/55 shadow-sm">
      <summary className="flex cursor-pointer list-none items-center justify-between gap-4 border-b border-border/70 px-5 py-4 [&::-webkit-details-marker]:hidden">
        <div className="min-w-0">
          <h2 className="truncate text-lg font-semibold tracking-tight">{gpu.name}</h2>
          <span className="font-mono text-xs text-muted-foreground">{gpu.busId}</span>
        </div>

        {/* The profile selector: the primary affordance of this variant */}
        <div className="relative" onClick={(e) => e.preventDefault()}>
          <button
            type="button"
            onClick={() => setMenuOpen((o) => !o)}
            className="flex min-w-64 items-center gap-3 rounded-md border border-border bg-background/60 px-3 py-2 text-left hover:bg-muted/40"
          >
            <Bookmark className="size-4 text-primary" />
            <span className="min-w-0 flex-1">
              <span className="block text-[10px] tracking-[0.18em] text-muted-foreground uppercase">Profile</span>
              <span className="flex items-center gap-2 text-sm font-semibold">
                {p.current?.name ?? "No current profile"}
                {p.unsaved.length > 0 && <span className="size-1.5 rounded-full bg-amber-300" title="Changes not saved to profile" />}
              </span>
            </span>
            <span className={cn("size-2 rounded-full", toneDot[status.tone])} title={status.title} />
            <ChevronDown className="size-4 text-muted-foreground" />
          </button>
          {menuOpen && (
            <ProfileMenu s={s} gpu={gpu} p={p} onClose={() => setMenuOpen(false)} onManage={() => setManageOpen(true)} />
          )}
        </div>

        <div className="flex items-center gap-4">
          <Headline />
          <ChevronDown className="size-4 text-muted-foreground transition-transform group-open/gpu:rotate-180" />
        </div>
      </summary>

      {gpu.startupOutcome && <StartupNoticeA gpu={gpu} />}

      {showStrip && (
        <div className={cn("space-y-2 border-b px-5 py-2.5 text-sm", toneClass[status.tone])}>
          <div className="flex flex-wrap items-center gap-3">
            <Gamepad2 className="size-4 shrink-0" />
            <span className="flex-1 font-medium">{status.title}</span>
            <StatusActions gpu={gpu} status={status} size="xs" />
          </div>
          <p className="pl-7 text-xs opacity-80">{status.detail}</p>
          {status.outcomes && (
            <div className="pl-7">
              <OutcomeList status={status} />
            </div>
          )}
        </div>
      )}

      <section className="flex flex-col gap-8 px-5 py-7 xl:flex-row">
        <aside className="w-full shrink-0 xl:w-[460px] xl:border-r xl:border-border/70 xl:pr-5">
          <div className="mb-4 flex items-center gap-2">
            <Gauge className="size-4 text-primary" />
            <span className="text-lg font-semibold tracking-tight">Overclocking</span>
          </div>
          {p.current && p.unsaved.length > 0 && (
            <div className="mb-4 flex flex-wrap items-center gap-2 rounded-md border border-amber-300/30 bg-amber-300/5 px-3 py-2 text-xs">
              <span className="flex-1 text-amber-100">
                Reported values differ from "{p.current.name}" ({valuesSummary(p.current.values)})
              </span>
              <Button size="xs" variant="outline" disabled={!!p.saveBlock} onClick={p.save}>
                Save to "{p.current.name}"
              </Button>
            </div>
          )}
          <TuningEditor s={s} gpu={gpu} />
        </aside>
        <FakeTelemetry />
      </section>
      {manageOpen && <ManageDialogA s={s} gpu={gpu} p={p} onClose={() => setManageOpen(false)} />}
      {p.dialog}
    </details>
  );
}

function ProfileMenu({
  s,
  gpu,
  p,
  onClose,
  onManage,
}: {
  s: State;
  gpu: Gpu;
  p: ReturnType<typeof useProfileActions>;
  onClose: () => void;
  onManage: () => void;
}) {
  const auto = gpu.auto.kind === "active" ? gpu.auto : null;
  const note = startupSaveNote(gpu);
  return (
    <>
      <div className="fixed inset-0 z-30" onClick={onClose} />
      <div className="absolute left-0 top-full z-40 mt-1 w-80 rounded-md border border-border bg-popover p-1 shadow-xl">
        {p.editBlock && <p className="px-2 py-1.5 text-xs text-amber-200">{p.editBlock}</p>}
        {profilesFor(s, gpu.id).map((profile) => {
          const isCurrent = profile.id === gpu.currentProfileId;
          const games = gamesLinkedTo(s, profile.id);
          return (
            <button
              key={profile.id}
              type="button"
              disabled={!!p.editBlock}
              onClick={() => {
                p.choose(profile.id);
                onClose();
              }}
              className="flex w-full items-start gap-2 rounded-sm px-2 py-1.5 text-left text-sm hover:bg-muted disabled:opacity-50"
            >
              <Check className={cn("mt-0.5 size-4", isCurrent ? "text-primary" : "invisible")} />
              <span className="min-w-0 flex-1">
                <span className="flex items-center gap-1.5 font-medium">
                  {profile.name}
                  {gpu.startupProfileId === profile.id && <Star className="size-3 fill-amber-300 text-amber-300" />}
                  {isCurrent && auto && <Badge variant="outline">auto · {s.games.find((g) => g.id === auto.gameId)?.name}</Badge>}
                </span>
                <span className="block text-xs text-muted-foreground">{valuesSummary(profile.values)}</span>
                {games.length > 0 && (
                  <span className="block text-xs text-muted-foreground">🎮 {games.map((g) => g.name).join(", ")}</span>
                )}
              </span>
            </button>
          );
        })}
        <div className="my-1 h-px bg-border" />
        {p.current && (
          <button
            type="button"
            disabled={!!p.saveBlock || p.unsaved.length === 0}
            onClick={() => (p.save(), onClose())}
            className="w-full rounded-sm px-2 py-1.5 text-left text-sm hover:bg-muted disabled:opacity-50"
          >
            Save to "{p.current.name}"
            {note && <span className="block text-xs text-amber-200">{note}</span>}
          </button>
        )}
        <button
          type="button"
          disabled={!!p.saveBlock}
          onClick={() => (p.saveAsNew(), onClose())}
          className="w-full rounded-sm px-2 py-1.5 text-left text-sm hover:bg-muted disabled:opacity-50"
        >
          Save as new…
        </button>
        <button
          type="button"
          onClick={() => (onManage(), onClose())}
          className="w-full rounded-sm px-2 py-1.5 text-left text-sm hover:bg-muted"
        >
          Manage profiles &amp; games…
        </button>
      </div>
    </>
  );
}

function ManageDialogA({
  s,
  gpu,
  p,
  onClose,
}: {
  s: State;
  gpu: Gpu;
  p: ReturnType<typeof useProfileActions>;
  onClose: () => void;
}) {
  const [tab, setTab] = useState<"profiles" | "games">("profiles");
  const profiles = profilesFor(s, gpu.id);
  return (
    <Modal title={`${gpu.name} · profiles`} onClose={onClose} wide>
      <div className="mb-4 flex gap-1 border-b border-border">
        {(["profiles", "games"] as const).map((t) => (
          <button
            key={t}
            type="button"
            onClick={() => setTab(t)}
            className={cn(
              "-mb-px border-b-2 px-3 py-1.5 text-sm capitalize",
              tab === t ? "border-primary text-foreground" : "border-transparent text-muted-foreground",
            )}
          >
            {t}
          </button>
        ))}
      </div>
      {tab === "profiles" && (
        <div className="space-y-2">
          {profiles.map((profile) => (
            <div key={profile.id} className="flex flex-wrap items-center gap-3 rounded-md border border-border/70 px-3 py-2 text-sm">
              <div className="min-w-0 flex-1">
                <p className="font-medium">{profile.name}</p>
                <p className="text-xs text-muted-foreground">{valuesSummary(profile.values)}</p>
              </div>
              <label className="flex items-center gap-1.5 text-xs">
                <input
                  type="radio"
                  name={`${gpu.id}-startup`}
                  checked={gpu.startupProfileId === profile.id}
                  onChange={() => actions.setStartup(gpu.id, profile.id)}
                />
                Startup profile
              </label>
              <Button size="xs" variant="ghost" onClick={() => p.rename(profile.id)}>
                Rename
              </Button>
              <Button size="xs" variant="destructive" onClick={() => p.remove(profile.id)}>
                Delete
              </Button>
            </div>
          ))}
          <div className="flex items-center justify-between pt-2 text-xs text-muted-foreground">
            <button type="button" className="underline" onClick={() => actions.setStartup(gpu.id, null)}>
              No startup profile
            </button>
            <span>
              Startup behavior (all GPUs): <b>{startupLabel(s)}</b> ·{" "}
              <Link to="/settings" className="text-primary underline">
                change in Settings
              </Link>
            </span>
          </div>
        </div>
      )}
      {tab === "games" && (
        <div className="space-y-3">
          <label className="flex items-center gap-2 text-sm">
            <input type="checkbox" checked={s.autoEnabled} onChange={(e) => actions.setAutoEnabled(e.target.checked)} />
            Switch profiles automatically while my games run
          </label>
          {s.games.map((game) => (
            <div key={game.id} className="flex items-center gap-3 rounded-md border border-border/70 px-3 py-2 text-sm">
              <div className="min-w-0 flex-1">
                <p className="font-medium">
                  {game.name} {!game.installed && <Badge variant="destructive">Not installed</Badge>}
                </p>
                <p className="truncate text-xs text-muted-foreground">{game.detail}</p>
              </div>
              <select
                className="h-7 rounded-sm border border-input bg-background px-2 text-xs"
                value={game.links[gpu.id] ?? ""}
                onChange={(e) => actions.link(game.id, gpu.id, e.target.value || null)}
              >
                <option value="">Don't change this GPU</option>
                {profiles.map((pr) => (
                  <option key={pr.id} value={pr.id}>
                    {pr.name}
                  </option>
                ))}
              </select>
            </div>
          ))}
          <div className="flex gap-2">
            <Button size="sm" variant="outline" onClick={() => actions.addGame("Hollow Knight: Silksong", "steam", "Steam app 1030300")}>
              Add Steam game…
            </Button>
            <Button size="sm" variant="outline" onClick={() => actions.addGame("RetroArch", "manual", "/usr/bin/retroarch")}>
              Add running program…
            </Button>
          </div>
          <p className="text-xs text-muted-foreground">
            Associations are per GPU. A game linked on another GPU is managed from that GPU's menu.
          </p>
        </div>
      )}
    </Modal>
  );
}

function StartupNoticeA({ gpu }: { gpu: Gpu }) {
  const o = gpu.startupOutcome!;
  return (
    <div className={cn("flex items-start gap-3 border-b px-5 py-2.5 text-sm", o.ok ? toneClass.neutral : toneClass.danger)}>
      <span className="flex-1">
        <b>At startup:</b> {o.summary}. <span className="text-xs opacity-80">{o.lines.join(" · ")}</span>
      </span>
      <Button size="xs" variant="ghost" onClick={() => actions.dismissStartupOutcome(gpu.id)}>
        Dismiss
      </Button>
    </div>
  );
}

export function ProfilesPageA() {
  return (
    <div className="mx-auto max-w-xl space-y-3 py-16 text-center">
      <h1 className="text-2xl font-semibold">No Profiles page in variant A</h1>
      <p className="text-sm text-muted-foreground">
        The sidebar entry would be removed. Profiles, startup profiles, and game associations are reached from each
        GPU's profile menu on Performance (<i>Manage profiles &amp; games…</i>). Startup behavior lives in Settings.
      </p>
      <Link className="text-sm text-primary underline" to="/performance">
        Back to Performance
      </Link>
    </div>
  );
}

export function startupLabel(s: State) {
  return { off: "Off", startupProfiles: "Startup profiles", restoreLast: "Restore last applied" }[s.startupBehavior];
}


