import { useEffect, useRef, useState } from "react";
import type { GpuPerformanceDevice } from "@/features/gpus/types";
import type { TuningEditStore } from "./use-tuning-edits";

/** Drafts stay in Performance; only their GPU identities cross the desktop boundary. */
export function useEditHolds(
  store: TuningEditStore,
  gpus: GpuPerformanceDevice[],
) {
  const knownGpus = useRef(new Map<string, GpuPerformanceDevice>());
  const reported = useRef("");
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    for (const gpu of gpus) knownGpus.current.set(gpu.id, gpu);
    const gpuIds = [
      ...new Set(
        Object.entries(store.devices)
          .filter(([, device]) => Object.keys(device.edits).length > 0)
          .flatMap(([id]) => {
            const stableId = knownGpus.current.get(id)?.stableId;
            return stableId ? [stableId] : [];
          }),
      ),
    ].sort();
    const key = JSON.stringify(gpuIds);
    if (reported.current === key) return;
    reported.current = key;
    void reportHolds(gpuIds).then(
      () => setError(null),
      () =>
        setError(
          "Could not report unapplied tuning changes. Automatic switching may not be waiting for these edits.",
        ),
    );
  }, [store.devices, gpus]);
  useEffect(
    () => () => {
      reported.current = "";
      void reportHolds([]).catch(() => undefined);
    },
    [],
  );
  return { knownGpus: knownGpus.current, error };
}

// Keep releases and subsequent edits ordered, including when Performance remounts.
let reporting = Promise.resolve();
let reportVersion = 0;
function reportHolds(gpuIds: string[]) {
  const version = ++reportVersion;
  const next = reporting.then(async () => {
    if (version !== reportVersion) return;
    const result = await window.prism.games.setEditHolds(gpuIds);
    if (!result.success)
      throw new Error(result.error ?? "Could not report edit holds");
  });
  reporting = next.catch(() => undefined);
  return next;
}
