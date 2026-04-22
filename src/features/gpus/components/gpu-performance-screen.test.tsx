import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

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

const connectedDaemon = {
  reachable: true,
  managedByDaemon: true,
  socketPath: "/run/prism/prismd.sock",
  message: null,
};

describe("GpuPerformanceScreen", () => {
  it("renders a loading skeleton", () => {
    render(
      <GpuPerformanceScreen
        isLoading
        daemonStatus={connectedDaemon}
        errorMessage={null}
        mutationErrorMessage={null}
        reapplyPending={false}
        reapplyErrorMessage={null}
        pendingAction={null}
        onReapplySavedSettings={() => undefined}
        onMutationAction={() => undefined}
      />,
    );

    expect(screen.getByTestId("gpu-performance-skeleton")).toBeInTheDocument();
  });

  it("renders the control rail, telemetry panel, and unavailable voltage", () => {
    render(
      <GpuPerformanceScreen
        inventory={sampleInventory}
        isLoading={false}
        daemonStatus={connectedDaemon}
        errorMessage={null}
        mutationErrorMessage={null}
        reapplyPending={false}
        reapplyErrorMessage={null}
        pendingAction={null}
        onReapplySavedSettings={() => undefined}
        onMutationAction={() => undefined}
      />,
    );

    expect(screen.getByText("NVIDIA GeForce RTX 5090")).toBeInTheDocument();
    expect(screen.getAllByText("Power").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Cooling").length).toBeGreaterThan(0);
    expect(screen.getByText("Clock Offsets")).toBeInTheDocument();
    expect(screen.getByTestId("gpu-performance-controls")).toBeInTheDocument();
    expect(screen.getByTestId("gpu-performance-telemetry")).toBeInTheDocument();
    expect(screen.getByText("Live Telemetry")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reapply Saved Settings" })).toBeInTheDocument();
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
    expect(
      screen.queryByText("Privileged GPU controls are connected"),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText("The memory clock offset is not supported on this GPU."),
    ).toBeInTheDocument();
  });

  it("emits a power-limit mutation action", async () => {
    const user = userEvent.setup();
    const onMutationAction = vi.fn();

    render(
      <GpuPerformanceScreen
        inventory={sampleInventory}
        isLoading={false}
        daemonStatus={connectedDaemon}
        errorMessage={null}
        mutationErrorMessage={null}
        reapplyPending={false}
        reapplyErrorMessage={null}
        pendingAction={null}
        onReapplySavedSettings={() => undefined}
        onMutationAction={onMutationAction}
      />,
    );

    const powerInput = screen.getAllByRole("spinbutton")[0];
    await user.clear(powerInput);
    await user.type(powerInput, "600");
    await user.click(screen.getAllByRole("button", { name: "Apply" })[0]);

    expect(onMutationAction).toHaveBeenCalledWith({
      kind: "setPowerLimit",
      gpuId: "GPU-123",
      limitWatts: 600,
    });
  });

  it("wires the reapply action from the header when the daemon is connected", async () => {
    const user = userEvent.setup();
    const onReapplySavedSettings = vi.fn();

    render(
      <GpuPerformanceScreen
        inventory={sampleInventory}
        isLoading={false}
        daemonStatus={connectedDaemon}
        errorMessage={null}
        mutationErrorMessage={null}
        reapplyPending={false}
        reapplyErrorMessage={null}
        pendingAction={null}
        onReapplySavedSettings={onReapplySavedSettings}
        onMutationAction={() => undefined}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Reapply Saved Settings" }));
    expect(onReapplySavedSettings).toHaveBeenCalledTimes(1);
    expect(
      screen.queryByText("Privileged GPU controls are unavailable"),
    ).not.toBeInTheDocument();
  });

  it("shows a destructive daemon alert and disables controls when unavailable", () => {
    const onReapplySavedSettings = vi.fn();

    render(
      <GpuPerformanceScreen
        inventory={sampleInventory}
        isLoading={false}
        daemonStatus={{
          ...connectedDaemon,
          reachable: false,
        }}
        errorMessage={null}
        mutationErrorMessage={null}
        reapplyPending={false}
        reapplyErrorMessage={null}
        pendingAction={null}
        onReapplySavedSettings={onReapplySavedSettings}
        onMutationAction={() => undefined}
      />,
    );

    expect(screen.getByText("Privileged GPU controls are unavailable")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Controls stay visible so you can inspect what the driver exposes on this system, but changes stay locked until prismd is reachable.",
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Reapply Saved Settings" }),
    ).not.toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Apply" })[0]).toBeDisabled();
    expect(screen.getAllByRole("spinbutton")[0]).toBeDisabled();
  });
});
