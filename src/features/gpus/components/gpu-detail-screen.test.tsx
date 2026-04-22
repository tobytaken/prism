import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";

import { GpuDetailScreen } from "./gpu-detail-screen";

const sampleGpu = {
  id: "GPU-123",
  vendor: "nvidia" as const,
  name: "NVIDIA GeForce RTX 5090",
  architecture: "Blackwell",
  driverVersion: "595.58.03",
  firmwareVersion: "98.02.2E.00.03",
  busId: "0000:01:00.0",
  memory: {
    totalMiB: 32_768,
  },
  compute: {
    api: "cuda" as const,
    apiVersion: "13.2",
    capability: "12.0",
    coreCount: 21_760,
  },
  pcie: {
    genMax: 5,
    widthMax: 16,
  },
  power: {
    enforcedLimitWatts: 475,
    defaultLimitWatts: 575,
  },
  clocks: {
    graphicsCurrentMHz: 1_365,
    graphicsMaxMHz: 3_300,
    smMaxMHz: 3_300,
    memoryMaxMHz: 14_001,
  },
  control: {
    managedByDaemon: true,
    daemonReachable: true,
    supportedFeatures: {
      powerLimit: true,
      gpuClockLock: true,
      memoryClockLock: true,
      graphicsOffset: true,
      memoryOffset: true,
      reset: true,
    },
    powerLimit: {
      currentWatts: 475,
      defaultWatts: 575,
      minWatts: 300,
      maxWatts: 575,
      desiredWatts: 450,
    },
    gpuClockLock: {
      currentMinMHz: 1_800,
      currentMaxMHz: 2_400,
      minAllowedMHz: 1_500,
      maxAllowedMHz: 3_000,
      supported: true,
    },
    memoryClockLock: {
      currentMinMHz: 1_000,
      currentMaxMHz: 1_100,
      minAllowedMHz: 800,
      maxAllowedMHz: 1_200,
      supported: true,
    },
    offsets: {
      graphicsMHz: 100,
      memoryMHz: 500,
      minGraphicsMHz: -200,
      maxGraphicsMHz: 200,
      minMemoryMHz: -1_000,
      maxMemoryMHz: 1_000,
    },
    warnings: [],
  },
};

function renderScreen(ui: React.ReactNode) {
  return render(<MemoryRouter>{ui}</MemoryRouter>);
}

describe("GpuDetailScreen", () => {
  it("renders a loading skeleton", () => {
    renderScreen(<GpuDetailScreen isLoading errorMessage={null} />);

    expect(screen.getByTestId("gpu-detail-skeleton")).toBeInTheDocument();
  });

  it("renders the selected GPU details and specs", () => {
    renderScreen(
      <GpuDetailScreen gpu={sampleGpu} isLoading={false} errorMessage={null} />,
    );

    expect(screen.getByRole("heading", { name: "NVIDIA GeForce RTX 5090" })).toBeInTheDocument();
    expect(
      screen.getByText("NVIDIA", { selector: '[data-vendor="nvidia"]' }),
    ).toHaveAttribute("data-vendor", "nvidia");
    expect(screen.getAllByText("Blackwell").length).toBeGreaterThan(0);
    expect(screen.getAllByText("0000:01:00.0").length).toBeGreaterThan(0);
    expect(screen.getByText("VRAM")).toBeInTheDocument();
    expect(screen.getAllByText("32,768 MiB").length).toBeGreaterThan(0);
    expect(screen.getByText("Core Count")).toBeInTheDocument();
    expect(screen.getAllByText("21,760").length).toBeGreaterThan(0);
    expect(screen.getByText("Current Graphics Clock")).toBeInTheDocument();
    expect(screen.getAllByText("1,365 MHz").length).toBeGreaterThan(0);
    expect(screen.getByText("Gen 5 x16")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /back to overview/i })).toBeInTheDocument();
  });

  it("renders a missing gpu state", () => {
    renderScreen(<GpuDetailScreen isLoading={false} errorMessage={null} />);

    expect(screen.getByText("GPU not found")).toBeInTheDocument();
  });
});
