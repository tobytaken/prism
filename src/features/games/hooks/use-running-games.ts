import { useEffect } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { RunningGamesState } from "@/features/games/types";

const RUNNING_GAMES_QUERY_KEY = ["running-games"];

/** A session snapshot followed by pushed changes; reconnecting refreshes the snapshot. */
export function useRunningGames(): RunningGamesState | undefined {
  const client = useQueryClient();
  const query = useQuery({
    queryKey: RUNNING_GAMES_QUERY_KEY,
    queryFn: () => window.prism.games.getRunning(),
  });
  useEffect(() => {
    const update = (state: RunningGamesState) => {
      // An older in-flight snapshot must not overwrite a newer push.
      void client.cancelQueries({ queryKey: RUNNING_GAMES_QUERY_KEY });
      client.setQueryData(RUNNING_GAMES_QUERY_KEY, state);
    };
    const unsubscribeGames = window.prism.games.onRunningChanged(update);
    const unsubscribeStatus = window.prism.gpu.onDaemonStatusChanged(
      (status) => {
        if (status.reachable && status.managedByDaemon)
          void client.invalidateQueries({ queryKey: RUNNING_GAMES_QUERY_KEY });
        else
          update({
            status: "unavailable",
            reason: "The Prism daemon is disconnected.",
          });
      },
    );
    return () => {
      unsubscribeGames();
      unsubscribeStatus();
    };
  }, [client]);
  if (query.error)
    return { status: "unavailable", reason: query.error.message };
  return query.data;
}
