import {
  createElement,
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  type PropsWithChildren,
} from "react";

import type { GpuPerformanceDevice } from "@/features/gpus/types";
import { useGpuPerformance } from "./use-gpu-performance";

export const GPU_HISTORY_KEYS = [
  "powerDrawWatts",
  "coreClockMHz",
  "memoryClockMHz",
  "fanSpeedPercent",
  "temperatureC",
  "gpuUtilizationPercent",
  "memoryUsedMiB",
] as const;

export type GpuHistoryKey = (typeof GPU_HISTORY_KEYS)[number];

export interface MetricHistorySeries {
  samples: Array<number | null>;
  hasData: boolean;
}

export type GpuMetricHistory = Record<GpuHistoryKey, MetricHistorySeries>;

export const GPU_METRIC_HISTORY_LIMIT = 240;

const GpuMetricHistoryContext = createContext<Record<string, GpuMetricHistory>>(
  {},
);

export function GpuMetricHistoryProvider({ children }: PropsWithChildren) {
  const query = useGpuPerformance();
  const [historyByGpuId, setHistoryByGpuId] = useState<Record<string, GpuMetricHistory>>({});
  const lastProcessedUpdateAtRef = useRef(0);

  useEffect(() => {
    if (!query.data || query.dataUpdatedAt === lastProcessedUpdateAtRef.current) {
      return;
    }

    lastProcessedUpdateAtRef.current = query.dataUpdatedAt;
    setHistoryByGpuId((current) => appendGpuSamples(current, query.data.gpus));
  }, [query.data, query.dataUpdatedAt]);

  return createElement(
    GpuMetricHistoryContext.Provider,
    { value: historyByGpuId },
    children,
  );
}

export function useGpuMetricHistory() {
  return useContext(GpuMetricHistoryContext);
}

export function appendGpuSamples(
  current: Record<string, GpuMetricHistory>,
  gpus: GpuPerformanceDevice[],
) {
  const next: Record<string, GpuMetricHistory> = {};

  for (const gpu of gpus) {
    const existing = current[gpu.id];
    const telemetry = gpu.telemetry;

    next[gpu.id] = {
      powerDrawWatts: appendMetricSample(existing?.powerDrawWatts, telemetry.powerDrawWatts),
      coreClockMHz: appendMetricSample(existing?.coreClockMHz, telemetry.coreClockMHz),
      memoryClockMHz: appendMetricSample(existing?.memoryClockMHz, telemetry.memoryClockMHz),
      fanSpeedPercent: appendMetricSample(
        existing?.fanSpeedPercent,
        telemetry.fanSpeedPercent,
      ),
      temperatureC: appendMetricSample(existing?.temperatureC, telemetry.temperatureC),
      gpuUtilizationPercent: appendMetricSample(
        existing?.gpuUtilizationPercent,
        telemetry.gpuUtilizationPercent,
      ),
      memoryUsedMiB: appendMetricSample(existing?.memoryUsedMiB, telemetry.memoryUsedMiB),
    };
  }

  return next;
}

function appendMetricSample(
  series: MetricHistorySeries | undefined,
  sample: number | null,
): MetricHistorySeries {
  const nextSamples = [...(series?.samples ?? []), sample].slice(-GPU_METRIC_HISTORY_LIMIT);

  return {
    samples: nextSamples,
    hasData: nextSamples.some((value) => value != null),
  };
}
