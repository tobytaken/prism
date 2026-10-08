import { useEffect } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { AutomaticSwitchingStatus } from "@/features/games/types";

const SWITCHING_STATUS_QUERY_KEY = ["automatic-switching-status"];

/** The status, or why the app can't know it, such as while the daemon is disconnected. */
export type ShownSwitchingStatus =
  | AutomaticSwitchingStatus
  | { condition: { status: "unknown"; reason: string }; gpus: [] };

/** A session snapshot followed by pushed changes; reconnecting refreshes the snapshot. */
export function useSwitchingStatus(): ShownSwitchingStatus | undefined {
  const client = useQueryClient();
  const query = useQuery<ShownSwitchingStatus>({
    queryKey: SWITCHING_STATUS_QUERY_KEY,
    queryFn: () => window.prism.games.getSwitchingStatus(),
  });
  useEffect(() => {
    const update = (status: ShownSwitchingStatus) => {
      // An older in-flight snapshot must not overwrite a newer push.
      void client.cancelQueries({ queryKey: SWITCHING_STATUS_QUERY_KEY });
      client.setQueryData(SWITCHING_STATUS_QUERY_KEY, status);
    };
    const unsubscribeStatus =
      window.prism.games.onSwitchingStatusChanged(update);
    const unsubscribeDaemon = window.prism.gpu.onDaemonStatusChanged(
      (daemon) => {
        if (daemon.reachable && daemon.managedByDaemon)
          void client.invalidateQueries({
            queryKey: SWITCHING_STATUS_QUERY_KEY,
          });
        else
          update({
            condition: {
              status: "unknown",
              reason: "The Prism daemon is disconnected.",
            },
            gpus: [],
          });
      },
    );
    return () => {
      unsubscribeStatus();
      unsubscribeDaemon();
    };
  }, [client]);
  if (query.error)
    return {
      condition: { status: "unknown", reason: query.error.message },
      gpus: [],
    };
  return query.data;
}

/** Retry is an explicit request, never a timer or mutation retry loop. */
export function useRetrySwitching() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (gpuId: string) => window.prism.games.retrySwitching({ gpuId }),
    retry: false,
    onSuccess: () =>
      client.invalidateQueries({ queryKey: SWITCHING_STATUS_QUERY_KEY }),
  });
}
