import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

const MIGRATION = ["gpu-migration"];

/** Persistent upgrade state, scoped to the authenticated daemon user. */
export function useMigration() {
  const client = useQueryClient();
  const query = useQuery({
    queryKey: MIGRATION,
    queryFn: () => window.prism.gpu.getMigrationState(),
    refetchInterval: 1000,
    refetchIntervalInBackground: true,
  });
  const refresh = () =>
    Promise.all([
      client.invalidateQueries({ queryKey: MIGRATION }),
      client.invalidateQueries({ queryKey: ["gpu-profiles"] }),
      client.invalidateQueries({ queryKey: ["gpu-startup"] }),
    ]);
  const adopt = useMutation({
    mutationFn: (profileId: string) =>
      window.prism.gpu.adoptMigratedProfile({ profileId }),
    onSettled: refresh,
  });
  const dismiss = useMutation({
    mutationFn: async () => {
      const result = await window.prism.gpu.dismissMigrationNotice();
      if (!result.success)
        throw new Error(result.error ?? "Failed to dismiss migration notice.");
    },
    onSettled: refresh,
  });
  return { query, adopt, dismiss };
}
