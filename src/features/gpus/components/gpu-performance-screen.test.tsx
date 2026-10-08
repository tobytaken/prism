import { screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  useGpuMetricHistory,
  type GpuMetricHistory,
} from "@/features/gpus/hooks/use-gpu-metric-history";
import {
  connectedDaemonStatus,
  createPerformanceResponse,
} from "@/test/gpu-fixtures";
import { renderWithQueryClient } from "@/test/query-client";
import { GpuPerformanceScreen } from "./gpu-performance-screen";

vi.mock("@/features/gpus/hooks/use-gpu-metric-history", async () => {
  const actual = await vi.importActual<
    typeof import("@/features/gpus/hooks/use-gpu-metric-history")
  >("@/features/gpus/hooks/use-gpu-metric-history");

  return {
    ...actual,
    useGpuMetricHistory: vi.fn(),
  };
});

const mockedUseGpuMetricHistory = vi.mocked(useGpuMetricHistory);

describe("GpuPerformanceScreen", () => {
  beforeEach(() => {
    mockedUseGpuMetricHistory.mockReturnValue({});
  });

  it("renders a loading skeleton", () => {
    renderScreen(
      <GpuPerformanceScreen
        isLoading
        errorMessage={null}
      />,
    );

    expect(screen.getByTestId("gpu-performance-skeleton")).toBeInTheDocument();
  });

  it("renders telemetry with the tuning controls", () => {
    renderScreen(
      <GpuPerformanceScreen
        inventory={createPerformanceResponse()}
        daemonStatus={connectedDaemonStatus}
        isLoading={false}
        errorMessage={null}
        pendingAction={null}
        onMutationAction={() => undefined}
      />,
    );

    expect(screen.getByText("NVIDIA GeForce RTX 5090")).toBeInTheDocument();
    expect(screen.getByTestId("gpu-performance-telemetry")).toBeInTheDocument();
    expect(screen.getByTestId("gpu-tuning-controls")).toBeInTheDocument();
    expect(screen.queryByText("Signal Monitor")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Reapply Saved Settings" }),
    ).not.toBeInTheDocument();
    expect(screen.getAllByText("312 W").length).toBeGreaterThan(0);
    expect(screen.getByText("Voltage")).toBeInTheDocument();
    expect(screen.getAllByText("Unavailable").length).toBeGreaterThan(0);
    expect(screen.getByText("Clock Offsets")).toBeInTheDocument();
  });

  it("links sampled metrics to their detail screens only", () => {
    renderScreen(
      <GpuPerformanceScreen
        inventory={createPerformanceResponse()}
        daemonStatus={connectedDaemonStatus}
        isLoading={false}
        errorMessage={null}
        pendingAction={null}
        onMutationAction={() => undefined}
      />,
    );

    expect(
      screen.getByRole("link", {
        name: "Open Power details for NVIDIA GeForce RTX 5090",
      }),
    ).toHaveAttribute("href", "/performance/GPU-123/powerDrawWatts");
    expect(
      screen.getByRole("link", {
        name: "Open Core Clock details for NVIDIA GeForce RTX 5090",
      }),
    ).toHaveAttribute("href", "/performance/GPU-123/coreClockMHz");
    expect(screen.getByText("Voltage").closest("a")).toBeNull();
    expect(screen.getByText("Performance State").closest("a")).toBeNull();
  });

  it("renders chart min and max labels from the recorded history", () => {
    mockedUseGpuMetricHistory.mockReturnValue({
      "GPU-123": buildMetricHistory({
        powerDrawWatts: [300, 318, 325],
      }),
    });

    renderScreen(
      <GpuPerformanceScreen
        inventory={createPerformanceResponse()}
        isLoading={false}
        errorMessage={null}
      />,
    );

    expect(screen.getByText("325 W")).toBeInTheDocument();
    expect(screen.getByText("300 W")).toBeInTheDocument();
  });

  it("renders overview sparklines from the last 120 samples", () => {
    mockedUseGpuMetricHistory.mockReturnValue({
      "GPU-123": buildMetricHistory({
        powerDrawWatts: [999, ...Array.from({ length: 120 }, (_, index) => index + 1)],
      }),
    });

    renderScreen(
      <GpuPerformanceScreen
        inventory={createPerformanceResponse()}
        isLoading={false}
        errorMessage={null}
      />,
    );

    expect(screen.queryByText("999 W")).not.toBeInTheDocument();
  });
});

// The cooling controls ask the daemon to check fan curve previews.
function renderScreen(ui: React.ReactNode) {
  return renderWithQueryClient(<MemoryRouter>{ui}</MemoryRouter>);
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
