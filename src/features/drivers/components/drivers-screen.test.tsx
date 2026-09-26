import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import {
  createDriverJob,
  createDriverPlan,
  createDriverStatus,
} from "@/test/driver-fixtures";
import { DriversScreen } from "./drivers-screen";

describe("DriversScreen", () => {
  it("renders a loading skeleton", () => {
    render(
      <DriversScreen
        isLoading
        errorMessage={null}
        isStarting={false}
        isCancelling={false}
        isRebooting={false}
        actionError={null}
        onStartAction={vi.fn()}
        onCancelJob={vi.fn()}
        onRequestReboot={vi.fn()}
      />,
    );

    expect(screen.getByTestId("drivers-skeleton")).toBeInTheDocument();
  });

  it("starts the recommended install action", async () => {
    const user = userEvent.setup();
    const onStartAction = vi.fn();

    render(
      <DriversScreen
        status={createDriverStatus()}
        isLoading={false}
        errorMessage={null}
        isStarting={false}
        isCancelling={false}
        isRebooting={false}
        actionError={null}
        onStartAction={onStartAction}
        onCancelJob={vi.fn()}
        onRequestReboot={vi.fn()}
      />,
    );

    expect(screen.getByText("NVIDIA GeForce RTX 5090")).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Install recommended driver" }),
    );

    expect(onStartAction).toHaveBeenCalledWith("installRecommended");
  });

  it("renders unsupported distro blockers without enabling install", () => {
    render(
      <DriversScreen
        status={createDriverStatus({
          host: {
            distroId: "arch",
            distroVersion: "rolling",
            kernelRelease: "6.8.0",
            packageManager: "unsupported",
            secureBoot: "unknown",
            isFlatpak: false,
            supported: false,
            unsupportedReason:
              "Automated driver management is not supported on arch",
          },
          nvidia: {
            detected: true,
            driverVersion: null,
            nvmlAvailable: false,
            nouveauLoaded: false,
            runfileInstallDetected: false,
            rpmFusionDetected: false,
            gpus: [],
            warnings: [],
            plans: [
              createDriverPlan({
                blockers: [
                  "Automated driver installs are currently supported only on Ubuntu and Fedora.",
                ],
              }),
            ],
          },
        })}
        isLoading={false}
        errorMessage={null}
        isStarting={false}
        isCancelling={false}
        isRebooting={false}
        actionError={null}
        onStartAction={vi.fn()}
        onCancelJob={vi.fn()}
        onRequestReboot={vi.fn()}
      />,
    );

    expect(screen.getByText("Blocked")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Install recommended driver" }),
    ).toBeDisabled();
  });

  it("renders active job progress and reboot action", async () => {
    const user = userEvent.setup();
    const onRequestReboot = vi.fn();

    render(
      <DriversScreen
        status={createDriverStatus({
          activeJob: createDriverJob({
            status: "rebootRequired",
            currentStep: "Reboot required",
            logTail: ["Package operation finished."],
          }),
        })}
        isLoading={false}
        errorMessage={null}
        isStarting={false}
        isCancelling={false}
        isRebooting={false}
        actionError={null}
        onStartAction={vi.fn()}
        onCancelJob={vi.fn()}
        onRequestReboot={onRequestReboot}
      />,
    );

    expect(screen.getByText("Package operation finished.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Reboot now" }));

    expect(onRequestReboot).toHaveBeenCalledWith();
  });
});
