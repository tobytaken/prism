import { createElement } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  appendGpuSamples,
  GPU_METRIC_HISTORY_LIMIT,
  GpuMetricHistoryProvider,
  type GpuMetricHistory,
  useGpuMetricHistory,
} from "./use-gpu-metric-history";
import { useGpuPerformance } from "./use-gpu-performance";

import type { GpuPerformanceDevice } from "@/features/gpus/types";

vi.mock("./use-gpu-performance", () => ({
  useGpuPerformance: vi.fn(),
}));

const mockedUseGpuPerformance = vi.mocked(useGpuPerformance);

beforeEach(() => {
  mockedUseGpuPerformance.mockReset();
});

describe("appendGpuSamples", () => {
  it("trims metric history to the last 120 samples", () => {
    let history: Record<string, GpuMetricHistory> = {};

    for (let sample = 1; sample <= GPU_METRIC_HISTORY_LIMIT + 10; sample += 1) {
      history = appendGpuSamples(history, [createGpuDevice("GPU-1", sample)]);
    }

    expect(history["GPU-1"].powerDrawWatts.samples).toHaveLength(
      GPU_METRIC_HISTORY_LIMIT,
    );
    expect(history["GPU-1"].powerDrawWatts.samples[0]).toBe(11);
    expect(
      history["GPU-1"].powerDrawWatts.samples[
        history["GPU-1"].powerDrawWatts.samples.length - 1
      ],
    ).toBe(130);
  });

  it("keeps history isolated per GPU id", () => {
    let history = appendGpuSamples({}, [
      createGpuDevice("GPU-1", 120),
      createGpuDevice("GPU-2", 220),
    ]);

    history = appendGpuSamples(history, [
      createGpuDevice("GPU-1", 125),
      createGpuDevice("GPU-2", 225),
    ]);

    expect(history["GPU-1"].powerDrawWatts.samples).toEqual([120, 125]);
    expect(history["GPU-2"].powerDrawWatts.samples).toEqual([220, 225]);
  });

  it("marks null-only history as unavailable", () => {
    const history = appendGpuSamples({}, [createGpuDevice("GPU-1", null)]);

    expect(history["GPU-1"].powerDrawWatts.samples).toEqual([null]);
    expect(history["GPU-1"].powerDrawWatts.hasData).toBe(false);
  });
});

describe("GpuMetricHistoryProvider", () => {
  it("collects telemetry in the background from shared performance updates", async () => {
    const queryResult = createPerformanceQueryResult();
    mockedUseGpuPerformance.mockImplementation(() => queryResult);

    const { rerender } = render(renderHistoryProbeTree());

    queryResult.data = { gpus: [createGpuDevice("GPU-1", 120)], warnings: [] };
    queryResult.dataUpdatedAt = 1;

    rerender(renderHistoryProbeTree());

    await waitFor(() => {
      expect(screen.getByTestId("history-samples")).toHaveTextContent("120");
    });

    queryResult.data = { gpus: [createGpuDevice("GPU-1", 125)], warnings: [] };
    queryResult.dataUpdatedAt = 2;

    rerender(renderHistoryProbeTree());

    await waitFor(() => {
      expect(screen.getByTestId("history-samples")).toHaveTextContent("120,125");
    });
  });

  it("preserves collected history across consumer remounts", async () => {
    const queryResult = createPerformanceQueryResult({
      gpus: [createGpuDevice("GPU-1", 312)],
      warnings: [],
    });
    queryResult.dataUpdatedAt = 1;
    mockedUseGpuPerformance.mockImplementation(() => queryResult);

    const { rerender } = render(createElement(HistoryProbeHarness, { showProbe: true }));

    await waitFor(() => {
      expect(screen.getByTestId("history-samples")).toHaveTextContent("312");
    });

    rerender(createElement(HistoryProbeHarness, { showProbe: false }));
    rerender(createElement(HistoryProbeHarness, { showProbe: true }));

    expect(screen.getByTestId("history-samples")).toHaveTextContent("312");
  });
});

function createGpuDevice(
  id: string,
  powerDrawWatts: number | null,
): GpuPerformanceDevice {
  return {
    id,
    vendor: "nvidia",
    name: `GPU ${id}`,
    architecture: "Blackwell",
    busId: `0000:${id}:00.0`,
    power: {
      isSupported: true,
      unsupportedReason: null,
      currentDrawWatts: powerDrawWatts,
      currentLimitWatts: 450,
      defaultLimitWatts: 450,
      enforcedLimitWatts: 450,
      minLimitWatts: 300,
      maxLimitWatts: 600,
    },
    cooling: {
      isSupported: true,
      unsupportedReason: null,
      fanCount: 1,
      fans: [
        {
          index: 0,
          currentSpeedPercent: 42,
          minSpeedPercent: 30,
          maxSpeedPercent: 100,
          mode: "auto",
        },
      ],
    },
    clocks: {
      isSupported: true,
      unsupportedReason: null,
      graphics: {
        isSupported: true,
        unsupportedReason: null,
        currentOffsetMHz: 50,
        minOffsetMHz: -200,
        maxOffsetMHz: 200,
      },
      memory: {
        isSupported: true,
        unsupportedReason: null,
        currentOffsetMHz: 100,
        minOffsetMHz: -500,
        maxOffsetMHz: 500,
      },
    },
    telemetry: {
      powerDrawWatts,
      coreClockMHz: 2550,
      memoryClockMHz: 14001,
      fanSpeedPercent: 42,
      voltageMillivolts: null,
      temperatureC: 71,
      gpuUtilizationPercent: 95,
      memoryUsedMiB: 12288,
      memoryTotalMiB: 32768,
      performanceState: "P0",
    },
  };
}

function createPerformanceQueryResult(
  data?: { gpus: GpuPerformanceDevice[]; warnings: string[] },
) {
  return {
    data,
    dataUpdatedAt: 0,
  } as ReturnType<typeof useGpuPerformance>;
}

function HistoryProbe() {
  const historyByGpuId = useGpuMetricHistory();
  const samples = historyByGpuId["GPU-1"]?.powerDrawWatts.samples ?? [];

  return createElement("div", { "data-testid": "history-samples" }, samples.join(","));
}

function HistoryProbeHarness({ showProbe }: { showProbe: boolean }) {
  return createElement(
    GpuMetricHistoryProvider,
    null,
    showProbe ? createElement(HistoryProbe) : null,
  );
}

function renderHistoryProbeTree() {
  return createElement(
    GpuMetricHistoryProvider,
    null,
    createElement(HistoryProbe),
  );
}
