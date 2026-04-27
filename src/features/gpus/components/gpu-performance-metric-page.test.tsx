import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";

import type { GpuMetricHistory } from "@/features/gpus/hooks/use-gpu-metric-history";
import type { GpuPerformanceDevice } from "@/features/gpus/types";
import { GpuPerformanceMetricScreen } from "./gpu-performance-metric-page";

const sampleGpu: GpuPerformanceDevice = {
  id: "GPU-123",
  vendor: "nvidia",
  name: "NVIDIA GeForce RTX 5090",
  architecture: "Blackwell",
  busId: "0000:01:00.0",
  power: {
    isSupported: true,
    unsupportedReason: null,
    currentDrawWatts: 312,
    currentLimitWatts: 475,
    defaultLimitWatts: 575,
    enforcedLimitWatts: 475,
    minLimitWatts: 400,
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
      currentOffsetMHz: 120,
      minOffsetMHz: -200,
      maxOffsetMHz: 200,
    },
    memory: {
      isSupported: false,
      unsupportedReason: "The memory clock offset is not supported on this GPU.",
      currentOffsetMHz: null,
      minOffsetMHz: null,
      maxOffsetMHz: null,
    },
  },
  telemetry: {
    powerDrawWatts: 312,
    coreClockMHz: 2550,
    memoryClockMHz: 14001,
    fanSpeedPercent: 42,
    voltageMillivolts: null,
    temperatureC: 71,
    gpuUtilizationPercent: 96,
    memoryUsedMiB: 12288,
    memoryTotalMiB: 32768,
    performanceState: "P0",
  },
};

describe("GpuPerformanceMetricScreen", () => {
  it("renders a loading skeleton", () => {
    renderScreen(
      <GpuPerformanceMetricScreen
        isLoading
        errorMessage={null}
      />,
    );

    expect(screen.getByTestId("gpu-performance-metric-skeleton")).toBeInTheDocument();
  });

  it("renders the selected metric details and history stats", () => {
    renderScreen(
      <GpuPerformanceMetricScreen
        gpu={sampleGpu}
        metricKey="powerDrawWatts"
        history={buildMetricHistory({ powerDrawWatts: [300, 318, 324] })}
        isLoading={false}
        errorMessage={null}
      />,
    );

    expect(screen.getByRole("heading", { name: "Power" })).toBeInTheDocument();
    expect(screen.getByText("NVIDIA GeForce RTX 5090")).toBeInTheDocument();
    expect(screen.getByText("Current")).toBeInTheDocument();
    expect(screen.getByText("312 W")).toBeInTheDocument();
    expect(screen.getByText("Minimum")).toBeInTheDocument();
    expect(screen.getAllByText("300 W").length).toBeGreaterThan(0);
    expect(screen.getByText("Maximum")).toBeInTheDocument();
    expect(screen.getAllByText("324 W").length).toBeGreaterThan(0);
    expect(screen.getByText("Average")).toBeInTheDocument();
    expect(screen.getByText("314 W")).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "Power history chart" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /back to performance/i })).toBeInTheDocument();
  });

  it("renders a missing gpu state", () => {
    renderScreen(
      <GpuPerformanceMetricScreen
        metricKey="powerDrawWatts"
        isLoading={false}
        errorMessage={null}
      />,
    );

    expect(screen.getByText("GPU not found")).toBeInTheDocument();
  });

  it("renders an invalid metric state", () => {
    renderScreen(
      <GpuPerformanceMetricScreen
        gpu={sampleGpu}
        metricKey="voltageMillivolts"
        isLoading={false}
        errorMessage={null}
      />,
    );

    expect(screen.getByText("Metric not found")).toBeInTheDocument();
  });

  it("shows current value when history is unavailable", () => {
    renderScreen(
      <GpuPerformanceMetricScreen
        gpu={sampleGpu}
        metricKey="temperatureC"
        history={buildMetricHistory({})}
        isLoading={false}
        errorMessage={null}
      />,
    );

    expect(screen.getByText("71 °C")).toBeInTheDocument();
    expect(screen.getByTestId("gpu-performance-metric-chart-empty")).toHaveTextContent(
      "Unavailable",
    );
  });
});

function renderScreen(ui: React.ReactNode) {
  return render(<MemoryRouter>{ui}</MemoryRouter>);
}

function buildMetricHistory(
  overrides: Partial<Record<keyof GpuMetricHistory, Array<number | null>>>,
): GpuMetricHistory {
  return {
    powerDrawWatts: buildSeries(overrides.powerDrawWatts),
    coreClockMHz: buildSeries(overrides.coreClockMHz),
    memoryClockMHz: buildSeries(overrides.memoryClockMHz),
    fanSpeedPercent: buildSeries(overrides.fanSpeedPercent),
    temperatureC: buildSeries(overrides.temperatureC),
    gpuUtilizationPercent: buildSeries(overrides.gpuUtilizationPercent),
    memoryUsedMiB: buildSeries(overrides.memoryUsedMiB),
  };
}

function buildSeries(samples: Array<number | null> | undefined) {
  const nextSamples = samples ?? [];

  return {
    samples: nextSamples,
    hasData: nextSamples.some((value) => value != null),
  };
}
