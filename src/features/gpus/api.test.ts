import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  getGpuDaemonStatus,
  getGpuInventory,
  getGpuPerformance,
  resetGpuClockOffsets,
  resetGpuPowerLimit,
  setGpuClockOffsets,
  setGpuFanModeAuto,
  setGpuFanSpeed,
  setGpuPowerLimit,
} from "./api";

describe("GPU API bridge", () => {
  const gpuApi = window.prism.gpu;

  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("delegates read calls", async () => {
    const getInventory = vi
      .spyOn(gpuApi, "getInventory")
      .mockResolvedValue({ gpus: [], warnings: [] });
    const getPerformance = vi
      .spyOn(gpuApi, "getPerformance")
      .mockResolvedValue({ gpus: [], warnings: [] });
    const getDaemonStatus = vi
      .spyOn(gpuApi, "getDaemonStatus")
      .mockResolvedValue({
        reachable: false,
        managedByDaemon: false,
        socketPath: "/run/prism/prismd.sock",
        message: null,
        tuningStore: null,
      });

    await getGpuInventory();
    await getGpuPerformance();
    await getGpuDaemonStatus();

    expect(getInventory).toHaveBeenCalledWith();
    expect(getPerformance).toHaveBeenCalledWith();
    expect(getDaemonStatus).toHaveBeenCalledWith();
  });

  it("delegates direct performance mutations", async () => {
    const setPowerLimit = vi
      .spyOn(gpuApi, "setPowerLimit")
      .mockResolvedValue({ success: true, error: null });
    const resetPowerLimit = vi
      .spyOn(gpuApi, "resetPowerLimit")
      .mockResolvedValue({ success: true, error: null });
    const setFanSpeed = vi
      .spyOn(gpuApi, "setFanSpeed")
      .mockResolvedValue({ success: true, error: null });
    const setFanModeAuto = vi
      .spyOn(gpuApi, "setFanModeAuto")
      .mockResolvedValue({ success: true, error: null });
    const setClockOffsets = vi
      .spyOn(gpuApi, "setClockOffsets")
      .mockResolvedValue({
        graphics: { status: "succeeded", error: null },
        memory: null,
      });
    const resetClockOffsets = vi
      .spyOn(gpuApi, "resetClockOffsets")
      .mockResolvedValue({
        graphics: { status: "succeeded", error: null },
        memory: null,
      });

    await setGpuPowerLimit("GPU-123", 450);
    await resetGpuPowerLimit("GPU-123");
    await setGpuFanSpeed("GPU-123", 0, 60);
    await setGpuFanModeAuto("GPU-123", 0);
    await setGpuClockOffsets("GPU-123", 100, null);
    await resetGpuClockOffsets("GPU-123");

    expect(setPowerLimit).toHaveBeenCalledWith("GPU-123", 450);
    expect(resetPowerLimit).toHaveBeenCalledWith("GPU-123");
    expect(setFanSpeed).toHaveBeenCalledWith("GPU-123", 0, 60);
    expect(setFanModeAuto).toHaveBeenCalledWith("GPU-123", 0);
    expect(setClockOffsets).toHaveBeenCalledWith("GPU-123", 100, null);
    expect(resetClockOffsets).toHaveBeenCalledWith("GPU-123");
  });
});
