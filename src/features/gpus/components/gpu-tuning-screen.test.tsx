import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import {
  connectedDaemonStatus,
  createPerformanceResponse,
} from "@/test/gpu-fixtures";
import { GpuTuningScreen } from "./gpu-tuning-screen";

describe("GpuTuningScreen", () => {
  it("renders a loading skeleton", () => {
    render(
      <GpuTuningScreen
        isLoading
        daemonStatus={connectedDaemonStatus}
        errorMessage={null}
        mutationErrorMessage={null}
        reapplyPending={false}
        reapplyErrorMessage={null}
        pendingAction={null}
        onReapplySavedSettings={() => undefined}
        onMutationAction={() => undefined}
      />,
    );

    expect(screen.getByTestId("gpu-tuning-skeleton")).toBeInTheDocument();
  });

  it("renders the control rail without the telemetry panel", () => {
    render(
      <GpuTuningScreen
        inventory={createPerformanceResponse()}
        isLoading={false}
        daemonStatus={connectedDaemonStatus}
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
    expect(screen.getByTestId("gpu-tuning-controls")).toBeInTheDocument();
    expect(
      screen.queryByTestId("gpu-performance-telemetry"),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("Live Telemetry")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reapply Saved Settings" })).toBeInTheDocument();
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
      <GpuTuningScreen
        inventory={createPerformanceResponse()}
        isLoading={false}
        daemonStatus={connectedDaemonStatus}
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
      <GpuTuningScreen
        inventory={createPerformanceResponse()}
        isLoading={false}
        daemonStatus={connectedDaemonStatus}
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
      <GpuTuningScreen
        inventory={createPerformanceResponse()}
        isLoading={false}
        daemonStatus={{
          ...connectedDaemonStatus,
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
