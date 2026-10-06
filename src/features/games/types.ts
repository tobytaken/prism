/** What identifies a game: a Steam app by its app ID, natively or through Proton. */
export type GameId = { kind: "steam"; appId: number };

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
