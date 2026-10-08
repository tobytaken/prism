/** What identifies a game: a Steam app ID or a manually identified program location. */
export type GameId =
  | { kind: "steam"; appId: number }
  | { kind: "program"; path: string };

export interface RunningProgram {
  pid: number;
  name: string;
  path: string | null;
  launcher: string | null;
  environment: string[];
  helper: boolean;
  recognitionWarning: string | null;
}

/** A fully installed Steam app from the caller's Steam libraries. */
export interface SteamApp {
  appId: number;
  /** From the app's manifest. */
  name: string;
}

/** One of the caller's game associations. */
export interface GameEntry {
  game: GameId;
  name: string;
  /** Whether the game is installed now; the association is kept while it isn't. */
  installed: boolean;
  /** The linked profile id per GPU UUID. GPUs without one are left unchanged. */
  profiles: Record<string, string>;
}

/** The caller's game associations and automatic switching opt-in. */
export interface GamesState {
  automaticSwitching: boolean;
  games: GameEntry[];
}

export interface AddSteamGameRequest {
  appId: number;
}

export interface SetGameProfileRequest {
  game: GameId;
  gpuId: string;
  /** Null leaves the GPU unchanged while the game runs. */
  profileId: string | null;
}

export interface GameRequest {
  game: GameId;
}

export interface SetAutomaticSwitchingRequest {
  enabled: boolean;
}

/** A recognized game, using the earliest recognized process on this host boot. */
export interface RunningGame {
  game: GameId;
  pid: number;
  /** Kernel clock ticks since boot, not wall-clock time. */
  startTimeTicks: number;
}

/** An unavailable scan does not establish whether any game ended. */
export type RunningGamesState =
  | { status: "available"; games: RunningGame[] }
  | { status: "unavailable"; reason: string };

/** What holds automatic switching back for the caller overall. */
export type SwitchingCondition =
  | { status: "ready" }
  | { status: "off" }
  | { status: "blocked"; reason: string }
  | { status: "recognitionUnavailable"; reason: string };

/** Why automation drives a GPU, or why its latest automatic operation didn't complete. */
export type GpuSwitchingState =
  | {
      status: "active";
      game: GameId;
      gameName: string;
      profile: { id: string; name: string };
    }
  | { status: "anotherUser" }
  | { status: "manualOverride"; changedOutsidePrism: boolean }
  | { status: "recoveryRequired"; message: string }
  | { status: "failed"; message: string };

/**
 * The caller's automatic switching status. GPUs without an entry are left unchanged; it doesn't
 * establish which tuning values are in effect.
 */
export interface AutomaticSwitchingStatus {
  condition: SwitchingCondition;
  gpus: { gpuId: string; state: GpuSwitchingState }[];
}
