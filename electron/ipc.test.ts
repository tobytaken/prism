import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";

import { beforeEach, describe, expect, it, vi } from "vitest";

import type { BrowserWindow } from "electron";
import tuningOutcomes from "@/test/contracts/tuning-outcomes.json";
import tuningObservations from "@/test/contracts/tuning-observations.json";
import { registerIpc } from "./ipc";

const mocks = vi.hoisted(() => ({
  handlers: new Map<string, (...args: unknown[]) => unknown>(),
  handle: vi.fn(),
  openExternal: vi.fn(),
  spawn: vi.fn(),
}));

vi.mock("electron", () => ({
  app: {
    isPackaged: false,
  },
  BrowserWindow: vi.fn(),
  ipcMain: {
    handle: mocks.handle,
  },
  shell: {
    openExternal: mocks.openExternal,
  },
}));

vi.mock("node:child_process", () => ({
  default: {
    spawn: mocks.spawn,
  },
  spawn: mocks.spawn,
}));

describe("registerIpc", () => {
  beforeEach(() => {
    mocks.handlers.clear();
    mocks.handle.mockReset();
    mocks.handle.mockImplementation((channel, handler) => {
      mocks.handlers.set(channel, handler);
    });
    mocks.openExternal.mockReset();
    mocks.spawn.mockReset();
  });

  it("rejects untrusted senders", async () => {
    registerIpc(getMainWindow);

    await expect(
      callHandler("prism:window:isMaximized", untrustedEvent()),
    ).rejects.toThrow("Untrusted IPC sender");
  });

  it("rejects invalid mutation payloads", async () => {
    registerIpc(getMainWindow);

    await expect(
      callHandler("prism:gpu:setPowerLimit", trustedEvent(), {
        gpuId: "GPU-123",
      }),
    ).rejects.toThrow("Invalid setPowerLimit payload");
    await expect(
      callHandler("prism:gpu:setFanSpeed", trustedEvent(), {
        gpuId: "GPU-123",
        fanIndex: "0",
        speedPercent: 60,
      }),
    ).rejects.toThrow("Invalid setFanSpeed payload");
    await expect(
      callHandler("prism:gpu:setClockOffsets", trustedEvent(), {
        gpuId: "GPU-123",
        graphicsOffsetMHz: "100",
        memoryOffsetMHz: null,
      }),
    ).rejects.toThrow("Invalid setClockOffsets payload");
    await expect(
      callHandler("prism:gpu:resetConfig", trustedEvent(), { gpuId: 123 }),
    ).rejects.toThrow("Invalid resetConfig payload");
    await expect(
      callHandler("prism:gpu:applyConfig", trustedEvent(), {
        powerLimitWatts: "450",
      }),
    ).rejects.toThrow("Invalid applyConfig payload");
    await expect(
      callHandler("prism:drivers:startAction", trustedEvent(), {
        action: "removeEverything",
      }),
    ).rejects.toThrow("Invalid startAction payload");
    await expect(
      callHandler("prism:drivers:getJob", trustedEvent(), { jobId: 123 }),
    ).rejects.toThrow("Invalid getJob payload");
  });

  it("preserves an acknowledged GPU command failure from the sidecar", async () => {
    const failure = { success: false, error: "Permission denied" };
    mocks.spawn.mockReturnValue(createBackendChild(failure));
    const dispose = registerIpc(getMainWindow);
    try {
      expect(
        await callHandler("prism:gpu:setPowerLimit", trustedEvent(), {
          gpuId: "GPU-123",
          limitWatts: 500,
        }),
      ).toEqual(failure);
    } finally {
      dispose();
    }
  });

  it("preserves mixed clock outcomes through the sidecar transport", async () => {
    mocks.spawn.mockReturnValue(createBackendChild(tuningOutcomes));
    const dispose = registerIpc(getMainWindow);
    try {
      expect(
        await callHandler("prism:gpu:setClockOffsets", trustedEvent(), {
          gpuId: "GPU-123",
          graphicsOffsetMHz: 150,
          memoryOffsetMHz: 80,
        }),
      ).toEqual(tuningOutcomes);
    } finally {
      dispose();
    }
  });

  it("preserves tuning observations from the sidecar response", async () => {
    mocks.spawn.mockReturnValue(createBackendChild(tuningObservations));
    const dispose = registerIpc(getMainWindow);
    try {
      expect(
        await callHandler("prism:gpu:getPerformance", trustedEvent()),
      ).toEqual(tuningObservations);
    } finally {
      dispose();
    }
  });

  it("passes valid GPU payloads to the backend", async () => {
    const child = createBackendChild();
    mocks.spawn.mockReturnValue(child);
    const dispose = registerIpc(getMainWindow);

    await callHandler("prism:gpu:setPowerLimit", trustedEvent(), {
      gpuId: "GPU-123",
      limitWatts: 450,
    });

    expect(child.stdin.write).toHaveBeenCalledWith(
      expect.stringContaining('"method":"gpu.setPowerLimit"'),
    );
    expect(child.stdin.write).toHaveBeenCalledWith(
      expect.stringContaining('"limitWatts":450'),
    );

    dispose();
    expect(child.kill).toHaveBeenCalledTimes(1);
  });

  it("passes valid driver payloads to the backend", async () => {
    const child = createBackendChild();
    mocks.spawn.mockReturnValue(child);
    const dispose = registerIpc(getMainWindow);

    await callHandler("prism:drivers:startAction", trustedEvent(), {
      action: "installRecommended",
    });
    await callHandler("prism:drivers:getJob", trustedEvent(), {
      jobId: "driver-1",
    });
    await callHandler("prism:drivers:cancelJob", trustedEvent(), {
      jobId: "driver-1",
    });

    expect(child.stdin.write).toHaveBeenCalledWith(
      expect.stringContaining('"method":"drivers.startAction"'),
    );
    expect(child.stdin.write).toHaveBeenCalledWith(
      expect.stringContaining('"action":"installRecommended"'),
    );
    expect(child.stdin.write).toHaveBeenCalledWith(
      expect.stringContaining('"method":"drivers.getJob"'),
    );
    expect(child.stdin.write).toHaveBeenCalledWith(
      expect.stringContaining('"method":"drivers.cancelJob"'),
    );

    dispose();
  });

  it("allows only safe external URLs", async () => {
    registerIpc(getMainWindow);

    await callHandler(
      "prism:shell:openExternal",
      trustedEvent(),
      "https://example.com",
    );
    await callHandler(
      "prism:shell:openExternal",
      trustedEvent(),
      "mailto:test@example.com",
    );

    await expect(
      callHandler(
        "prism:shell:openExternal",
        trustedEvent(),
        "http://example.com",
      ),
    ).rejects.toThrow("Blocked external URL");
    await expect(
      callHandler(
        "prism:shell:openExternal",
        trustedEvent(),
        "file:///etc/passwd",
      ),
    ).rejects.toThrow("Blocked external URL");

    expect(mocks.openExternal).toHaveBeenCalledWith("https://example.com");
    expect(mocks.openExternal).toHaveBeenCalledWith("mailto:test@example.com");
    expect(mocks.spawn).not.toHaveBeenCalled();
  });
});

function getMainWindow(): BrowserWindow {
  return {
    webContents: { id: 42 },
    minimize: vi.fn(),
    maximize: vi.fn(),
    unmaximize: vi.fn(),
    close: vi.fn(),
    isMaximized: vi.fn(() => false),
  } as unknown as BrowserWindow;
}

function trustedEvent() {
  return { sender: { id: 42 } };
}

function untrustedEvent() {
  return { sender: { id: 7 } };
}

function callHandler(channel: string, ...args: unknown[]) {
  const handler = mocks.handlers.get(channel);
  expect(handler).toBeDefined();
  return Promise.resolve(handler?.(...args));
}

function createBackendChild(result: unknown = null) {
  const child = new EventEmitter() as EventEmitter & {
    stdin: { write: ReturnType<typeof vi.fn> };
    stdout: PassThrough;
    stderr: PassThrough;
    exitCode: number | null;
    killed: boolean;
    kill: ReturnType<typeof vi.fn>;
  };

  child.stdout = new PassThrough();
  child.stderr = new PassThrough();
  child.exitCode = null;
  child.killed = false;
  child.stdin = {
    write: vi.fn((payload: string) => {
      const request = JSON.parse(payload) as { id: number };
      queueMicrotask(() => {
        child.stdout.write(`${JSON.stringify({ id: request.id, result })}\n`);
      });
      return true;
    }),
  };
  child.kill = vi.fn(() => {
    child.killed = true;
    child.exitCode = 0;
    child.emit("exit", 0, null);
  });

  return child;
}
