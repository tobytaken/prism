import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  useGpuMetricHistory,
  type GpuMetricHistory,
} from "@/features/gpus/hooks/use-gpu-metric-history";
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

const sampleInventory = {
  gpus: [
    {
      id: "GPU-123",
      vendor: "nvidia" as const,
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
            mode: "auto" as const,
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
    },
  ],
  warnings: [],
};

const connectedDaemon = {
  reachable: true,
  managedByDaemon: true,
  socketPath: "/run/prism/prismd.sock",
  message: null,
};

describe("GpuPerformanceScreen", () => {
  beforeEach(() => {
    mockedUseGpuMetricHistory.mockReturnValue({});
  });

  it("renders a loading skeleton", () => {
    render(
      <GpuPerformanceScreen
        isLoading
        errorMessage={null}
      />,
    );

    expect(screen.getByTestId("gpu-performance-skeleton")).toBeInTheDocument();
  });

  it("renders telemetry with the tuning controls", () => {
    render(
      <GpuPerformanceScreen
        inventory={sampleInventory}
        daemonStatus={connectedDaemon}
        isLoading={false}
        errorMessage={null}
        reapplyPending={false}
        pendingAction={null}
        onReapplySavedSettings={() => undefined}
        onMutationAction={() => undefined}
      />,
    );

    expect(screen.getByText("NVIDIA GeForce RTX 5090")).toBeInTheDocument();
    expect(screen.getByTestId("gpu-performance-telemetry")).toBeInTheDocument();
    expect(screen.getByTestId("gpu-tuning-controls")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Reapply Saved Settings" }),
    ).toBeInTheDocument();
    expect(screen.getAllByText("312 W").length).toBeGreaterThan(0);
    expect(screen.getByText("2,550 MHz")).toBeInTheDocument();
    expect(screen.getByText("14,001 MHz")).toBeInTheDocument();
    expect(screen.getAllByText("42%").length).toBeGreaterThan(0);
    expect(screen.getByText("71 °C")).toBeInTheDocument();
    expect(screen.getByText("96%")).toBeInTheDocument();
    expect(screen.getByText("12,288 MiB")).toBeInTheDocument();
    expect(screen.getAllByText("P0").length).toBeGreaterThan(0);
    expect(screen.getByText("Voltage")).toBeInTheDocument();
    expect(screen.getAllByText("Unavailable").length).toBeGreaterThan(0);
    expect(screen.getByText("Clock Offsets")).toBeInTheDocument();
  });

  it("renders chart min and max labels from the recorded history", () => {
    mockedUseGpuMetricHistory.mockReturnValue({
      "GPU-123": buildMetricHistory({
        powerDrawWatts: [300, 318, 325],
      }),
    });

    render(
      <GpuPerformanceScreen
        inventory={sampleInventory}
        isLoading={false}
        errorMessage={null}
      />,
    );

    expect(screen.getByText("325 W")).toBeInTheDocument();
    expect(screen.getByText("300 W")).toBeInTheDocument();
  });
});

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
