// PROTOTYPE (#7) Variant D — the user's pick: A's Performance page (profile menu in
// each GPU header) with C's Game tuning console on top and C's Games page in place
// of Profiles. The console is the single home for automatic switching status,
// Pause/Resume/Retry, and startup outcomes; GPU headers keep a short status chip.
import type { State } from "./profile-prototype-store";
import { PageTitle } from "./profile-prototype-chrome";
import { GpuPanelA } from "./profile-prototype-variant-a";
import { GameTuningConsole, ProfilesPageC } from "./profile-prototype-variant-c";

export function VariantD({ s }: { s: State }) {
  return (
    <div className="space-y-8">
      <PageTitle title="Performance" />
      <GameTuningConsole s={s} />
      {s.gpus.map((gpu) => (
        <GpuPanelA key={gpu.id} s={s} gpu={gpu} combined />
      ))}
    </div>
  );
}

export const GamesPageD = ProfilesPageC;
