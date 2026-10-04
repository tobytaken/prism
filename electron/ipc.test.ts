import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import { mkdtemp, rm, writeFile, readFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";

import { beforeEach, describe, expect, it, vi } from "vitest";

import type { BrowserWindow } from "electron";
import tuningOutcomes from "@/test/contracts/tuning-outcomes.json";
import profileOutcomes from "@/test/contracts/profile-outcomes.json";
import profileCorrections from "@/test/contracts/profile-corrections.json";
import tuningObservations from "@/test/contracts/tuning-observations.json";
import unhealthyStoreStatus from "@/test/contracts/daemon-status-unhealthy-store.json";
import { createPerformanceResponse } from "@/test/gpu-fixtures";
import { registerIpc } from "./ipc";

const mocks = vi.hoisted(() => ({
  handlers: new Map<string, (...args: unknown[]) => unknown>(),
  handle: vi.fn(),
  openExternal: vi.fn(),
  spawn: vi.fn(),
  getPath: vi.fn(),
}));

vi.mock("electron", () => ({
  app: {
    isPackaged: false,
    getPath: mocks.getPath,
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

  it("forwards validated id-less notifications to the subscribed renderer without consuming a pending response", async () => {
    const child = createBackendChild({ success: true });
    mocks.spawn.mockReturnValue(child);
    const send = vi.fn();
    const window = getMainWindow();
    window.webContents.send = send;
    const dispose = registerIpc(() => window);
    try {
      await callHandler("prism:daemon:subscribe", trustedEvent());
      child.stdout.write(
        `${JSON.stringify({ method: "daemon.statusChanged", params: unhealthyStoreStatus })}\n`,
      );
      expect(send).toHaveBeenCalledWith(
        "prism:daemon:statusChanged",
        unhealthyStoreStatus,
      );
      expect(
        await callHandler("prism:gpu:getDaemonStatus", trustedEvent()),
      ).toEqual({ success: true });
      await callHandler("prism:daemon:unsubscribe", trustedEvent());
      send.mockClear();
      child.stdout.write(
        `${JSON.stringify({ method: "daemon.statusChanged", params: unhealthyStoreStatus })}\n`,
      );
      expect(send).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });

  it.each(["subscribe", "unsubscribe"])(
    "validates the sender and payload of daemon %s",
    async (method) => {
      const dispose = registerIpc(getMainWindow);
      try {
        const channel = `prism:daemon:${method}`;
        await expect(callHandler(channel, untrustedEvent())).rejects.toThrow(
          "Untrusted IPC sender",
        );
        await expect(
          callHandler(channel, trustedEvent(), { uid: 0 }),
        ).rejects.toThrow("Invalid daemon subscription payload");
        expect(mocks.spawn).not.toHaveBeenCalled();
      } finally {
        dispose();
      }
    },
  );

  it("ignores malformed notification payloads and envelopes", async () => {
    const child = createBackendChild();
    mocks.spawn.mockReturnValue(child);
    const send = vi.fn();
    const window = getMainWindow();
    window.webContents.send = send;
    const dispose = registerIpc(() => window);
    try {
      await callHandler("prism:daemon:subscribe", trustedEvent());
      for (const value of [
        null,
        [],
        { method: "daemon.statusChanged", params: null },
        {
          method: "daemon.statusChanged",
          params: { ...unhealthyStoreStatus, reachable: "true" },
        },
        {
          method: "daemon.statusChanged",
          params: {
            ...unhealthyStoreStatus,
            tuningStore: {
              status: "unhealthy",
              problem: "unknown",
              path: "/store",
              detail: "problem",
            },
          },
        },
        {
          method: "daemon.statusChanged",
          params: unhealthyStoreStatus,
          id: 99,
        },
        { method: "unknown", params: unhealthyStoreStatus },
      ])
        child.stdout.write(`${JSON.stringify(value)}\n`);
      expect(send).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });

  it("forwards profile application to backend validation and preserves each outcome", async () => {
    const child = createBackendChild(profileOutcomes);
    mocks.spawn.mockReturnValue(child);
    const dispose = registerIpc(getMainWindow);
    const request = {
      gpuId: "GPU-123",
      settings: {
        powerLimitWatts: { kind: "default" },
        graphicsOffsetMHz: { kind: "value", value: -150 },
      },
    };
    try {
      expect(
        await callHandler("prism:gpu:applyProfile", trustedEvent(), request),
      ).toEqual(profileOutcomes);
      expect(JSON.parse(child.stdin.write.mock.calls[0][0])).toMatchObject({
        method: "gpu.applyProfile",
        params: request,
      });
      expect(
        JSON.parse(child.stdin.write.mock.calls[0][0]).params.settings,
      ).not.toHaveProperty("memoryOffsetMHz");
    } finally {
      dispose();
    }
  });

  it("forwards proposed range corrections from fresh backend validation unchanged", async () => {
    const child = createBackendChild(profileCorrections);
    mocks.spawn.mockReturnValue(child);
    const dispose = registerIpc(getMainWindow);
    try {
      expect(
        await callHandler("prism:gpu:applyProfile", trustedEvent(), {
          gpuId: "GPU-123",
          settings: {
            powerLimitWatts: { kind: "value", value: 300 },
            graphicsOffsetMHz: { kind: "value", value: -150 },
          },
        }),
      ).toEqual(profileCorrections);
    } finally {
      dispose();
    }
  });

  it("persists exactly the approved corrections, preserving other settings, without touching the GPU", async () => {
    const directory = await mkdtemp(path.join(os.tmpdir(), "prism-profiles-"));
    mocks.getPath.mockReturnValue(directory);
    const quiet = {
      id: "quiet",
      name: "Quiet",
      gpuId: "GPU-123",
      settings: {
        powerLimitWatts: { kind: "value", value: 300 },
        graphicsOffsetMHz: { kind: "value", value: -150 },
        memoryOffsetMHz: { kind: "default" },
      },
    };
    const other = {
      id: "other",
      name: "Other",
      gpuId: "GPU-123",
      settings: { powerLimitWatts: { kind: "value", value: 300 } },
    };
    await writeFile(
      path.join(directory, "gpu-tuning-profiles.json"),
      JSON.stringify({ version: 1, profiles: [quiet, other] }),
    );
    let dispose = registerIpc(getMainWindow);
    try {
      const corrected = {
        ...quiet,
        settings: {
          ...quiet.settings,
          powerLimitWatts: { kind: "value", value: 280 },
          graphicsOffsetMHz: { kind: "value", value: -100 },
        },
      };
      expect(
        await callHandler("prism:gpu:correctProfile", trustedEvent(), {
          profileId: "quiet",
          corrections: profileCorrections.corrections,
        }),
      ).toEqual(corrected);
      dispose();
      dispose = registerIpc(getMainWindow);
      expect(
        await callHandler("prism:gpu:listProfiles", trustedEvent()),
      ).toEqual([corrected, other]);
      expect(mocks.spawn).not.toHaveBeenCalled();
    } finally {
      dispose();
      await rm(directory, { recursive: true, force: true });
    }
  });

  it.each([
    {
      label: "a saved value that changed during review",
      corrections: [
        {
          setting: "powerLimitWatts",
          savedValue: 320,
          proposedValue: 280,
          bound: "maximum",
        },
      ],
      reason: /changed while its corrections were being reviewed/,
    },
    {
      label: "an explicit default",
      corrections: [
        {
          setting: "memoryOffsetMHz",
          savedValue: 80,
          proposedValue: 50,
          bound: "maximum",
        },
      ],
      reason: /changed while its corrections were being reviewed/,
    },
    {
      label: "an omitted setting",
      profileId: "power-only",
      corrections: [
        {
          setting: "graphicsOffsetMHz",
          savedValue: -150,
          proposedValue: -100,
          bound: "minimum",
        },
      ],
      reason: /changed while its corrections were being reviewed/,
    },
    {
      label: "a missing profile",
      profileId: "deleted",
      corrections: profileCorrections.corrections,
      reason: /no longer exists/,
    },
  ])(
    "rejects corrections for $label and leaves every profile unchanged",
    async ({ profileId = "quiet", corrections, reason }) => {
      const directory = await mkdtemp(
        path.join(os.tmpdir(), "prism-profiles-"),
      );
      mocks.getPath.mockReturnValue(directory);
      const file = path.join(directory, "gpu-tuning-profiles.json");
      const original = JSON.stringify({
        version: 1,
        profiles: [
          {
            id: "quiet",
            name: "Quiet",
            gpuId: "GPU-123",
            settings: {
              powerLimitWatts: { kind: "value", value: 300 },
              memoryOffsetMHz: { kind: "default" },
            },
          },
          {
            id: "power-only",
            name: "Power only",
            gpuId: "GPU-123",
            settings: { powerLimitWatts: { kind: "value", value: 300 } },
          },
        ],
      });
      await writeFile(file, original);
      const dispose = registerIpc(getMainWindow);
      try {
        await expect(
          callHandler("prism:gpu:correctProfile", trustedEvent(), {
            profileId,
            corrections,
          }),
        ).rejects.toThrow(reason);
        expect(await readFile(file, "utf8")).toBe(original);
        expect(mocks.spawn).not.toHaveBeenCalled();
      } finally {
        dispose();
        await rm(directory, { recursive: true, force: true });
      }
    },
  );

  it("reports unreadable profile storage when persisting corrections and preserves the file", async () => {
    const directory = await mkdtemp(path.join(os.tmpdir(), "prism-profiles-"));
    mocks.getPath.mockReturnValue(directory);
    const file = path.join(directory, "gpu-tuning-profiles.json");
    await writeFile(file, "{ not json");
    const dispose = registerIpc(getMainWindow);
    try {
      await expect(
        callHandler("prism:gpu:correctProfile", trustedEvent(), {
          profileId: "quiet",
          corrections: profileCorrections.corrections,
        }),
      ).rejects.toThrow();
      expect(await readFile(file, "utf8")).toBe("{ not json");
    } finally {
      dispose();
      await rm(directory, { recursive: true, force: true });
    }
  });

  it("rejects untrusted or malformed correction requests before reading profiles", async () => {
    const directory = await mkdtemp(path.join(os.tmpdir(), "prism-profiles-"));
    mocks.getPath.mockReturnValue(directory);
    const dispose = registerIpc(getMainWindow);
    const correction = profileCorrections.corrections[0];
    try {
      await expect(
        callHandler("prism:gpu:correctProfile", untrustedEvent(), {
          profileId: "quiet",
          corrections: [correction],
        }),
      ).rejects.toThrow("Untrusted IPC sender");
      for (const payload of [
        undefined,
        { profileId: "quiet", corrections: [] },
        { profileId: " ", corrections: [correction] },
        { profileId: "quiet", corrections: [correction], gpuId: "GPU-123" },
        { profileId: "quiet", corrections: [correction, correction] },
        {
          profileId: "quiet",
          corrections: [{ ...correction, setting: "fanSpeed" }],
        },
        {
          profileId: "quiet",
          corrections: [{ ...correction, proposedValue: 280.5 }],
        },
        {
          profileId: "quiet",
          corrections: [{ ...correction, bound: "minimum" }],
        },
        {
          profileId: "quiet",
          corrections: [{ ...correction, proposedValue: 320 }],
        },
        {
          profileId: "quiet",
          corrections: [{ ...correction, extra: true }],
        },
      ]) {
        await expect(
          callHandler("prism:gpu:correctProfile", trustedEvent(), payload),
        ).rejects.toThrow("Invalid correctProfile payload");
      }
      expect(mocks.spawn).not.toHaveBeenCalled();
    } finally {
      dispose();
      await rm(directory, { recursive: true, force: true });
    }
  });

  it("rejects untrusted or malformed profile application without invoking the backend", async () => {
    const dispose = registerIpc(getMainWindow);
    try {
      await expect(
        callHandler("prism:gpu:applyProfile", untrustedEvent(), {
          gpuId: "GPU-123",
          settings: { powerLimitWatts: { kind: "value", value: 450 } },
        }),
      ).rejects.toThrow("Untrusted IPC sender");
      const value = (value: unknown) => ({ kind: "value", value });
      for (const payload of [
        null,
        {},
        { gpuId: " ", settings: { powerLimitWatts: value(450) } },
        { gpuId: "GPU-123", settings: {} },
        { gpuId: "GPU-123", settings: { powerLimitWatts: null } },
        { gpuId: "GPU-123", settings: { powerLimitWatts: 450 } },
        { gpuId: "GPU-123", settings: { powerLimitWatts: value(450.5) } },
        { gpuId: "GPU-123", settings: { powerLimitWatts: value(Infinity) } },
        {
          gpuId: "GPU-123",
          settings: { powerLimitWatts: value(Number.MAX_SAFE_INTEGER + 1) },
        },
        { gpuId: "GPU-123", settings: { powerLimitWatts: value("450") } },
        { gpuId: "GPU-123", settings: { powerLimitWatts: { kind: "value" } } },
        {
          gpuId: "GPU-123",
          settings: { powerLimitWatts: { kind: "default", value: 450 } },
        },
        {
          gpuId: "GPU-123",
          settings: { powerLimitWatts: { kind: "reset" } },
        },
        {
          gpuId: "GPU-123",
          settings: { powerLimitWatts: value(450), fanSpeed: value(80) },
        },
        {
          gpuId: "GPU-123",
          settings: {
            graphicsOffsetMHz: value(120),
            gpuClockLock: value(1800),
          },
        },
        {
          gpuId: "GPU-123",
          settings: { powerLimitWatts: value(450) },
          reapplySaved: true,
        },
      ]) {
        await expect(
          callHandler("prism:gpu:applyProfile", trustedEvent(), payload),
        ).rejects.toThrow("Invalid applyProfile payload");
      }
      expect(mocks.spawn).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });

  it("saves known numeric profile settings without tuning and retains them after app reload", async () => {
    const directory = await mkdtemp(path.join(os.tmpdir(), "prism-profiles-"));
    mocks.getPath.mockReturnValue(directory);
    const readings = createPerformanceResponse();
    readings.gpus[0].power.currentLimitWatts = 575;
    readings.gpus[0].power.currentValueError = null;
    const child = createBackendChild(readings);
    mocks.spawn.mockReturnValue(child);
    let dispose = registerIpc(getMainWindow);
    try {
      const saved = await callHandler("prism:gpu:saveProfile", trustedEvent(), {
        gpuId: "GPU-123",
        name: "Quiet",
      });
      expect(saved).toEqual({
        id: expect.any(String),
        name: "Quiet",
        gpuId: "GPU-123",
        settings: {
          powerLimitWatts: { kind: "value", value: 575 },
          graphicsOffsetMHz: { kind: "value", value: 120 },
        },
      });
      expect(
        child.stdin.write.mock.calls.map(
          ([payload]) => JSON.parse(payload).method,
        ),
      ).toEqual(["gpu.getPerformance"]);
      dispose();
      dispose = registerIpc(getMainWindow);
      expect(
        await callHandler("prism:gpu:listProfiles", trustedEvent()),
      ).toEqual([saved]);
    } finally {
      dispose();
      await rm(directory, { recursive: true, force: true });
    }
  });

  it("captures all supported settings from fresh readings and gives each saved profile its own identity", async () => {
    const directory = await mkdtemp(path.join(os.tmpdir(), "prism-profiles-"));
    mocks.getPath.mockReturnValue(directory);
    const readings = createPerformanceResponse();
    Object.assign(readings.gpus[0].clocks.memory, {
      support: "supported",
      currentOffsetMHz: 80,
      reason: null,
    });
    const child = createBackendChild(readings);
    mocks.spawn.mockReturnValue(child);
    const dispose = registerIpc(getMainWindow);
    try {
      const first = await callHandler("prism:gpu:saveProfile", trustedEvent(), {
        gpuId: "GPU-123",
        name: "Quiet",
        settings: { powerLimitWatts: 999 },
      });
      readings.gpus[0].power.currentLimitWatts = 450;
      const second = await callHandler(
        "prism:gpu:saveProfile",
        trustedEvent(),
        { gpuId: "GPU-123", name: "Quiet" },
      );
      expect(first).toMatchObject({
        settings: {
          powerLimitWatts: { kind: "value", value: 475 },
          graphicsOffsetMHz: { kind: "value", value: 120 },
          memoryOffsetMHz: { kind: "value", value: 80 },
        },
      });
      expect(second).toMatchObject({
        id: expect.any(String),
        gpuId: "GPU-123",
        settings: { powerLimitWatts: { kind: "value", value: 450 } },
      });
      expect(second).not.toEqual(first);
      expect(
        await callHandler("prism:gpu:listProfiles", trustedEvent()),
      ).toEqual([first, second]);
      expect(
        child.stdin.write.mock.calls.map(
          ([payload]) => JSON.parse(payload).method,
        ),
      ).toEqual(["gpu.getPerformance", "gpu.getPerformance"]);
    } finally {
      dispose();
      await rm(directory, { recursive: true, force: true });
    }
  });

  it("saves applied explicit default choices only while fresh readings still reflect them, and retains them after reload", async () => {
    const directory = await mkdtemp(path.join(os.tmpdir(), "prism-profiles-"));
    mocks.getPath.mockReturnValue(directory);
    const readings = createPerformanceResponse();
    Object.assign(readings.gpus[0].power, {
      currentLimitWatts: 575,
      defaultLimitWatts: 575,
    });
    readings.gpus[0].clocks.graphics.currentOffsetMHz = 0;
    const child = createBackendChild(readings);
    mocks.spawn.mockReturnValue(child);
    let dispose = registerIpc(getMainWindow);
    try {
      const numeric = await callHandler(
        "prism:gpu:saveProfile",
        trustedEvent(),
        { gpuId: "GPU-123", name: "Numeric" },
      );
      const defaults = await callHandler(
        "prism:gpu:saveProfile",
        trustedEvent(),
        {
          gpuId: "GPU-123",
          name: "Defaults",
          appliedDefaults: ["powerLimitWatts", "graphicsOffsetMHz"],
        },
      );
      // The driver now reports another default, so the earlier choice no longer describes the GPU.
      readings.gpus[0].power.defaultLimitWatts = 520;
      const stale = await callHandler("prism:gpu:saveProfile", trustedEvent(), {
        gpuId: "GPU-123",
        name: "Stale",
        appliedDefaults: ["powerLimitWatts"],
      });
      expect(numeric).toMatchObject({
        settings: {
          powerLimitWatts: { kind: "value", value: 575 },
          graphicsOffsetMHz: { kind: "value", value: 0 },
        },
      });
      expect(defaults).toMatchObject({
        settings: {
          powerLimitWatts: { kind: "default" },
          graphicsOffsetMHz: { kind: "default" },
        },
      });
      expect(stale).toMatchObject({
        settings: {
          powerLimitWatts: { kind: "value", value: 575 },
          graphicsOffsetMHz: { kind: "value", value: 0 },
        },
      });
      dispose();
      dispose = registerIpc(getMainWindow);
      expect(
        await callHandler("prism:gpu:listProfiles", trustedEvent()),
      ).toEqual([numeric, defaults, stale]);
    } finally {
      dispose();
      await rm(directory, { recursive: true, force: true });
    }
  });

  it("reloads omitted, numeric, and explicit-default settings through the public list command without a GPU", async () => {
    const directory = await mkdtemp(path.join(os.tmpdir(), "prism-profiles-"));
    mocks.getPath.mockReturnValue(directory);
    const profile = {
      id: "future-default",
      name: "Default clocks",
      gpuId: "GPU-absent",
      settings: {
        powerLimitWatts: { kind: "value", value: 575 },
        graphicsOffsetMHz: { kind: "default" },
      },
    };
    await writeFile(
      path.join(directory, "gpu-tuning-profiles.json"),
      JSON.stringify({ version: 1, profiles: [profile] }),
    );
    let dispose = registerIpc(getMainWindow);
    try {
      expect(
        await callHandler("prism:gpu:listProfiles", trustedEvent()),
      ).toEqual([profile]);
      dispose();
      dispose = registerIpc(getMainWindow);
      expect(
        await callHandler("prism:gpu:listProfiles", trustedEvent()),
      ).toEqual([profile]);
      expect(mocks.spawn).not.toHaveBeenCalled();
    } finally {
      dispose();
      await rm(directory, { recursive: true, force: true });
    }
  });

  it.each([
    {
      label: "missing stable identity",
      change: { stableId: null },
      reason: /stable identity/,
    },
    {
      label: "absent GPU",
      change: { id: "GPU-other", stableId: "GPU-other" },
      reason: /owning GPU is unavailable/,
    },
  ])(
    "rejects profile saving for $label at the owner boundary",
    async ({ change, reason }) => {
      const directory = await mkdtemp(
        path.join(os.tmpdir(), "prism-profiles-"),
      );
      mocks.getPath.mockReturnValue(directory);
      const readings = createPerformanceResponse();
      Object.assign(readings.gpus[0], change);
      mocks.spawn.mockReturnValue(createBackendChild(readings));
      const dispose = registerIpc(getMainWindow);
      try {
        await expect(
          callHandler("prism:gpu:saveProfile", trustedEvent(), {
            gpuId: "GPU-123",
            name: "Quiet",
          }),
        ).rejects.toThrow(reason);
        expect(
          await callHandler("prism:gpu:listProfiles", trustedEvent()),
        ).toEqual([]);
      } finally {
        dispose();
        await rm(directory, { recursive: true, force: true });
      }
    },
  );

  it("rejects untrusted or invalid profile requests", async () => {
    const dispose = registerIpc(getMainWindow);
    try {
      await expect(
        callHandler("prism:gpu:listProfiles", untrustedEvent()),
      ).rejects.toThrow("Untrusted IPC sender");
      await expect(
        callHandler("prism:gpu:saveProfile", untrustedEvent(), {
          gpuId: "GPU-123",
          name: "Quiet",
        }),
      ).rejects.toThrow("Untrusted IPC sender");
      for (const payload of [
        { gpuId: "GPU-123", name: " " },
        { gpuId: 0, name: "Quiet" },
        { name: "Quiet" },
        { gpuId: "GPU-123", name: "Quiet", appliedDefaults: "powerLimitWatts" },
        { gpuId: "GPU-123", name: "Quiet", appliedDefaults: ["fanSpeed"] },
        {
          gpuId: "GPU-123",
          name: "Quiet",
          appliedDefaults: ["powerLimitWatts", "powerLimitWatts"],
        },
      ]) {
        await expect(
          callHandler("prism:gpu:saveProfile", trustedEvent(), payload),
        ).rejects.toThrow("Invalid saveProfile payload");
      }
      expect(mocks.spawn).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });

  it("rejects stored profile values that the application boundaries could never accept", async () => {
    const directory = await mkdtemp(path.join(os.tmpdir(), "prism-profiles-"));
    mocks.getPath.mockReturnValue(directory);
    await writeFile(
      path.join(directory, "gpu-tuning-profiles.json"),
      JSON.stringify({
        version: 1,
        profiles: [
          {
            id: "fractional",
            name: "Fractional",
            gpuId: "GPU-123",
            settings: { powerLimitWatts: { kind: "value", value: 250.5 } },
          },
        ],
      }),
    );
    const dispose = registerIpc(getMainWindow);
    try {
      await expect(
        callHandler("prism:gpu:listProfiles", trustedEvent()),
      ).rejects.toThrow("profile file has been preserved");
    } finally {
      dispose();
      await rm(directory, { recursive: true, force: true });
    }
  });

  it("preserves unreadable storage and reports errors instead of overwriting retained profiles", async () => {
    const directory = await mkdtemp(path.join(os.tmpdir(), "prism-profiles-"));
    mocks.getPath.mockReturnValue(directory);
    const file = path.join(directory, "gpu-tuning-profiles.json");
    const original = JSON.stringify({
      version: 2,
      profiles: [{ id: "existing" }],
    });
    await writeFile(file, original);
    mocks.spawn.mockReturnValue(
      createBackendChild(createPerformanceResponse()),
    );
    const dispose = registerIpc(getMainWindow);
    try {
      await expect(
        callHandler("prism:gpu:listProfiles", trustedEvent()),
      ).rejects.toThrow("profile file has been preserved");
      await expect(
        callHandler("prism:gpu:saveProfile", trustedEvent(), {
          gpuId: "GPU-123",
          name: "Quiet",
        }),
      ).rejects.toThrow("profile file has been preserved");
      expect(await readFile(file, "utf8")).toBe(original);
    } finally {
      dispose();
      await rm(directory, { recursive: true, force: true });
    }
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
