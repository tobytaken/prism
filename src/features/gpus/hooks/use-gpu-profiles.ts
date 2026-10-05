import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { listGpuProfiles, saveGpuProfile } from "@/features/gpus/api";

export function useGpuProfiles() {
  const queryClient = useQueryClient();
  const [selectedIds, setSelectedIds] = useState<Record<string, string>>({});
  const query = useQuery({
    queryKey: ["gpu-profiles"],
    queryFn: listGpuProfiles,
  });
  const save = useMutation({
    mutationFn: saveGpuProfile,
    onSuccess: async (profile) => {
      setSelectedIds((current) => ({
        ...current,
        [profile.gpuId]: profile.id,
      }));
      await queryClient.invalidateQueries({ queryKey: ["gpu-profiles"] });
    },
  });
  return {
    query,
    save,
    selectedIds,
    select(gpuId: string, profileId: string) {
      setSelectedIds((current) => ({ ...current, [gpuId]: profileId }));
    },
    /** Reloads the profiles after the daemon changed one, such as by saving corrections. */
    refresh() {
      return queryClient.invalidateQueries({ queryKey: ["gpu-profiles"] });
    },
  };
}

export type GpuProfileWorkflow = ReturnType<typeof useGpuProfiles>;
