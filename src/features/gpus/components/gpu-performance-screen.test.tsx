import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { GpuPerformanceScreen } from "./gpu-performance-screen";

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

describe("GpuPerformanceScreen", () => {
  it("renders a loading skeleton", () => {
    render(
      <GpuPerformanceScreen
        isLoading
        errorMessage={null}
      />,
    );

    expect(screen.getByTestId("gpu-performance-skeleton")).toBeInTheDocument();
  });

  it("renders telemetry without the tuning controls", () => {
    render(
      <GpuPerformanceScreen
        inventory={sampleInventory}
        isLoading={false}
        errorMessage={null}
      />,
    );

    expect(screen.getByText("NVIDIA GeForce RTX 5090")).toBeInTheDocument();
    expect(screen.getByTestId("gpu-performance-telemetry")).toBeInTheDocument();
    expect(screen.getByText("Live Telemetry")).toBeInTheDocument();
    expect(screen.queryByTestId("gpu-tuning-controls")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Reapply Saved Settings" }),
    ).not.toBeInTheDocument();
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
    expect(screen.queryByText("Clock Offsets")).not.toBeInTheDocument();
  });
});
