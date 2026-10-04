// PROTOTYPE for "Prototype profile controls and decide the Profiles page role" (#7).
// Throwaway: in-memory stub of the profile and automatic-switching behavior settled
// in #4 and #6, so the UI variants can be driven through every state. No daemon calls.
import { useSyncExternalStore } from "react";

export type SettingKey = "power" | "graphics" | "memory";
export type Values = Record<SettingKey, number>;
export type Edits = Partial<Values> & { fan?: number };

export const SETTING_KEYS: SettingKey[] = ["power", "graphics", "memory"];
export const SETTING_LABEL: Record<SettingKey, { label: string; unit: string }> = {
  power: { label: "Power limit", unit: "W" },
  graphics: { label: "Graphics offset", unit: "MHz" },
  memory: { label: "Memory offset", unit: "MHz" },
};

export type Profile = { id: string; gpuId: string; name: string; values: Values };
export type Game = {
  id: string;
  name: string;
  source: "steam" | "manual";
  detail: string;
  installed: boolean;
  /** gpuId -> profileId */
  links: Record<string, string>;
};
export type Outcome = { setting: SettingKey; ok: boolean; message: string };

export type Auto =
  | { kind: "idle" }
  | { kind: "active"; gameId: string; profileId: string }
  /** profileId null = restoring pre-game values is what is waiting */
  | { kind: "waiting"; gameId: string | null; profileId: string | null }
  | { kind: "override"; gameId: string }
  | { kind: "paused" }
  | {
      kind: "failed";
      gameId: string;
      profileId: string;
      outcomes: Outcome[];
      recovery: Outcome[];
    }
  | { kind: "blocked"; gameId: string; profileId: string; reason: string }
  | { kind: "recognitionLost" };

export type StartupOutcome = {
  ok: boolean;
  summary: string;
  lines: string[];
};

export type Gpu = {
  id: string;
  name: string;
  busId: string;
  limits: Record<SettingKey, [number, number]>;
  reported: Values;
  fan: number | "auto";
  currentProfileId: string | null;
  currentIsAutomatic: boolean;
  startupProfileId: string | null;
  edits: Edits;
  auto: Auto;
  baseline: { values: Values; previousProfileId: string | null } | null;
  startupOutcome: StartupOutcome | null;
  lastResult: string | null;
};

export type State = {
  gpus: Gpu[];
  profiles: Profile[];
  games: Game[];
  running: { gameId: string; startedAt: number }[];
  autoEnabled: boolean;
  startupBehavior: "off" | "startupProfiles" | "restoreLast";
  failNext: boolean;
  clock: number;
  log: string[];
};

// ---------------------------------------------------------------------------
// Seed data

function seed(): State {
  return {
    gpus: [
      {
        id: "gpu-0",
        name: "NVIDIA GeForce RTX 4080",
        busId: "00000000:01:00.0",
        limits: { power: [150, 320], graphics: [-500, 250], memory: [-1000, 1500] },
        reported: { power: 220, graphics: 0, memory: 0 },
        fan: "auto",
        currentProfileId: "p-quiet",
        currentIsAutomatic: false,
        startupProfileId: "p-quiet",
        edits: {},
        auto: { kind: "idle" },
        baseline: null,
        startupOutcome: null,
        lastResult: null,
      },
      {
        id: "gpu-1",
        name: "NVIDIA GeForce RTX 3060",
        busId: "00000000:08:00.0",
        limits: { power: [100, 170], graphics: [-500, 200], memory: [-1000, 1200] },
        reported: { power: 170, graphics: 0, memory: 0 },
        fan: "auto",
        currentProfileId: null,
        currentIsAutomatic: false,
        startupProfileId: null,
        edits: {},
        auto: { kind: "idle" },
        baseline: null,
        startupOutcome: null,
        lastResult: null,
      },
    ],
    profiles: [
      { id: "p-quiet", gpuId: "gpu-0", name: "Quiet", values: { power: 220, graphics: 0, memory: 0 } },
      { id: "p-balanced", gpuId: "gpu-0", name: "Balanced", values: { power: 280, graphics: 100, memory: 500 } },
      { id: "p-perf", gpuId: "gpu-0", name: "Performance", values: { power: 320, graphics: 150, memory: 1000 } },
      { id: "p-silent", gpuId: "gpu-1", name: "Silent", values: { power: 120, graphics: 0, memory: 0 } },
      { id: "p-max", gpuId: "gpu-1", name: "Max", values: { power: 170, graphics: 120, memory: 800 } },
    ],
    games: [
      { id: "g-cp", name: "Cyberpunk 2077", source: "steam", detail: "Steam app 1091500", installed: true, links: { "gpu-0": "p-perf" } },
      { id: "g-er", name: "Elden Ring", source: "steam", detail: "Steam app 1245620", installed: true, links: { "gpu-0": "p-balanced" } },
      { id: "g-hades", name: "Hades II", source: "steam", detail: "Steam app 1145350", installed: true, links: { "gpu-0": "p-quiet" } },
      { id: "g-aw", name: "Alan Wake 2", source: "manual", detail: "~/Games/Heroic/AlanWake2/AlanWake2.exe", installed: true, links: { "gpu-0": "p-perf", "gpu-1": "p-max" } },
      { id: "g-bg3", name: "Baldur's Gate 3", source: "steam", detail: "Steam app 1086940", installed: true, links: {} },
      { id: "g-sf", name: "Starfield", source: "steam", detail: "Steam app 1716740 · not installed", installed: false, links: { "gpu-0": "p-perf" } },
    ],
    running: [],
    autoEnabled: true,
    startupBehavior: "startupProfiles",
    failNext: false,
    clock: 1,
    log: [],
  };
}

// ---------------------------------------------------------------------------
// Rules (simplified from the #6 resolution)

const isProfileKey = (key: string): key is SettingKey =>
  (SETTING_KEYS as string[]).includes(key);

export const hasProfileEdits = (gpu: Gpu) =>
  Object.keys(gpu.edits).some(isProfileKey);
export const hasAnyEdits = (gpu: Gpu) => Object.keys(gpu.edits).length > 0;

export function profileById(s: State, id: string | null | undefined) {
  return id ? s.profiles.find((p) => p.id === id) : undefined;
}
export function gameById(s: State, id: string | null | undefined) {
  return id ? s.games.find((g) => g.id === id) : undefined;
}
export function profilesFor(s: State, gpuId: string) {
  return s.profiles.filter((p) => p.gpuId === gpuId);
}
export function gamesLinkedTo(s: State, profileId: string) {
  return s.games.filter((g) => Object.values(g.links).includes(profileId));
}
export function changesNotSaved(s: State, gpu: Gpu): SettingKey[] {
  const current = profileById(s, gpu.currentProfileId);
  if (!current) return [];
  return SETTING_KEYS.filter((k) => current.values[k] !== gpu.reported[k]);
}
function associatedRunning(s: State, gpuId: string) {
  return s.running.filter((r) => {
    const game = gameById(s, r.gameId);
    return game?.installed && game.links[gpuId] && profileById(s, game.links[gpuId]);
  });
}
export function winnerFor(s: State, gpuId: string) {
  if (!s.autoEnabled) return undefined;
  const [top] = [...associatedRunning(s, gpuId)].sort((a, b) => b.startedAt - a.startedAt);
  if (!top) return undefined;
  const game = gameById(s, top.gameId)!;
  return { game, profile: profileById(s, game.links[gpuId])! };
}

function log(s: State, line: string) {
  s.log = [`${s.clock}: ${line}`, ...s.log].slice(0, 30);
  s.clock += 1;
}

function reconcile(s: State) {
  for (const gpu of s.gpus) {
    const { kind } = gpu.auto;
    if (kind === "paused" || kind === "failed" || kind === "recognitionLost") continue;
    if (kind === "override") {
      if (associatedRunning(s, gpu.id).length === 0) {
        gpu.auto = { kind: "idle" };
        gpu.baseline = null;
        log(s, `${gpu.name}: associated games ended, manual override cleared (values kept)`);
      }
      continue;
    }

    const winner = winnerFor(s, gpu.id);
    if (winner) {
      if (hasProfileEdits(gpu)) {
        gpu.auto = { kind: "waiting", gameId: winner.game.id, profileId: winner.profile.id };
        continue;
      }
      if (
        gpu.auto.kind === "active" &&
        gpu.auto.profileId === winner.profile.id &&
        gpu.auto.gameId === winner.game.id
      )
        continue;
      if (gpu.auto.kind === "active" && gpu.auto.profileId === winner.profile.id) {
        gpu.auto = { kind: "active", gameId: winner.game.id, profileId: winner.profile.id };
        continue;
      }
      const outOfRange = SETTING_KEYS.find((k) => {
        const [min, max] = gpu.limits[k];
        return winner.profile.values[k] < min || winner.profile.values[k] > max;
      });
      if (outOfRange) {
        const [, max] = gpu.limits[outOfRange];
        gpu.auto = {
          kind: "blocked",
          gameId: winner.game.id,
          profileId: winner.profile.id,
          reason: `${SETTING_LABEL[outOfRange].label} ${winner.profile.values[outOfRange]} ${SETTING_LABEL[outOfRange].unit} is above this GPU's current maximum of ${max} ${SETTING_LABEL[outOfRange].unit}. Nothing was changed on this GPU.`,
        };
        continue;
      }
      if (!gpu.baseline) {
        gpu.baseline = {
          values: { ...gpu.reported },
          previousProfileId: gpu.currentProfileId,
        };
      }
      if (s.failNext) {
        s.failNext = false;
        gpu.auto = {
          kind: "failed",
          gameId: winner.game.id,
          profileId: winner.profile.id,
          outcomes: [
            { setting: "power", ok: true, message: `Applied ${winner.profile.values.power} W` },
            { setting: "graphics", ok: true, message: `Applied +${winner.profile.values.graphics} MHz` },
            { setting: "memory", ok: false, message: "Driver rejected the memory offset" },
          ],
          recovery: [
            { setting: "power", ok: true, message: `Restored ${gpu.reported.power} W` },
            { setting: "graphics", ok: false, message: "Could not restore the previous graphics offset" },
          ],
        };
        gpu.reported = { ...gpu.reported, graphics: winner.profile.values.graphics };
        gpu.currentIsAutomatic = false;
        log(s, `${gpu.name}: automatic "${winner.profile.name}" partly failed, failure pause`);
        continue;
      }
      gpu.reported = { ...winner.profile.values };
      gpu.currentProfileId = winner.profile.id;
      gpu.currentIsAutomatic = true;
      gpu.auto = { kind: "active", gameId: winner.game.id, profileId: winner.profile.id };
      log(s, `${gpu.name}: applied "${winner.profile.name}" for ${winner.game.name}`);
      continue;
    }

    // No winner: restore, unless edits hold automatic changes.
    if (gpu.baseline && (kind === "active" || kind === "waiting" || kind === "blocked" || kind === "idle")) {
      if (hasProfileEdits(gpu)) {
        gpu.auto = { kind: "waiting", gameId: null, profileId: null };
        continue;
      }
      gpu.reported = { ...gpu.baseline.values };
      const previous = profileById(s, gpu.baseline.previousProfileId);
      gpu.currentProfileId = previous?.id ?? null;
      gpu.currentIsAutomatic = false;
      gpu.baseline = null;
      log(s, `${gpu.name}: restored pre-game values${previous ? ` ("${previous.name}")` : ""}`);
    }
    gpu.auto = { kind: "idle" };
  }
}

// ---------------------------------------------------------------------------
// Store

type Store = { state: State; variant: string; preset: string };
let store: Store = { state: seed(), variant: "A", preset: "manual-sync" };
let prototypeActive = false;
const listeners = new Set<() => void>();

function emit() {
  listeners.forEach((l) => l());
}
function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useProto() {
  return useSyncExternalStore(subscribe, () => store);
}

export function isPrototypeActive() {
  return prototypeActive;
}
export function setPrototypeVariant(variant: string | null) {
  if (variant == null) return;
  prototypeActive = true;
  if (store.variant !== variant) {
    store = { ...store, variant };
    emit();
  }
}
export function exitPrototype() {
  prototypeActive = false;
  emit();
}

function update(fn: (s: State, find: (id: string) => Gpu) => void) {
  const next = structuredClone(store.state);
  fn(next, (id) => next.gpus.find((g) => g.id === id)!);
  reconcile(next);
  store = { ...store, state: next };
  emit();
}

function manualWrite(s: State, gpu: Gpu, description: string) {
  if (gpu.auto.kind === "paused") {
    log(s, `${gpu.name}: ${description} (automatic switching stays paused)`);
    return;
  }
  if (associatedRunning(s, gpu.id).length > 0 && s.autoEnabled) {
    const top = [...associatedRunning(s, gpu.id)].sort((a, b) => b.startedAt - a.startedAt)[0];
    gpu.auto = { kind: "override", gameId: top.gameId };
    gpu.baseline = null;
    log(s, `${gpu.name}: ${description} → manual override until associated games end`);
    return;
  }
  gpu.auto = { kind: "idle" };
  gpu.baseline = null;
  log(s, `${gpu.name}: ${description}`);
}

export const actions = {
  setEdit(gpuId: string, key: SettingKey | "fan", value: number | undefined) {
    update((_s, find) => {
      const gpu = find(gpuId);
      if (value === undefined) delete gpu.edits[key];
      else gpu.edits[key] = value;
    });
  },
  discardEdits(gpuId: string) {
    update((s, find) => {
      find(gpuId).edits = {};
      log(s, `${find(gpuId).name}: discarded unapplied changes`);
    });
  },
  discardAllEdits() {
    update((s) => {
      s.gpus.forEach((g) => (g.edits = {}));
      log(s, "Left the tuning screen: unapplied changes discarded");
    });
  },
  applyEdits(gpuId: string) {
    update((s, find) => {
      const gpu = find(gpuId);
      const profileEdit = hasProfileEdits(gpu);
      const { fan, ...rest } = gpu.edits;
      gpu.reported = { ...gpu.reported, ...rest };
      if (fan !== undefined) gpu.fan = fan;
      gpu.edits = {};
      gpu.currentIsAutomatic = false;
      gpu.lastResult = "All changes applied.";
      if (profileEdit) manualWrite(s, gpu, "applied live tuning changes");
      else log(s, `${gpu.name}: applied fan change`);
    });
  },
  applyProfile(gpuId: string, profileId: string, saveCurrentFirst = false) {
    update((s, find) => {
      const gpu = find(gpuId);
      const profile = profileById(s, profileId)!;
      if (saveCurrentFirst) {
        const current = profileById(s, gpu.currentProfileId);
        if (current) current.values = { ...gpu.reported };
      }
      gpu.reported = { ...profile.values };
      gpu.currentProfileId = profile.id;
      gpu.currentIsAutomatic = false;
      gpu.lastResult = `Applied "${profile.name}".`;
      manualWrite(s, gpu, `applied "${profile.name}"`);
    });
  },
  saveToCurrent(gpuId: string) {
    update((s, find) => {
      const gpu = find(gpuId);
      const current = profileById(s, gpu.currentProfileId);
      if (!current) return;
      current.values = { ...gpu.reported };
      log(s, `${gpu.name}: saved current values to "${current.name}"`);
    });
  },
  saveAsNew(gpuId: string, name: string) {
    update((s, find) => {
      const gpu = find(gpuId);
      const id = `p-${Math.random().toString(36).slice(2, 7)}`;
      s.profiles.push({ id, gpuId, name, values: { ...gpu.reported } });
      gpu.currentProfileId = id;
      gpu.currentIsAutomatic = false;
      log(s, `${gpu.name}: saved new profile "${name}"`);
    });
  },
  rename(profileId: string, name: string) {
    update((s) => {
      const profile = profileById(s, profileId)!;
      log(s, `Renamed "${profile.name}" to "${name}"`);
      profile.name = name;
    });
  },
  remove(profileId: string) {
    update((s) => {
      const profile = profileById(s, profileId)!;
      s.profiles = s.profiles.filter((p) => p.id !== profileId);
      s.games.forEach((g) => {
        for (const [gpuId, pid] of Object.entries(g.links)) if (pid === profileId) delete g.links[gpuId];
      });
      s.gpus.forEach((g) => {
        if (g.currentProfileId === profileId) g.currentProfileId = null;
        if (g.startupProfileId === profileId) g.startupProfileId = null;
        if (g.baseline?.previousProfileId === profileId) g.baseline.previousProfileId = null;
      });
      log(s, `Deleted "${profile.name}" and its game associations`);
    });
  },
  setStartup(gpuId: string, profileId: string | null) {
    update((s, find) => {
      find(gpuId).startupProfileId = profileId;
      log(s, `${find(gpuId).name}: startup profile → ${profileById(s, profileId)?.name ?? "none"}`);
    });
  },
  setStartupBehavior(value: State["startupBehavior"]) {
    update((s) => {
      s.startupBehavior = value;
    });
  },
  link(gameId: string, gpuId: string, profileId: string | null) {
    update((s) => {
      const game = gameById(s, gameId)!;
      if (profileId) game.links[gpuId] = profileId;
      else delete game.links[gpuId];
      log(s, `${game.name}: ${gpuId} → ${profileById(s, profileId)?.name ?? "no profile"}`);
    });
  },
  addGame(name: string, source: Game["source"], detail: string) {
    update((s) => {
      s.games.push({ id: `g-${Math.random().toString(36).slice(2, 7)}`, name, source, detail, installed: true, links: {} });
    });
  },
  removeGame(gameId: string) {
    update((s) => {
      s.games = s.games.filter((g) => g.id !== gameId);
      s.running = s.running.filter((r) => r.gameId !== gameId);
    });
  },
  setAutoEnabled(enabled: boolean) {
    update((s) => {
      s.autoEnabled = enabled;
      log(s, `Automatic switching ${enabled ? "enabled" : "disabled"}`);
    });
  },
  pause(gpuId: string) {
    update((s, find) => {
      find(gpuId).auto = { kind: "paused" };
      log(s, `${find(gpuId).name}: automatic switching paused`);
    });
  },
  resume(gpuId: string) {
    update((s, find) => {
      find(gpuId).auto = { kind: "idle" };
      log(s, `${find(gpuId).name}: automatic switching resumed`);
    });
  },
  retry(gpuId: string) {
    update((s, find) => {
      find(gpuId).auto = { kind: "idle" };
      log(s, `${find(gpuId).name}: retry requested`);
    });
  },
  dismissStartupOutcome(gpuId: string) {
    update((_s, find) => {
      find(gpuId).startupOutcome = null;
    });
  },
  // Simulation controls
  toggleGame(gameId: string) {
    update((s) => {
      const game = gameById(s, gameId)!;
      if (s.running.some((r) => r.gameId === gameId)) {
        s.running = s.running.filter((r) => r.gameId !== gameId);
        log(s, `${game.name} ended`);
      } else {
        s.running.push({ gameId, startedAt: s.clock });
        log(s, `${game.name} started`);
      }
    });
  },
  setFailNext(value: boolean) {
    update((s) => {
      s.failNext = value;
    });
  },
  setRecognitionLost(gpuId: string, lost: boolean) {
    update((_s, find) => {
      find(gpuId).auto = lost ? { kind: "recognitionLost" } : { kind: "idle" };
    });
  },
  loadPreset(id: string) {
    const preset = PRESETS.find((p) => p.id === id)!;
    const s = seed();
    preset.build(s);
    reconcile(s);
    s.log = [`Scenario: ${preset.label}`];
    store = { ...store, state: s, preset: id };
    emit();
  },
};

// ---------------------------------------------------------------------------
// Scenario presets: each builds a consistent state by replaying steps on the seed.

function start(s: State, gameId: string) {
  s.running.push({ gameId, startedAt: s.clock });
  s.clock += 1;
  reconcile(s);
}

export const PRESETS: { id: string; label: string; build: (s: State) => void }[] = [
  { id: "manual-sync", label: "Manual · current profile in sync", build: () => {} },
  {
    id: "manual-unsaved",
    label: "Manual · changes not saved to profile",
    build: (s) => {
      s.gpus[0].reported = { power: 240, graphics: 0, memory: 100 };
    },
  },
  {
    id: "edits",
    label: "Unapplied tuning changes (no game)",
    build: (s) => {
      s.gpus[0].edits = { power: 260, memory: 300 };
    },
  },
  { id: "auto-active", label: "Game running · automatic profile applied", build: (s) => start(s, "g-cp") },
  {
    id: "auto-overlap",
    label: "Overlapping games · newest wins",
    build: (s) => {
      start(s, "g-cp");
      start(s, "g-aw");
      start(s, "g-er");
    },
  },
  {
    id: "waiting",
    label: "Automatic profile waiting for unapplied changes",
    build: (s) => {
      s.gpus[0].edits = { power: 260 };
      start(s, "g-cp");
    },
  },
  {
    id: "override",
    label: "Manual override during a game",
    build: (s) => {
      start(s, "g-cp");
      const gpu = s.gpus[0];
      gpu.reported = { power: 280, graphics: 100, memory: 500 };
      gpu.currentProfileId = "p-balanced";
      gpu.currentIsAutomatic = false;
      gpu.auto = { kind: "override", gameId: "g-cp" };
      gpu.baseline = null;
    },
  },
  {
    id: "paused",
    label: "Explicitly paused",
    build: (s) => {
      s.gpus[0].auto = { kind: "paused" };
      start(s, "g-cp");
    },
  },
  {
    id: "failed",
    label: "Failure pause · partial automatic application",
    build: (s) => {
      s.failNext = true;
      start(s, "g-cp");
    },
  },
  {
    id: "blocked",
    label: "Blocked · saved value out of range",
    build: (s) => {
      s.gpus[0].limits.power = [150, 300];
      start(s, "g-cp");
    },
  },
  {
    id: "recognition",
    label: "Game recognition unavailable",
    build: (s) => {
      start(s, "g-cp");
      s.gpus[0].auto = { kind: "recognitionLost" };
    },
  },
  {
    id: "startup",
    label: "First open after boot · startup outcome",
    build: (s) => {
      s.gpus[0].startupOutcome = {
        ok: true,
        summary: 'Startup profile "Quiet" applied',
        lines: ["Power limit 220 W applied", "Graphics offset +0 MHz applied", "Memory offset +0 MHz applied"],
      };
      s.gpus[1].startupProfileId = "p-max";
      s.gpus[1].startupOutcome = {
        ok: false,
        summary: 'Startup profile "Max" was not applied',
        lines: ["Power limit 170 W is above the current maximum of 160 W", "Nothing was changed on this GPU"],
      };
    },
  },
  {
    id: "auto-off",
    label: "Automatic switching off · game running",
    build: (s) => {
      s.autoEnabled = false;
      start(s, "g-cp");
    },
  },
];
