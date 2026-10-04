// PROTOTYPE (#7): three variants of profile controls on the existing /performance
// route (and /profiles), switchable via ?variant=A|B|C and the floating bar.
// Run: `bun run prototype:profiles`, or add ?variant=A to #/performance in dev.
import { useEffect, useState, type ReactNode } from "react";
import { useLocation, useNavigate, useSearchParams } from "react-router-dom";

import { PrototypeSwitcher } from "@/components/prototype-switcher";
import { GpuPerformancePage } from "@/features/gpus/components/gpu-performance-page";
import { PlaceholderPage } from "@/features/gpus/components/placeholder-page";
import {
  actions,
  exitPrototype,
  isPrototypeActive,
  PRESETS,
  profileById,
  setPrototypeVariant,
  useProto,
} from "./profile-prototype-store";
import { DiscardGuard, valuesSummary } from "./profile-prototype-shared";
import { ProfilesPageA, VariantA } from "./profile-prototype-variant-a";
import { ProfilesPageB, VariantB } from "./profile-prototype-variant-b";
import { ProfilesPageC, VariantC } from "./profile-prototype-variant-c";
import { GamesPageD, VariantD } from "./profile-prototype-variant-d";

const VARIANTS = [
  { key: "A", name: "Header menu · no Profiles page" },
  { key: "B", name: "Rail list · Profiles library" },
  { key: "C", name: "Game console · Games page" },
  { key: "D", name: "A + C console · Games page" },
];

installBrowserStub();

function usePrototypeVariant() {
  const [params] = useSearchParams();
  const fromUrl = params.get("variant");
  const { variant } = useProto();
  useEffect(() => setPrototypeVariant(fromUrl), [fromUrl]);
  const active =
    fromUrl != null || isPrototypeActive() || !!import.meta.env.VITE_PROFILE_PROTOTYPE;
  return active ? (fromUrl ?? variant) : null;
}

export function PerformancePrototypeRoute() {
  const variant = usePrototypeVariant();
  const { state } = useProto();
  if (!variant) return <GpuPerformancePage />;
  return (
    <Frame variant={variant}>
      <DiscardGuard s={state} />
      {variant === "A" && <VariantA s={state} />}
      {variant === "B" && <VariantB s={state} />}
      {variant === "C" && <VariantC s={state} />}
      {variant === "D" && <VariantD s={state} />}
    </Frame>
  );
}

export function ProfilesPrototypeRoute() {
  const variant = usePrototypeVariant();
  const { state } = useProto();
  if (!variant) return <PlaceholderPage title="Profiles" />;
  return (
    <Frame variant={variant}>
      {variant === "A" && <ProfilesPageA />}
      {variant === "B" && <ProfilesPageB s={state} />}
      {variant === "C" && <ProfilesPageC s={state} />}
      {variant === "D" && <GamesPageD s={state} />}
    </Frame>
  );
}

function Frame({ variant, children }: { variant: string; children: ReactNode }) {
  const navigate = useNavigate();
  const location = useLocation();
  return (
    <div className="pb-24">
      {children}
      <PrototypeSwitcher
        variants={VARIANTS}
        current={variant}
        onChange={(key) => navigate(`${location.pathname}?variant=${key}`, { replace: true })}
      >
        <SimulationPanel />
      </PrototypeSwitcher>
    </div>
  );
}

function SimulationPanel() {
  const { state: s, preset } = useProto();
  const [open, setOpen] = useState(false);
  const navigate = useNavigate();
  return (
    <>
      <select
        aria-label="Scenario"
        className="h-7 max-w-64 rounded-full border border-white/30 bg-black px-2 text-xs"
        value={preset}
        onChange={(e) => actions.loadPreset(e.target.value)}
      >
        {PRESETS.map((p) => (
          <option key={p.id} value={p.id}>
            {p.label}
          </option>
        ))}
      </select>
      <button type="button" className="rounded-full px-2 text-xs underline" onClick={() => setOpen((o) => !o)}>
        {open ? "Hide" : "Simulate"}
      </button>
      {open && (
        <div className="absolute bottom-full right-0 mb-2 max-h-[60vh] w-[30rem] space-y-3 overflow-y-auto rounded-lg border-2 border-fuchsia-400 bg-black p-3 text-xs text-white shadow-2xl">
          <div>
            <p className="mb-1 font-semibold text-fuchsia-300">Games (start / end)</p>
            <div className="flex flex-wrap gap-1">
              {s.games.map((g) => {
                const running = s.running.some((r) => r.gameId === g.id);
                return (
                  <button
                    key={g.id}
                    type="button"
                    onClick={() => actions.toggleGame(g.id)}
                    className={`rounded-full border px-2 py-0.5 ${running ? "border-emerald-400 bg-emerald-500/30" : "border-white/30"}`}
                  >
                    {running ? "■" : "▶"} {g.name}
                  </button>
                );
              })}
            </div>
          </div>
          <div className="flex flex-wrap gap-x-4 gap-y-1">
            <label className="flex items-center gap-1">
              <input type="checkbox" checked={s.autoEnabled} onChange={(e) => actions.setAutoEnabled(e.target.checked)} />
              Automatic switching on
            </label>
            <label className="flex items-center gap-1">
              <input type="checkbox" checked={s.failNext} onChange={(e) => actions.setFailNext(e.target.checked)} />
              Next automatic application partly fails
            </label>
            <label className="flex items-center gap-1">
              <input
                type="checkbox"
                checked={s.gpus[0].auto.kind === "recognitionLost"}
                onChange={(e) => actions.setRecognitionLost(s.gpus[0].id, e.target.checked)}
              />
              Recognition lost (GPU 0)
            </label>
          </div>
          <div>
            <p className="mb-1 font-semibold text-fuchsia-300">State</p>
            {s.gpus.map((g) => (
              <div key={g.id} className="mb-1 font-mono leading-snug">
                <b>{g.name}</b>
                <br />
                reported: {valuesSummary(g.reported)} · fan {g.fan}
                <br />
                current: {profileById(s, g.currentProfileId)?.name ?? "none"}
                {g.currentIsAutomatic ? " (automatic)" : ""} · startup: {profileById(s, g.startupProfileId)?.name ?? "none"}
                <br />
                auto: {g.auto.kind} · edits: {JSON.stringify(g.edits)}
                <br />
                baseline: {g.baseline ? `${valuesSummary(g.baseline.values)} / ${profileById(s, g.baseline.previousProfileId)?.name ?? "none"}` : "none"}
              </div>
            ))}
          </div>
          <div>
            <p className="mb-1 font-semibold text-fuchsia-300">Log</p>
            <ul className="font-mono text-white/80">
              {s.log.map((l, i) => (
                <li key={i}>{l}</li>
              ))}
            </ul>
          </div>
          <button
            type="button"
            className="underline"
            onClick={() => {
              exitPrototype();
              navigate("/performance");
            }}
          >
            Exit prototype (real Performance page)
          </button>
        </div>
      )}
    </>
  );
}

/** Lets the prototype open in a plain browser (renderer dev server) without Electron. */
function installBrowserStub() {
  if (typeof window === "undefined" || window.prism) return;
  const never = () => new Promise(() => undefined);
  const stub = new Proxy(
    {},
    {
      get: (_t, ns) =>
        ns === "window"
          ? { isMaximized: async () => false, onResized: () => () => undefined, minimize: never, maximize: never, unmaximize: never, close: never }
          : new Proxy({}, { get: () => () => Promise.reject(new Error("Prototype: no Electron bridge")) }),
    },
  );
  (window as unknown as { prism: unknown }).prism = stub;
}
