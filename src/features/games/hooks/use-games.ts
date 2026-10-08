import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import type {
  GameEntry,
  GamesState,
  SetGameProfileRequest,
  GameId,
} from "@/features/games/types";

export const GAMES_QUERY_KEY = ["games"];

type Change = "setProfile" | "remove" | "setAutomaticSwitching";

/** The caller's game associations and automatic switching opt-in, from the daemon. */
export function useGames() {
  const client = useQueryClient();
  const [latest, setLatest] = useState<Change | null>(null);
  const query = useQuery({
    queryKey: GAMES_QUERY_KEY,
    queryFn: () => window.prism.games.get(),
  });
  // Every change answers with the caller's resulting games.
  const changed = {
    onSuccess: (state: GamesState) =>
      client.setQueryData(GAMES_QUERY_KEY, state),
    onError: () => client.invalidateQueries({ queryKey: GAMES_QUERY_KEY }),
  };
  const addSteamGame = useMutation({
    mutationFn: (appId: number) => window.prism.games.addSteamGame({ appId }),
    ...changed,
  });
  const addProgram = useMutation({
    mutationFn: (path: string) => window.prism.games.addProgram({ path }),
    ...changed,
  });
  const browseProgram = useMutation({
    mutationFn: () => window.prism.games.browseProgram(),
    onSuccess: (state) => {
      if (state) changed.onSuccess(state);
    },
    onError: changed.onError,
  });
  const setProfile = useMutation({
    mutationFn: (request: SetGameProfileRequest) =>
      window.prism.games.setProfile(request),
    onMutate: () => setLatest("setProfile"),
    ...changed,
  });
  const remove = useMutation({
    mutationFn: (game: GameId) => window.prism.games.remove({ game }),
    onMutate: () => setLatest("remove"),
    ...changed,
  });
  const setAutomaticSwitching = useMutation({
    mutationFn: (enabled: boolean) =>
      window.prism.games.setAutomaticSwitching({ enabled }),
    onMutate: () => setLatest("setAutomaticSwitching"),
    ...changed,
  });
  const changes = { setProfile, remove, setAutomaticSwitching };
  return {
    query,
    addSteamGame,
    addProgram,
    browseProgram,
    setProfile,
    remove,
    setAutomaticSwitching,
    /** The latest change's error, so an earlier failure isn't shown after a later change. */
    changeError: latest ? changes[latest].error : null,
    refresh: () => client.invalidateQueries({ queryKey: GAMES_QUERY_KEY }),
    /** The games linked to the profile on any GPU. */
    linkedTo(profileId: string): GameEntry[] {
      return (
        query.data?.games.filter((game) =>
          Object.values(game.profiles).includes(profileId),
        ) ?? []
      );
    },
  };
}

export type GamesWorkflow = ReturnType<typeof useGames>;

/** A stable key for a game, for lists and comparisons. */
export function gameKey(game: GameId) {
  return game.kind === "steam" ? `steam:${game.appId}` : `program:${game.path}`;
}
