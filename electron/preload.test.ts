import { beforeEach, describe, expect, it, vi } from "vitest";

import tuningOutcomes from "@/test/contracts/tuning-outcomes.json";
import profileOutcomes from "@/test/contracts/profile-outcomes.json";
import tuningObservations from "@/test/contracts/tuning-observations.json";

import type { PrismDesktopApi } from "@/types/global";

const mocks = vi.hoisted(() => ({
  exposeInMainWorld: vi.fn(),
  invoke: vi.fn(),
  on: vi.fn(),
  removeListener: vi.fn(),
}));

vi.mock("electron", () => ({
  contextBridge: {
    exposeInMainWorld: mocks.exposeInMainWorld,
  },
  ipcRenderer: {
    invoke: mocks.invoke,
    on: mocks.on,
    removeListener: mocks.removeListener,
  },
}));

describe("preload bridge", () => {
  beforeEach(() => {
    vi.resetModules();
    mocks.exposeInMainWorld.mockReset();
    mocks.invoke.mockReset();
    mocks.on.mockReset();
    mocks.removeListener.mockReset();
  });

  it("transports stable ownership and included profile values without hiding partial outcomes or transport failures", async () => {
    const prism = await loadPrismApi();
    const request = {
      gpuId: "GPU-123",
      settings: {
        powerLimitWatts: { kind: "default" as const },
        graphicsOffsetMHz: { kind: "value" as const, value: -150 },
      },
    };
    mocks.invoke.mockResolvedValue(profileOutcomes);
    expect(await prism.gpu.applyProfile(request)).toEqual(profileOutcomes);
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "prism:gpu:applyProfile",
      request,
    );
    mocks.invoke.mockRejectedValue(new Error("Connection lost"));
    await expect(prism.gpu.applyProfile(request)).rejects.toThrow(
      "Connection lost",
    );
  });

  it("exposes GPU IPC methods with exact channels and payloads", async () => {
    const prism = await loadPrismApi();

    await prism.gpu.getInventory();
    await prism.gpu.getPerformance();
    await prism.gpu.getDaemonStatus();
    await prism.gpu.applyConfig({ gpuId: "GPU-123", powerLimitWatts: 450 });
    await prism.gpu.resetConfig({ gpuId: "GPU-123" });
    await prism.gpu.setPowerLimit("GPU-123", 450);
    await prism.gpu.resetPowerLimit("GPU-123");
    await prism.gpu.setFanSpeed("GPU-123", 0, 60);
    await prism.gpu.setFanModeAuto("GPU-123", 0);
    await prism.gpu.setClockOffsets("GPU-123", 100, null);
    await prism.gpu.resetClockOffsets("GPU-123");

    expect(mocks.invoke.mock.calls).toEqual([
      ["prism:gpu:getInventory"],
      ["prism:gpu:getPerformance"],
      ["prism:gpu:getDaemonStatus"],
      ["prism:gpu:applyConfig", { gpuId: "GPU-123", powerLimitWatts: 450 }],
      ["prism:gpu:resetConfig", { gpuId: "GPU-123" }],
      ["prism:gpu:setPowerLimit", { gpuId: "GPU-123", limitWatts: 450 }],
      ["prism:gpu:resetPowerLimit", { gpuId: "GPU-123" }],
      [
        "prism:gpu:setFanSpeed",
        { gpuId: "GPU-123", fanIndex: 0, speedPercent: 60 },
      ],
      ["prism:gpu:setFanModeAuto", { gpuId: "GPU-123", fanIndex: 0 }],
      [
        "prism:gpu:setClockOffsets",
        { gpuId: "GPU-123", graphicsOffsetMHz: 100, memoryOffsetMHz: null },
      ],
      ["prism:gpu:resetClockOffsets", { gpuId: "GPU-123" }],
    ]);
  });

  it("preserves acknowledged power and fan failures separately from transport rejection", async () => {
    const prism = await loadPrismApi();
    const failure = { success: false, error: "Permission denied" };
    mocks.invoke.mockResolvedValue(failure);
    expect(await prism.gpu.setPowerLimit("GPU-123", 500)).toEqual(failure);
    expect(await prism.gpu.setFanSpeed("GPU-123", 0, 65)).toEqual(failure);
    mocks.invoke.mockRejectedValue(new Error("Connection lost"));
    await expect(prism.gpu.setFanModeAuto("GPU-123", 0)).rejects.toThrow(
      "Connection lost",
    );
  });

  it("exposes profile save/list commands and preserves omitted, numeric, and default setting meanings", async () => {
    const prism = await loadPrismApi();
    const profile = {
      id: "quiet",
      name: "Quiet",
      gpuId: "GPU-123",
      settings: {
        powerLimitWatts: { kind: "value", value: 575 },
        graphicsOffsetMHz: { kind: "default" },
      },
    };
    mocks.invoke.mockResolvedValue(profile);
    expect(
      await prism.gpu.saveProfile({
        gpuId: "GPU-123",
        name: "Quiet",
        appliedDefaults: ["graphicsOffsetMHz"],
      }),
    ).toEqual(profile);
    expect(mocks.invoke).toHaveBeenLastCalledWith("prism:gpu:saveProfile", {
      gpuId: "GPU-123",
      name: "Quiet",
      appliedDefaults: ["graphicsOffsetMHz"],
    });
    mocks.invoke.mockResolvedValue([profile]);
    expect(await prism.gpu.listProfiles()).toEqual([profile]);
    expect(mocks.invoke).toHaveBeenLastCalledWith("prism:gpu:listProfiles");
  });

  it("returns individual clock application outcomes without turning partial failure into success", async () => {
    const prism = await loadPrismApi();
    mocks.invoke.mockResolvedValue(tuningOutcomes);
    expect(await prism.gpu.setClockOffsets("GPU-123", 150, 80)).toEqual(
      tuningOutcomes,
    );
    expect(await prism.gpu.resetClockOffsets("GPU-123")).toEqual(
      tuningOutcomes,
    );
  });

  it("preserves tuning support states, null readings, and reasons across preload", async () => {
    const prism = await loadPrismApi();
    mocks.invoke.mockResolvedValue(tuningObservations);
    expect(await prism.gpu.getPerformance()).toEqual(tuningObservations);
    expect(mocks.invoke).toHaveBeenCalledWith("prism:gpu:getPerformance");
  });

  it("exposes window IPC methods and resize cleanup", async () => {
    const prism = await loadPrismApi();
    const callback = vi.fn();

    await prism.window.minimize();
    await prism.window.maximize();
    await prism.window.unmaximize();
    await prism.window.close();
    await prism.window.isMaximized();
    const cleanup = prism.window.onResized(callback);

    expect(mocks.invoke.mock.calls).toEqual([
      ["prism:window:minimize"],
      ["prism:window:maximize"],
      ["prism:window:unmaximize"],
      ["prism:window:close"],
      ["prism:window:isMaximized"],
    ]);
    expect(mocks.on).toHaveBeenCalledWith(
      "prism:window:resized",
      expect.any(Function),
    );

    const handler = mocks.on.mock.calls[0]?.[1] as (
      event: unknown,
      isMaximized: boolean,
    ) => void;
    handler({}, true);
    expect(callback).toHaveBeenCalledWith(true);

    cleanup();
    expect(mocks.removeListener).toHaveBeenCalledWith(
      "prism:window:resized",
      handler,
    );
  });

  it("exposes driver IPC methods with exact channels and payloads", async () => {
    const prism = await loadPrismApi();

    await prism.drivers.getStatus();
    await prism.drivers.startAction("installRecommended");
    await prism.drivers.getJob("driver-1");
    await prism.drivers.cancelJob("driver-1");
    await prism.drivers.requestReboot();

    expect(mocks.invoke.mock.calls).toEqual([
      ["prism:drivers:getStatus"],
      ["prism:drivers:startAction", { action: "installRecommended" }],
      ["prism:drivers:getJob", { jobId: "driver-1" }],
      ["prism:drivers:cancelJob", { jobId: "driver-1" }],
      ["prism:drivers:requestReboot"],
    ]);
  });

  it("exposes shell IPC methods", async () => {
    const prism = await loadPrismApi();

    await prism.shell.openExternal("https://example.com");

    expect(mocks.invoke).toHaveBeenCalledWith(
      "prism:shell:openExternal",
      "https://example.com",
    );
  });
});

async function loadPrismApi() {
  await import("./preload");

  expect(mocks.exposeInMainWorld).toHaveBeenCalledWith(
    "prism",
    expect.any(Object),
  );
  return mocks.exposeInMainWorld.mock.calls[0]?.[1] as PrismDesktopApi;
}
