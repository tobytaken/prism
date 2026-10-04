// PROTOTYPE (#7) — shared pieces whose behavior is already settled (#4, #6): dialogs,
// status wording, the tuning editor, the discard guard. Layout lives in each variant.
import { useState, type ReactNode } from "react";
import { useBlocker } from "react-router-dom";
import { X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import {
  actions,
  changesNotSaved,
  gameById,
  hasAnyEdits,
  hasProfileEdits,
  profileById,
  profilesFor,
  SETTING_KEYS,
  SETTING_LABEL,
  type Gpu,
  type Outcome,
  type SettingKey,
  type State,
} from "./profile-prototype-store";

export function fmt(key: SettingKey, value: number) {
  const { unit } = SETTING_LABEL[key];
  if (key === "power") return `${value} ${unit}`;
  return `${value > 0 ? "+" : ""}${value} ${unit}`;
}

export function valuesSummary(values: Record<SettingKey, number>) {
  return SETTING_KEYS.map((k) => fmt(k, values[k])).join(" · ");
}

// ---------------------------------------------------------------------------
// Automatic switching status wording

export type Tone = "neutral" | "active" | "warning" | "danger" | "muted";
export type StatusAction = "pause" | "resume" | "retry" | "applyEdits" | "discardEdits";
export type AutoStatus = {
  tone: Tone;
  short: string;
  title: string;
  detail: string;
  actions: StatusAction[];
  outcomes?: { heading: string; items: Outcome[] }[];
};

export function autoStatus(s: State, gpu: Gpu): AutoStatus {
  const a = gpu.auto;
  const game = (id: string | null) => gameById(s, id)?.name ?? "a game";
  const profile = (id: string | null) => profileById(s, id)?.name ?? "?";
  const hasLinks = s.games.some((g) => g.links[gpu.id]);

  if (!s.autoEnabled && a.kind !== "paused") {
    const running = s.running.map((r) => gameById(s, r.gameId)).find((g) => g?.links[gpu.id]);
    return {
      tone: "muted",
      short: "Auto off",
      title: "Automatic switching is off",
      detail: running
        ? `${running.name} is running, but automatic switching is turned off for you.`
        : "Game associations are kept but do nothing until you turn it on.",
      actions: [],
    };
  }
  switch (a.kind) {
    case "idle":
      return {
        tone: "neutral",
        short: hasLinks ? "Auto ready" : "No games linked",
        title: hasLinks ? "Automatic switching ready" : "No games linked to this GPU",
        detail: hasLinks
          ? "No associated game is running."
          : "Link a game to one of this GPU's profiles to switch automatically.",
        actions: hasLinks ? ["pause"] : [],
      };
    case "active":
      return {
        tone: "active",
        short: `${game(a.gameId)} → ${profile(a.profileId)}`,
        title: `${game(a.gameId)} is running → "${profile(a.profileId)}" applied automatically`,
        detail: `When it ends, Prism restores the pre-game values${
          gpu.baseline
            ? ` (${valuesSummary(gpu.baseline.values)}${
                profileById(s, gpu.baseline.previousProfileId)
                  ? `, "${profile(gpu.baseline.previousProfileId)}"`
                  : ""
              })`
            : ""
        }.`,
        actions: ["pause"],
      };
    case "waiting":
      return a.profileId
        ? {
            tone: "warning",
            short: "Waiting for your changes",
            title: `Automatic profile "${profile(a.profileId)}" is waiting for unapplied tuning changes.`,
            detail: `${game(a.gameId)} is running. "${profile(a.profileId)}" has not been applied. Applying your changes overrides it until the game ends; discarding them lets it apply.`,
            actions: ["applyEdits", "discardEdits"],
          }
        : {
            tone: "warning",
            short: "Restore waiting",
            title: "Restoring pre-game values is waiting for unapplied tuning changes.",
            detail: "The game has ended. Apply or discard your changes to continue.",
            actions: ["applyEdits", "discardEdits"],
          };
    case "override":
      return {
        tone: "warning",
        short: "Manual override",
        title: "Manual override: automatic switching paused for this GPU",
        detail: `You changed tuning while ${game(a.gameId)} was running. Your values stay until its associated games end, and nothing is restored afterward.`,
        actions: ["resume"],
      };
    case "paused":
      return {
        tone: "muted",
        short: "Paused",
        title: "Automatic switching paused for this GPU",
        detail: "Current values are kept. Nothing changes automatically until you resume, including after restarts.",
        actions: ["resume"],
      };
    case "failed":
      return {
        tone: "danger",
        short: "Failed · retry needed",
        title: `Automatic profile "${profile(a.profileId)}" was only partly applied`,
        detail: `For ${game(a.gameId)}. Prism tried to put back the previous values and stopped automatic changes on this GPU until you retry. Check the current values below.`,
        actions: ["retry"],
        outcomes: [
          { heading: "Application", items: a.outcomes },
          { heading: "Recovery", items: a.recovery },
        ],
      };
    case "blocked":
      return {
        tone: "danger",
        short: "Blocked",
        title: `Automatic profile "${profile(a.profileId)}" was not applied`,
        detail: `${game(a.gameId)} is running. ${a.reason} Update the profile to apply it.`,
        actions: [],
      };
    case "recognitionLost":
      return {
        tone: "warning",
        short: "Recognition unavailable",
        title: "Game recognition is unavailable",
        detail: "Prism can't tell which games are running, so it keeps the current values and won't restore until recognition is reliable again.",
        actions: [],
      };
  }
}

export const toneClass: Record<Tone, string> = {
  neutral: "border-border/80 bg-muted/20 text-muted-foreground",
  active: "border-primary/50 bg-primary/10 text-foreground",
  warning: "border-amber-400/50 bg-amber-400/10 text-amber-100",
  danger: "border-destructive/50 bg-destructive/12 text-destructive-foreground",
  muted: "border-border/80 bg-muted/10 text-muted-foreground",
};
export const toneDot: Record<Tone, string> = {
  neutral: "bg-muted-foreground/60",
  active: "bg-primary",
  warning: "bg-amber-400",
  danger: "bg-destructive",
  muted: "bg-muted-foreground/40",
};

const actionLabel: Record<StatusAction, string> = {
  pause: "Pause",
  resume: "Resume",
  retry: "Retry",
  applyEdits: "Apply my changes",
  discardEdits: "Discard my changes",
};

export function StatusActions({ gpu, status, size = "sm" }: { gpu: Gpu; status: AutoStatus; size?: "xs" | "sm" }) {
  return (
    <>
      {status.actions.map((action) => (
        <Button
          key={action}
          size={size}
          variant={action === "retry" || action === "resume" ? "default" : "outline"}
          onClick={() => {
            if (action === "pause") actions.pause(gpu.id);
            if (action === "resume") actions.resume(gpu.id);
            if (action === "retry") actions.retry(gpu.id);
            if (action === "applyEdits") actions.applyEdits(gpu.id);
            if (action === "discardEdits") actions.discardEdits(gpu.id);
          }}
        >
          {actionLabel[action]}
        </Button>
      ))}
    </>
  );
}

export function OutcomeList({ status }: { status: AutoStatus }) {
  if (!status.outcomes) return null;
  return (
    <div className="grid gap-3 sm:grid-cols-2">
      {status.outcomes.map((group) => (
        <div key={group.heading}>
          <p className="text-[11px] font-medium tracking-[0.18em] uppercase opacity-70">{group.heading}</p>
          <ul className="mt-1 space-y-0.5 text-xs">
            {group.items.map((item) => (
              <li key={item.setting} className={item.ok ? "" : "text-destructive"}>
                {item.ok ? "✓" : "✕"} {SETTING_LABEL[item.setting].label}: {item.message}
              </li>
            ))}
          </ul>
        </div>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Dialogs

export function Modal({
  title,
  children,
  onClose,
  wide,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  wide?: boolean;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6" onClick={onClose}>
      <div
        role="dialog"
        aria-label={title}
        className={cn(
          "max-h-[85vh] w-full overflow-y-auto rounded-lg border border-border bg-card p-5 shadow-2xl",
          wide ? "max-w-3xl" : "max-w-md",
        )}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-3 flex items-center justify-between gap-3">
          <h2 className="text-base font-semibold">{title}</h2>
          <Button size="icon-sm" variant="ghost" aria-label="Close" onClick={onClose}>
            <X />
          </Button>
        </div>
        {children}
      </div>
    </div>
  );
}

function NameDialog({
  s,
  gpu,
  title,
  initial,
  submitLabel,
  ignoreId,
  onSubmit,
  onClose,
}: {
  s: State;
  gpu: Gpu;
  title: string;
  initial: string;
  submitLabel: string;
  ignoreId?: string;
  onSubmit: (name: string) => void;
  onClose: () => void;
}) {
  const [name, setName] = useState(initial);
  const trimmed = name.trim();
  const clash = profilesFor(s, gpu.id).find(
    (p) => p.id !== ignoreId && p.name.toLowerCase() === trimmed.toLowerCase(),
  );
  return (
    <Modal title={title} onClose={onClose}>
      <form
        className="space-y-3"
        onSubmit={(e) => {
          e.preventDefault();
          if (!trimmed || clash) return;
          onSubmit(trimmed);
          onClose();
        }}
      >
        <Input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. Quiet" />
        {clash && (
          <p className="text-sm text-destructive">
            {gpu.name} already has a profile named "{clash.name}". Names are unique per GPU, ignoring case.
          </p>
        )}
        <p className="text-xs text-muted-foreground">Captures the applied values: {valuesSummary(gpu.reported)}</p>
        <div className="flex justify-end gap-2">
          <Button type="button" variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={!trimmed || !!clash}>
            {submitLabel}
          </Button>
        </div>
      </form>
    </Modal>
  );
}

/**
 * Everything a profile selector needs: choosing (with the settled blocking and
 * save-before-switch rules), save, save as, rename, delete, startup designation.
 * Each variant decides where these affordances live.
 */
export function useProfileActions(s: State, gpu: Gpu) {
  const [dialog, setDialog] = useState<ReactNode>(null);
  const close = () => setDialog(null);
  const current = profileById(s, gpu.currentProfileId);
  const unsaved = changesNotSaved(s, gpu);
  const editBlock = hasAnyEdits(gpu)
    ? "Apply or discard unapplied tuning changes before switching profiles."
    : null;
  const saveBlock = hasProfileEdits(gpu)
    ? "Apply or discard power-limit or clock-offset changes before saving."
    : null;

  function choose(profileId: string) {
    if (editBlock || profileId === gpu.currentProfileId) return;
    const target = profileById(s, profileId)!;
    if (current && unsaved.length > 0) {
      setDialog(
        <Modal title={`"${current.name}" has changes not saved to it`} onClose={close}>
          <p className="text-sm text-muted-foreground">
            The applied values differ from "{current.name}" ({unsaved.map((k) => SETTING_LABEL[k].label.toLowerCase()).join(", ")}).
            Switching to "{target.name}" replaces them.
          </p>
          <div className="mt-4 flex flex-wrap justify-end gap-2">
            <Button variant="outline" onClick={close}>
              Cancel
            </Button>
            <Button variant="outline" onClick={() => (actions.applyProfile(gpu.id, profileId), close())}>
              Switch without saving
            </Button>
            <Button onClick={() => (actions.applyProfile(gpu.id, profileId, true), close())}>
              Save to "{current.name}" and switch
            </Button>
          </div>
        </Modal>,
      );
      return;
    }
    actions.applyProfile(gpu.id, profileId);
  }

  function saveAsNew() {
    setDialog(
      <NameDialog
        s={s}
        gpu={gpu}
        title="Save as new profile"
        initial=""
        submitLabel="Save"
        onSubmit={(name) => actions.saveAsNew(gpu.id, name)}
        onClose={close}
      />,
    );
  }
  function rename(profileId: string) {
    const p = profileById(s, profileId)!;
    setDialog(
      <NameDialog
        s={s}
        gpu={gpu}
        title={`Rename "${p.name}"`}
        initial={p.name}
        submitLabel="Rename"
        ignoreId={p.id}
        onSubmit={(name) => actions.rename(p.id, name)}
        onClose={close}
      />,
    );
  }
  function remove(profileId: string) {
    const p = profileById(s, profileId)!;
    const games = s.games.filter((g) => Object.values(g.links).includes(p.id));
    setDialog(
      <Modal title={`Delete "${p.name}"?`} onClose={close}>
        <ul className="list-disc space-y-1 pl-5 text-sm text-muted-foreground">
          <li>The GPU's current tuning values stay as they are.</li>
          {gpu.currentProfileId === p.id && <li>This GPU will have no current profile.</li>}
          {gpu.startupProfileId === p.id && (
            <li className="text-amber-200">It is this GPU's startup profile; nothing will be applied to it at startup.</li>
          )}
          {games.length > 0 && <li>Removes its association with {games.map((g) => g.name).join(", ")}.</li>}
        </ul>
        <div className="mt-4 flex justify-end gap-2">
          <Button variant="outline" onClick={close}>
            Cancel
          </Button>
          <Button variant="destructive" onClick={() => (actions.remove(p.id), close())}>
            Delete
          </Button>
        </div>
      </Modal>,
    );
  }

  return {
    current,
    unsaved,
    editBlock,
    saveBlock,
    choose,
    save: () => actions.saveToCurrent(gpu.id),
    saveAsNew,
    rename,
    remove,
    toggleStartup: (profileId: string) =>
      actions.setStartup(gpu.id, gpu.startupProfileId === profileId ? null : profileId),
    dialog,
  };
}

/** Shown when saving over the startup profile (#4). */
export function startupSaveNote(gpu: Gpu) {
  return gpu.currentProfileId && gpu.currentProfileId === gpu.startupProfileId
    ? "Also changes the values applied at startup."
    : null;
}

// ---------------------------------------------------------------------------
// Tuning editor (existing behavior; not what's being decided here)

export function TuningEditor({
  s,
  gpu,
  showSavedMarkers = false,
  compact = false,
}: {
  s: State;
  gpu: Gpu;
  showSavedMarkers?: boolean;
  compact?: boolean;
}) {
  const current = profileById(s, gpu.currentProfileId);
  return (
    <div className={cn("space-y-4", compact && "space-y-3")}>
      {SETTING_KEYS.map((key) => {
        const [min, max] = gpu.limits[key];
        const edit = gpu.edits[key];
        const value = edit ?? gpu.reported[key];
        const saved = current?.values[key];
        const differs = saved !== undefined && saved !== gpu.reported[key];
        return (
          <div key={key} className="space-y-1.5">
            <div className="flex items-baseline justify-between gap-2 text-sm">
              <span className="font-medium">{SETTING_LABEL[key].label}</span>
              <span className="flex items-baseline gap-2 tabular-nums">
                {edit !== undefined && (
                  <span className="text-xs text-amber-300">unapplied · reported {fmt(key, gpu.reported[key])}</span>
                )}
                <span className={cn("font-semibold", edit !== undefined && "text-amber-300")}>{fmt(key, value)}</span>
              </span>
            </div>
            <div className="relative">
              <input
                type="range"
                min={min}
                max={max}
                step={key === "power" ? 5 : 10}
                value={value}
                className="w-full accent-[var(--primary)]"
                onChange={(e) => {
                  const next = Number(e.target.value);
                  actions.setEdit(gpu.id, key, next === gpu.reported[key] ? undefined : next);
                }}
              />
              {showSavedMarkers && saved !== undefined && (
                <span
                  title={`Saved in "${current!.name}": ${fmt(key, saved)}`}
                  className={cn(
                    "pointer-events-none absolute top-0 h-full w-0.5 rounded",
                    differs ? "bg-amber-300" : "bg-foreground/40",
                  )}
                  style={{ left: `${((saved - min) / (max - min)) * 100}%` }}
                />
              )}
            </div>
            {showSavedMarkers && differs && (
              <p className="text-xs text-amber-200/90">
                "{current!.name}" has {fmt(key, saved!)} · reported {fmt(key, gpu.reported[key])}
              </p>
            )}
          </div>
        );
      })}
      <div className="flex items-center justify-between gap-2 text-sm">
        <span className="font-medium">Fan</span>
        <select
          className="h-7 rounded-sm border border-input bg-background px-2 text-xs"
          value={gpu.edits.fan ?? (gpu.fan === "auto" ? -1 : gpu.fan)}
          onChange={(e) => {
            const v = Number(e.target.value);
            const reported = gpu.fan === "auto" ? -1 : gpu.fan;
            actions.setEdit(gpu.id, "fan", v === reported ? undefined : v);
          }}
        >
          <option value={-1}>Automatic</option>
          <option value={50}>Manual 50 %</option>
          <option value={70}>Manual 70 %</option>
        </select>
      </div>
      <p className="text-[11px] text-muted-foreground">Fan settings are outside profile scope.</p>
      {hasAnyEdits(gpu) && (
        <div className="flex gap-2">
          <Button size="sm" onClick={() => actions.applyEdits(gpu.id)}>
            Apply all
          </Button>
          <Button size="sm" variant="outline" onClick={() => actions.discardEdits(gpu.id)}>
            Discard
          </Button>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// In-app navigation guard (#6): unapplied changes, including fan edits.

export function DiscardGuard({ s }: { s: State }) {
  const dirty = s.gpus.filter(hasAnyEdits);
  const blocker = useBlocker(
    ({ currentLocation, nextLocation }) =>
      dirty.length > 0 && currentLocation.pathname !== nextLocation.pathname,
  );
  if (blocker.state !== "blocked") return null;
  return (
    <Modal title="Discard unapplied tuning changes?" onClose={() => blocker.reset()}>
      <p className="text-sm text-muted-foreground">
        Leaving Performance discards the changes you haven't applied on {dirty.map((g) => g.name).join(" and ")}.
        {dirty.some(hasProfileEdits) && " Any automatic profile waiting for them will then be allowed to apply."}
      </p>
      <div className="mt-4 flex justify-end gap-2">
        <Button variant="outline" onClick={() => blocker.reset()}>
          Cancel
        </Button>
        <Button
          variant="destructive"
          onClick={() => {
            actions.discardAllEdits();
            blocker.proceed();
          }}
        >
          Continue
        </Button>
      </div>
    </Modal>
  );
}
