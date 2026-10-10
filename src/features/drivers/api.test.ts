import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  cancelDriverJob,
  getDriverJob,
  getDriverStatus,
  requestHostReboot,
  startDriverAction,
} from "./api";

describe("driver API bridge", () => {
  const driverApi = window.prism.drivers;

  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("delegates driver calls", async () => {
    const getStatus = vi.spyOn(driverApi, "getStatus").mockResolvedValue({
      host: {
        distroId: "ubuntu",
        distroVersion: "24.04",
        kernelRelease: "6.8.0",
        packageManager: "apt",
        secureBoot: "disabled",
        isFlatpak: false,
        supported: true,
        unsupportedReason: null,
      },
      nvidia: {
        detected: false,
        driverVersion: null,
        nvmlAvailable: false,
        nouveauLoaded: false,
        runfileInstallDetected: false,
        rpmFusionDetected: false,
        gpus: [],
        plans: [],
        warnings: [],
      },
      activeJob: null,
      warnings: [],
    });
    const startAction = vi.spyOn(driverApi, "startAction").mockResolvedValue({
      id: "driver-1",
      status: "queued",
      action: "installRecommended",
      startedAtUnixMs: 0,
      finishedAtUnixMs: null,
      currentStep: "Queued",
      logTail: [],
      error: null,
      requiresReboot: true,
    });
    const getJob = vi.spyOn(driverApi, "getJob").mockResolvedValue({
      id: "driver-1",
      status: "running",
      action: "installRecommended",
      startedAtUnixMs: 0,
      finishedAtUnixMs: null,
      currentStep: "Installing",
      logTail: [],
      error: null,
      requiresReboot: true,
    });
    const cancelJob = vi.spyOn(driverApi, "cancelJob").mockResolvedValue({
      id: "driver-1",
      status: "canceled",
      action: "installRecommended",
      startedAtUnixMs: 0,
      finishedAtUnixMs: 1,
      currentStep: null,
      logTail: [],
      error: null,
      requiresReboot: true,
    });
    const requestReboot = vi
      .spyOn(driverApi, "requestReboot")
      .mockResolvedValue(undefined);

    await getDriverStatus();
    await startDriverAction("installRecommended");
    await getDriverJob("driver-1");
    await cancelDriverJob("driver-1");
    await requestHostReboot();

    expect(getStatus).toHaveBeenCalledWith();
    expect(startAction).toHaveBeenCalledWith("installRecommended");
    expect(getJob).toHaveBeenCalledWith("driver-1");
    expect(cancelJob).toHaveBeenCalledWith("driver-1");
    expect(requestReboot).toHaveBeenCalledWith();
  });
});
