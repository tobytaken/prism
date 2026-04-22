import { useEffect, useState } from "react";

import type { GpuPerformanceDevice } from "@/features/gpus/types";

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

export const GPU_METRIC_HISTORY_LIMIT = 120;

export function useGpuMetricHistory(gpus: GpuPerformanceDevice[]) {
  const [historyByGpuId, setHistoryByGpuId] = useState<Record<string, GpuMetricHistory>>({});

  useEffect(() => {
    setHistoryByGpuId((current) => appendGpuSamples(current, gpus));
  }, [gpus]);

  return historyByGpuId;
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
