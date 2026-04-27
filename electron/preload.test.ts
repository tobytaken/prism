import { beforeEach, describe, expect, it, vi } from "vitest";

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
      ["prism:gpu:setFanSpeed", { gpuId: "GPU-123", fanIndex: 0, speedPercent: 60 }],
      ["prism:gpu:setFanModeAuto", { gpuId: "GPU-123", fanIndex: 0 }],
      [
        "prism:gpu:setClockOffsets",
        { gpuId: "GPU-123", graphicsOffsetMHz: 100, memoryOffsetMHz: null },
      ],
      ["prism:gpu:resetClockOffsets", { gpuId: "GPU-123" }],
    ]);
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
    expect(mocks.on).toHaveBeenCalledWith("prism:window:resized", expect.any(Function));

    const handler = mocks.on.mock.calls[0]?.[1] as (
      event: unknown,
      isMaximized: boolean,
    ) => void;
    handler({}, true);
    expect(callback).toHaveBeenCalledWith(true);

    cleanup();
    expect(mocks.removeListener).toHaveBeenCalledWith("prism:window:resized", handler);
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

  expect(mocks.exposeInMainWorld).toHaveBeenCalledWith("prism", expect.any(Object));
  return mocks.exposeInMainWorld.mock.calls[0]?.[1] as PrismDesktopApi;
}
