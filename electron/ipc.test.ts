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
import correctedProfileApplication from "@/test/contracts/corrected-profile-application.json";
import gpuTuningProfiles from "@/test/contracts/gpu-tuning-profiles.json";
import gpuTuningStates from "@/test/contracts/gpu-tuning-states.json";
import { connectedDaemonStatus } from "@/test/gpu-fixtures";
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
    // No profile file to import unless a test writes one.
    mocks.getPath.mockReset();
    mocks.getPath.mockReturnValue(path.join(os.tmpdir(), "prism-no-user-data"));
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

  it("forwards validated tuning state changes to the subscribed renderer", async () => {
    const child = createBackendChild();
    mocks.spawn.mockReturnValue(child);
    const send = vi.fn();
    const window = getMainWindow();
    window.webContents.send = send;
    const dispose = registerIpc(() => window);
    try {
      await callHandler("prism:daemon:subscribe", trustedEvent());
      for (const state of gpuTuningStates)
        child.stdout.write(
          `${JSON.stringify({ method: "gpu.tuningStateChanged", params: state })}\n`,
        );
      expect(send.mock.calls).toEqual(
        gpuTuningStates.map((state) => ["prism:gpu:tuningStateChanged", state]),
      );
      send.mockClear();
      for (const params of [
        null,
        { ...gpuTuningStates[0], revision: 7 },
        { ...gpuTuningStates[0], currentProfile: { id: "quiet" } },
        { ...gpuTuningStates[0], uid: 0 },
        { ...gpuTuningStates[0], reported: { powerLimitWatts: 300 } },
        {
          ...gpuTuningStates[0],
          reported: { ...gpuTuningStates[0].reported, memoryOffsetMHz: "0" },
        },
        { gpuId: "GPU-123", revision: "r" },
      ])
        child.stdout.write(
          `${JSON.stringify({ method: "gpu.tuningStateChanged", params })}\n`,
        );
      expect(send).not.toHaveBeenCalled();
      await callHandler("prism:daemon:unsubscribe", trustedEvent());
      child.stdout.write(
        `${JSON.stringify({ method: "gpu.tuningStateChanged", params: gpuTuningStates[0] })}\n`,
      );
      expect(send).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });

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

  it("reports a sidecar crash, restarts its session, and cancels restart on disposal", async () => {
    vi.useFakeTimers();
    const first = createBackendChild();
    const second = createBackendChild();
    mocks.spawn.mockReturnValueOnce(first).mockReturnValueOnce(second);
    const send = vi.fn();
    const window = getMainWindow();
    window.webContents.send = send;
    const dispose = registerIpc(() => window);
    try {
      await callHandler("prism:daemon:subscribe", trustedEvent());
      first.stdout.write(
        `${JSON.stringify({ method: "daemon.statusChanged", params: connectedDaemonStatus })}\n`,
      );
      first.emit("exit", 1, null);
      expect(send).toHaveBeenLastCalledWith(
        "prism:daemon:statusChanged",
        expect.objectContaining({ reachable: false, tuningStore: null }),
      );
      await vi.advanceTimersByTimeAsync(250);
      expect(mocks.spawn).toHaveBeenCalledTimes(2);
      second.stdout.write(
        `${JSON.stringify({ method: "daemon.statusChanged", params: connectedDaemonStatus })}\n`,
      );
      expect(send).toHaveBeenLastCalledWith(
        "prism:daemon:statusChanged",
        connectedDaemonStatus,
      );
      second.emit("exit", 1, null);
      dispose();
      await vi.advanceTimersByTimeAsync(1_000);
      expect(mocks.spawn).toHaveBeenCalledTimes(2);
    } finally {
      dispose();
      vi.useRealTimers();
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

  it("forwards profile application by id and correct-and-apply to the daemon unchanged", async () => {
    const child = createBackendChild((method: string) =>
      method === "gpu.applyProfileById"
        ? profileCorrections
        : correctedProfileApplication,
    );
    mocks.spawn.mockReturnValue(child);
    const dispose = registerIpc(getMainWindow);
    try {
      expect(
        await callHandler("prism:gpu:applyProfileById", trustedEvent(), {
          profileId: "quiet",
        }),
      ).toEqual(profileCorrections);
      const request = {
        profileId: "quiet",
        corrections: profileCorrections.corrections,
      };
      expect(
        await callHandler(
          "prism:gpu:correctAndApplyProfile",
          trustedEvent(),
          request,
        ),
      ).toEqual(correctedProfileApplication);
      expect(sentRequests(child)).toEqual([
        { method: "gpu.applyProfileById", params: { profileId: "quiet" } },
        { method: "gpu.correctAndApplyProfile", params: request },
      ]);
    } finally {
      dispose();
    }
  });

  it("rejects untrusted or malformed correction requests before the daemon sees them", async () => {
    const dispose = registerIpc(getMainWindow);
    const correction = profileCorrections.corrections[0];
    try {
      await expect(
        callHandler("prism:gpu:correctAndApplyProfile", untrustedEvent(), {
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
          callHandler(
            "prism:gpu:correctAndApplyProfile",
            trustedEvent(),
            payload,
          ),
        ).rejects.toThrow("Invalid correctAndApplyProfile payload");
      }
      await expect(
        callHandler("prism:gpu:applyProfileById", untrustedEvent(), {
          profileId: "quiet",
        }),
      ).rejects.toThrow("Untrusted IPC sender");
      for (const payload of [
        undefined,
        {},
        { profileId: " " },
        { profileId: 7 },
        { profileId: "quiet", gpuId: "GPU-123" },
      ]) {
        await expect(
          callHandler("prism:gpu:applyProfileById", trustedEvent(), payload),
        ).rejects.toThrow("Invalid applyProfileById payload");
      }
      expect(mocks.spawn).not.toHaveBeenCalled();
    } finally {
      dispose();
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

  it("forwards profile rename and delete to the daemon, preserving profile identity", async () => {
    const renamed = { ...gpuTuningProfiles[0], name: "Silent" };
    const deleted = { success: true, error: null };
    const child = createBackendChild((method: string) =>
      method === "gpu.renameProfile" ? renamed : deleted,
    );
    mocks.spawn.mockReturnValue(child);
    const dispose = registerIpc(getMainWindow);
    try {
      const rename = { profileId: renamed.id, name: "  Silent  " };
      const remove = { profileId: renamed.id };
      expect(
        await callHandler("prism:gpu:renameProfile", trustedEvent(), rename),
      ).toEqual(renamed);
      expect(
        await callHandler("prism:gpu:deleteProfile", trustedEvent(), remove),
      ).toEqual(deleted);
      expect(sentRequests(child)).toEqual([
        { method: "gpu.renameProfile", params: rename },
        { method: "gpu.deleteProfile", params: remove },
      ]);
    } finally {
      dispose();
    }
  });

  it("rejects untrusted or malformed profile management requests before reaching the daemon", async () => {
    const dispose = registerIpc(getMainWindow);
    try {
      for (const command of ["renameProfile", "deleteProfile"]) {
        await expect(
          callHandler(`prism:gpu:${command}`, untrustedEvent(), {
            profileId: "quiet",
            name: "Silent",
          }),
        ).rejects.toThrow("Untrusted IPC sender");
      }
      for (const payload of [
        undefined,
        {},
        { profileId: "", name: "Silent" },
        { profileId: 7, name: "Silent" },
        { profileId: "quiet", name: " \n " },
        { profileId: "quiet", name: 7 },
        { profileId: "quiet", name: "Silent", uid: 0 },
        { profileId: "quiet", name: "Silent", gpuId: "GPU-456" },
      ]) {
        await expect(
          callHandler("prism:gpu:renameProfile", trustedEvent(), payload),
        ).rejects.toThrow("Invalid renameProfile payload");
      }
      for (const payload of [
        undefined,
        {},
        { profileId: " " },
        { profileId: 7 },
        { profileId: "quiet", uid: 0 },
        { profileId: "quiet", gpuId: "GPU-456" },
      ]) {
        await expect(
          callHandler("prism:gpu:deleteProfile", trustedEvent(), payload),
        ).rejects.toThrow("Invalid deleteProfile payload");
      }
      expect(mocks.spawn).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });

  it("saves as new or to the current profile, and lists profiles and tuning states, through the daemon", async () => {
    const [saved] = gpuTuningProfiles;
    const child = createBackendChild((method: string) =>
      method === "gpu.saveProfileAsNew" || method === "gpu.saveProfileToCurrent"
        ? saved
        : method === "gpu.getTuningStates"
          ? gpuTuningStates
          : gpuTuningProfiles,
    );
    mocks.spawn.mockReturnValue(child);
    const dispose = registerIpc(getMainWindow);
    try {
      expect(
        await callHandler("prism:gpu:saveProfile", trustedEvent(), {
          gpuId: "GPU-123",
          name: "Quiet",
          revision: "4f1c2b9d0e8a7c35",
        }),
      ).toEqual(saved);
      expect(
        await callHandler("prism:gpu:saveProfileToCurrent", trustedEvent(), {
          gpuId: "GPU-123",
          revision: "4f1c2b9d0e8a7c35",
        }),
      ).toEqual(saved);
      expect(
        await callHandler("prism:gpu:listProfiles", trustedEvent()),
      ).toEqual(gpuTuningProfiles);
      expect(
        await callHandler("prism:gpu:getTuningStates", trustedEvent()),
      ).toEqual(gpuTuningStates);
      expect(sentRequests(child)).toEqual([
        {
          method: "gpu.saveProfileAsNew",
          params: {
            gpuId: "GPU-123",
            name: "Quiet",
            revision: "4f1c2b9d0e8a7c35",
          },
        },
        {
          method: "gpu.saveProfileToCurrent",
          params: { gpuId: "GPU-123", revision: "4f1c2b9d0e8a7c35" },
        },
        { method: "gpu.listProfiles" },
        { method: "gpu.getTuningStates" },
      ]);
    } finally {
      dispose();
    }
  });

  it("rejects untrusted or invalid profile requests", async () => {
    const dispose = registerIpc(getMainWindow);
    try {
      await expect(
        callHandler("prism:gpu:listProfiles", untrustedEvent()),
      ).rejects.toThrow("Untrusted IPC sender");
      await expect(
        callHandler("prism:gpu:listProfiles", trustedEvent(), { uid: 0 }),
      ).rejects.toThrow("Invalid listProfiles payload");
      await expect(
        callHandler("prism:gpu:saveProfile", untrustedEvent(), {
          gpuId: "GPU-123",
          name: "Quiet",
          revision: "r",
        }),
      ).rejects.toThrow("Untrusted IPC sender");
      for (const payload of [
        { gpuId: "GPU-123", name: " ", revision: "r" },
        { gpuId: 0, name: "Quiet", revision: "r" },
        { gpuId: " ", name: "Quiet", revision: "r" },
        { name: "Quiet", revision: "r" },
        { gpuId: "GPU-123", name: "Quiet" },
        { gpuId: "GPU-123", name: "Quiet", revision: 7 },
        { gpuId: "GPU-123", name: "Quiet", revision: "r", appliedDefaults: [] },
        {
          gpuId: "GPU-123",
          name: "Quiet",
          revision: "r",
          settings: { powerLimitWatts: 999 },
        },
      ]) {
        await expect(
          callHandler("prism:gpu:saveProfile", trustedEvent(), payload),
        ).rejects.toThrow("Invalid saveProfile payload");
      }
      await expect(
        callHandler("prism:gpu:saveProfileToCurrent", untrustedEvent(), {
          gpuId: "GPU-123",
          revision: "r",
        }),
      ).rejects.toThrow("Untrusted IPC sender");
      for (const payload of [
        undefined,
        { gpuId: "GPU-123" },
        { gpuId: " ", revision: "r" },
        { gpuId: "GPU-123", revision: null },
        { gpuId: "GPU-123", revision: "r", profileId: "quiet" },
      ]) {
        await expect(
          callHandler(
            "prism:gpu:saveProfileToCurrent",
            trustedEvent(),
            payload,
          ),
        ).rejects.toThrow("Invalid saveProfileToCurrent payload");
      }
      await expect(
        callHandler("prism:gpu:getTuningStates", untrustedEvent()),
      ).rejects.toThrow("Untrusted IPC sender");
      await expect(
        callHandler("prism:gpu:getTuningStates", trustedEvent(), {
          gpuId: "GPU-123",
        }),
      ).rejects.toThrow("Invalid getTuningStates payload");
      expect(mocks.spawn).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });

  it("imports the app's profile file into the daemon once, keeping ids, then renames it", async () => {
    const directory = await mkdtemp(path.join(os.tmpdir(), "prism-profiles-"));
    mocks.getPath.mockReturnValue(directory);
    const file = path.join(directory, "gpu-tuning-profiles.json");
    await writeFile(
      file,
      JSON.stringify({ version: 1, profiles: gpuTuningProfiles }),
    );
    let failImport = true;
    const child = createBackendChild((method: string) => {
      if (method !== "gpu.importProfiles") return gpuTuningProfiles;
      if (failImport) return new Error("Prism's tuning store can't be used.");
      return { imported: gpuTuningProfiles, skipped: [] };
    });
    mocks.spawn.mockReturnValue(child);
    let dispose = registerIpc(getMainWindow);
    try {
      // A failed import still lists the daemon's profiles and keeps the file for another try.
      expect(
        await callHandler("prism:gpu:listProfiles", trustedEvent()),
      ).toEqual(gpuTuningProfiles);
      expect(await readFile(file, "utf8")).toContain('"version":1');

      failImport = false;
      await callHandler("prism:gpu:listProfiles", trustedEvent());
      await callHandler("prism:gpu:listProfiles", trustedEvent());
      expect(sentRequests(child)).toEqual([
        {
          method: "gpu.importProfiles",
          params: { profiles: gpuTuningProfiles },
        },
        { method: "gpu.listProfiles" },
        {
          method: "gpu.importProfiles",
          params: { profiles: gpuTuningProfiles },
        },
        { method: "gpu.listProfiles" },
        { method: "gpu.listProfiles" },
      ]);
      await expect(readFile(file, "utf8")).rejects.toThrow();
      expect(
        JSON.parse(await readFile(`${file}.imported`, "utf8")).profiles,
      ).toEqual(gpuTuningProfiles);

      // A later app start finds nothing left to import.
      dispose();
      child.stdin.write.mockClear();
      dispose = registerIpc(getMainWindow);
      await callHandler("prism:gpu:listProfiles", trustedEvent());
      expect(sentRequests(child)).toEqual([{ method: "gpu.listProfiles" }]);
    } finally {
      dispose();
      await rm(directory, { recursive: true, force: true });
    }
  });

  it("starts the profile import when the daemon session connects", async () => {
    const directory = await mkdtemp(path.join(os.tmpdir(), "prism-profiles-"));
    mocks.getPath.mockReturnValue(directory);
    const file = path.join(directory, "gpu-tuning-profiles.json");
    await writeFile(
      file,
      JSON.stringify({ version: 1, profiles: gpuTuningProfiles }),
    );
    const child = createBackendChild({ imported: [], skipped: [] });
    mocks.spawn.mockReturnValue(child);
    const window = getMainWindow();
    window.webContents.send = vi.fn();
    const dispose = registerIpc(() => window);
    try {
      await callHandler("prism:daemon:subscribe", trustedEvent());
      child.stdout.write(
        `${JSON.stringify({ method: "daemon.statusChanged", params: connectedDaemonStatus })}\n`,
      );
      await vi.waitFor(() =>
        expect(readFile(`${file}.imported`, "utf8")).resolves.toBeTruthy(),
      );
      expect(sentRequests(child)).toEqual([
        {
          method: "gpu.importProfiles",
          params: { profiles: gpuTuningProfiles },
        },
      ]);
    } finally {
      dispose();
      await rm(directory, { recursive: true, force: true });
    }
  });

  it("preserves an unreadable profile file instead of importing or replacing it", async () => {
    const directory = await mkdtemp(path.join(os.tmpdir(), "prism-profiles-"));
    mocks.getPath.mockReturnValue(directory);
    const file = path.join(directory, "gpu-tuning-profiles.json");
    for (const original of [
      "{ not json",
      JSON.stringify({ version: 2, profiles: [{ id: "existing" }] }),
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
    ]) {
      await writeFile(file, original);
      const child = createBackendChild([]);
      mocks.spawn.mockReturnValue(child);
      const dispose = registerIpc(getMainWindow);
      try {
        expect(
          await callHandler("prism:gpu:listProfiles", trustedEvent()),
        ).toEqual([]);
        expect(sentRequests(child)).toEqual([{ method: "gpu.listProfiles" }]);
        expect(await readFile(file, "utf8")).toBe(original);
      } finally {
        dispose();
      }
    }
    await rm(directory, { recursive: true, force: true });
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
    for (const graphicsOffsetMHz of ["100", "reset", 100.5]) {
      await expect(
        callHandler("prism:gpu:setClockOffsets", trustedEvent(), {
          gpuId: "GPU-123",
          graphicsOffsetMHz,
          memoryOffsetMHz: null,
        }),
      ).rejects.toThrow("Invalid setClockOffsets payload");
    }
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
    const child = createBackendChild(tuningOutcomes);
    mocks.spawn.mockReturnValue(child);
    const dispose = registerIpc(getMainWindow);
    try {
      const request = {
        gpuId: "GPU-123",
        graphicsOffsetMHz: "default",
        memoryOffsetMHz: 80,
      };
      expect(
        await callHandler("prism:gpu:setClockOffsets", trustedEvent(), request),
      ).toEqual(tuningOutcomes);
      expect(sentRequests(child)).toEqual([
        { method: "gpu.setClockOffsets", params: request },
      ]);
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

/** The requests the sidecar received, without their ids. */
function sentRequests(child: ReturnType<typeof createBackendChild>) {
  return child.stdin.write.mock.calls.map(([payload]) => {
    const { method, params } = JSON.parse(payload as string);
    return params === undefined ? { method } : { method, params };
  });
}

/** A sidecar answering every request with the result, or with what it returns for the method. */
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
      const request = JSON.parse(payload) as {
        id: number;
        method: string;
        params?: unknown;
      };
      const answer =
        typeof result === "function"
          ? result(request.method, request.params)
          : result;
      const response =
        answer instanceof Error
          ? {
              id: request.id,
              error: { code: "io", message: answer.message },
            }
          : { id: request.id, result: answer };
      queueMicrotask(() => {
        child.stdout.write(`${JSON.stringify(response)}\n`);
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
