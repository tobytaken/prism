import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import type {
  GameEntry,
  GamesState,
  SetGameProfileRequest,
  GameId,
} from "@/features/games/types";

export const GAMES_QUERY_KEY = ["games"];

/** The caller's game associations and automatic switching opt-in, from the daemon. */
export function useGames() {
  const client = useQueryClient();
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
  const setProfile = useMutation({
    mutationFn: (request: SetGameProfileRequest) =>
      window.prism.games.setProfile(request),
    ...changed,
  });
  const remove = useMutation({
    mutationFn: (game: GameId) => window.prism.games.remove({ game }),
    ...changed,
  });
  const setAutomaticSwitching = useMutation({
    mutationFn: (enabled: boolean) =>
      window.prism.games.setAutomaticSwitching({ enabled }),
    ...changed,
  });
  return {
    query,
    addSteamGame,
    setProfile,
    remove,
    setAutomaticSwitching,
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
  return `${game.kind}:${game.appId}`;
}
