import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";

import type { GpuMetricHistory } from "@/features/gpus/hooks/use-gpu-metric-history";
import { createPerformanceGpu } from "@/test/gpu-fixtures";
import { GpuPerformanceMetricScreen } from "./gpu-performance-metric-page";

const sampleGpu = createPerformanceGpu();

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

  it("rounds the average stat to a whole number", () => {
    renderScreen(
      <GpuPerformanceMetricScreen
        gpu={sampleGpu}
        metricKey="powerDrawWatts"
        history={buildMetricHistory({ powerDrawWatts: [300, 301, 302, 304] })}
        isLoading={false}
        errorMessage={null}
      />,
    );

    expect(screen.getByText("Average")).toBeInTheDocument();
    expect(screen.getByText("302 W")).toBeInTheDocument();
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

  it("renders x-axis labels in ten second increments", () => {
    renderScreen(
      <GpuPerformanceMetricScreen
        gpu={sampleGpu}
        metricKey="powerDrawWatts"
        history={buildMetricHistory({
          powerDrawWatts: Array.from({ length: 41 }, (_, index) => 300 + index),
        })}
        isLoading={false}
        errorMessage={null}
      />,
    );

    expect(screen.getByText("0s")).toBeInTheDocument();
    expect(screen.getByText("10s")).toBeInTheDocument();
    expect(screen.getByText("20s")).toBeInTheDocument();
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
