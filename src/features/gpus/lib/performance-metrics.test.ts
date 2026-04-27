import { describe, expect, it } from "vitest";

import { createPerformanceGpu } from "@/test/gpu-fixtures";
import {
  PERFORMANCE_METRICS,
  formatPercent,
  getPerformanceMetricDefinition,
} from "./performance-metrics";

describe("performance metrics", () => {
  it("finds metric definitions by key", () => {
    expect(getPerformanceMetricDefinition("powerDrawWatts")?.label).toBe("Power");
    expect(getPerformanceMetricDefinition("voltageMillivolts")).toBeUndefined();
  });

  it("formats nullable percentages", () => {
    expect(formatPercent(null)).toBe("Unavailable");
    expect(formatPercent(42)).toBe("42%");
  });

  it("maps every metric definition to a GPU telemetry value", () => {
    const gpu = createPerformanceGpu();
    const values = Object.fromEntries(
      PERFORMANCE_METRICS.map((metric) => [metric.key, metric.getValue(gpu)]),
    );

    expect(values).toEqual({
      powerDrawWatts: 312,
      coreClockMHz: 2_550,
      memoryClockMHz: 14_001,
      fanSpeedPercent: 42,
      temperatureC: 71,
      gpuUtilizationPercent: 96,
      memoryUsedMiB: 12_288,
    });
  });
});
