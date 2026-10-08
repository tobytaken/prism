import runningGames from "@/test/contracts/running-games.json";
import switchingStatuses from "@/test/contracts/automatic-switching-status.json";
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
import gamesState from "@/test/contracts/games-state.json";
import fanCurvePreview from "@/test/contracts/fan-curve-preview.json";
import coolingApplication from "@/test/contracts/cooling-application.json";
import { connectedDaemonStatus } from "@/test/gpu-fixtures";
import { registerIpc } from "./ipc";

const mocks = vi.hoisted(() => ({
  handlers: new Map<string, (...args: unknown[]) => unknown>(),
  handle: vi.fn(),
  openExternal: vi.fn(),
  showOpenDialog: vi.fn(),
  spawn: vi.fn(),
  getPath: vi.fn(),
}));

vi.mock("electron", () => ({
  app: {
    isPackaged: false,
    getPath: mocks.getPath,
  },
  BrowserWindow: vi.fn(),
  dialog: { showOpenDialog: mocks.showOpenDialog },
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

  it("reads the caller's running games and rejects extra arguments or an untrusted sender", async () => {
    const child = createBackendChild(runningGames[0]);
    mocks.spawn.mockReturnValue(child);
    const dispose = registerIpc(getMainWindow);
    try {
      expect(
        await callHandler("prism:games:getRunning", trustedEvent()),
      ).toEqual(runningGames[0]);
      expect(sentRequests(child)).toEqual([{ method: "games.getRunning" }]);
      await expect(
        callHandler("prism:games:getRunning", untrustedEvent()),
      ).rejects.toThrow("Untrusted IPC sender");
      await expect(
        callHandler("prism:games:getRunning", trustedEvent(), { uid: 0 }),
      ).rejects.toThrow("Invalid running games payload");
    } finally {
      dispose();
    }
  });

  it("forwards only validated running game snapshots to subscribed renderers", async () => {
    const child = createBackendChild();
    mocks.spawn.mockReturnValue(child);
    const send = vi.fn();
    const window = getMainWindow();
    window.webContents.send = send;
    const dispose = registerIpc(() => window);
    try {
      await callHandler("prism:daemon:subscribe", trustedEvent());
      for (const params of runningGames)
        child.stdout.write(
          `${JSON.stringify({ method: "games.runningChanged", params })}\n`,
        );
      expect(send.mock.calls).toEqual(
        runningGames.map((state) => ["prism:games:runningChanged", state]),
      );
      send.mockClear();
      for (const params of [
        null,
        { status: "available" },
        {
          status: "available",
          games: [
            { game: { kind: "steam", appId: 0 }, pid: 1, startTimeTicks: 1 },
          ],
        },
        { ...runningGames[0], uid: 0 },
      ])
        child.stdout.write(
          `${JSON.stringify({ method: "games.runningChanged", params })}\n`,
        );
      expect(send).not.toHaveBeenCalled();
      await callHandler("prism:daemon:unsubscribe", trustedEvent());
      child.stdout.write(
        `${JSON.stringify({ method: "games.runningChanged", params: runningGames[0] })}\n`,
      );
      expect(send).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });

  it("reports edit holds only from a trusted renderer with a GPU identity set", async () => {
    const child = createBackendChild({ success: true, error: null });
    mocks.spawn.mockReturnValue(child);
    const dispose = registerIpc(getMainWindow);
    try {
      for (const gpuIds of [["GPU-A", "GPU-B"], []]) {
        await callHandler("prism:games:setEditHolds", trustedEvent(), {
          gpuIds,
        });
      }
      expect(sentRequests(child)).toEqual([
        {
          method: "games.setEditHolds",
          params: { gpuIds: ["GPU-A", "GPU-B"] },
        },
        { method: "games.setEditHolds", params: { gpuIds: [] } },
      ]);
      await expect(
        callHandler("prism:games:setEditHolds", untrustedEvent(), {
          gpuIds: [],
        }),
      ).rejects.toThrow("Untrusted IPC sender");
      for (const payload of [
        null,
        {},
        { gpuIds: "GPU-A" },
        { gpuIds: [1] },
        { gpuIds: [""] },
        { gpuIds: [], uid: 0 },
      ]) {
        await expect(
          callHandler("prism:games:setEditHolds", trustedEvent(), payload),
        ).rejects.toThrow("Invalid edit holds payload");
      }
    } finally {
      dispose();
    }
  });

  it("reads the caller's automatic switching status and rejects extra arguments or an untrusted sender", async () => {
    const child = createBackendChild(switchingStatuses[0]);
    mocks.spawn.mockReturnValue(child);
    const dispose = registerIpc(getMainWindow);
    try {
      expect(
        await callHandler("prism:games:getSwitchingStatus", trustedEvent()),
      ).toEqual(switchingStatuses[0]);
      expect(sentRequests(child)).toEqual([
        { method: "games.getSwitchingStatus" },
      ]);
      await expect(
        callHandler("prism:games:getSwitchingStatus", untrustedEvent()),
      ).rejects.toThrow("Untrusted IPC sender");
      await expect(
        callHandler("prism:games:getSwitchingStatus", trustedEvent(), {
          uid: 0,
        }),
      ).rejects.toThrow("Invalid switching status payload");
    } finally {
      dispose();
    }
  });

  it("forwards only validated automatic switching statuses to subscribed renderers", async () => {
    const child = createBackendChild();
    mocks.spawn.mockReturnValue(child);
    const send = vi.fn();
    const window = getMainWindow();
    window.webContents.send = send;
    const dispose = registerIpc(() => window);
    const push = (params: unknown) =>
      child.stdout.write(
        `${JSON.stringify({ method: "games.switchingStatusChanged", params })}\n`,
      );
    try {
      await callHandler("prism:daemon:subscribe", trustedEvent());
      for (const params of switchingStatuses) push(params);
      expect(send.mock.calls).toEqual(
        switchingStatuses.map((status) => [
          "prism:games:switchingStatusChanged",
          status,
        ]),
      );
      send.mockClear();
      for (const params of [
        null,
        { condition: { status: "ready" } },
        { condition: { status: "paused" }, gpus: [] },
        { condition: { status: "blocked" }, gpus: [] },
        {
          condition: { status: "off" },
          gpus: [
            { gpuId: "GPU-123", state: { status: "anotherUser", uid: 0 } },
          ],
        },
        { ...switchingStatuses[0], uid: 0 },
      ])
        push(params);
      expect(send).not.toHaveBeenCalled();
      await callHandler("prism:daemon:unsubscribe", trustedEvent());
      push(switchingStatuses[0]);
      expect(send).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });

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
      await callHandler("prism:games:setEditHolds", trustedEvent(), {
        gpuIds: ["GPU-A"],
      });
      first.emit("exit", 1, null);
      expect(send).toHaveBeenLastCalledWith(
        "prism:daemon:statusChanged",
        expect.objectContaining({ reachable: false, tuningStore: null }),
      );
      await vi.advanceTimersByTimeAsync(250);
      expect(mocks.spawn).toHaveBeenCalledTimes(2);
      expect(mocks.spawn.mock.calls[1][2].env.PRISM_INITIAL_EDIT_HOLDS).toBe(
        JSON.stringify({ gpuIds: ["GPU-A"] }),
      );
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
          unexpected: true,
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

  it("keeps a queued GPU mutation pending through the startup wait", async () => {
    vi.useFakeTimers();
    const child = createBackendChild({ success: true, error: null });
    child.stdin.write.mockImplementation(() => true);
    mocks.spawn.mockReturnValue(child);
    const dispose = registerIpc(getMainWindow);
    try {
      const pending = callHandler("prism:gpu:setPowerLimit", trustedEvent(), {
        gpuId: "GPU-123",
        limitWatts: 350,
      });
      const observed = pending.then(
        (value) => ({ value }),
        (error: Error) => ({ error: error.message }),
      );
      await vi.advanceTimersByTimeAsync(31_000);
      const request = JSON.parse(child.stdin.write.mock.calls[0][0]) as {
        id: number;
      };
      child.stdout.write(
        `${JSON.stringify({ id: request.id, result: { success: true, error: null } })}\n`,
      );
      expect(await observed).toEqual({ value: { success: true, error: null } });
    } finally {
      dispose();
      vi.useRealTimers();
    }
  });

  it("forwards startup requests and rejects client ownership and invalid modes", async () => {
    const child = createBackendChild({ success: true, error: null });
    mocks.spawn.mockReturnValue(child);
    const dispose = registerIpc(getMainWindow);
    try {
      for (const [method, payload] of [
        ["setStartupBehavior", { behavior: "startupProfiles" }],
        ["setStartupBehavior", { behavior: "restoreLastApplied" }],
        ["setStartupProfile", { gpuId: "GPU-123", profileId: "quiet" }],
        ["setStartupProfile", { gpuId: "GPU-123", profileId: null }],
        ["dismissStartupOutcome", { gpuId: "GPU-123" }],
      ] as const) {
        expect(
          await callHandler(`prism:gpu:${method}`, trustedEvent(), payload),
        ).toEqual({ success: true, error: null });
        await expect(
          callHandler(`prism:gpu:${method}`, untrustedEvent(), payload),
        ).rejects.toThrow("Untrusted IPC sender");
        await expect(
          callHandler(`prism:gpu:${method}`, trustedEvent(), {
            ...payload,
            uid: 0,
          }),
        ).rejects.toThrow(`Invalid ${method} payload`);
      }
      await callHandler("prism:gpu:getStartupState", trustedEvent());
      expect(sentRequests(child)).toEqual([
        {
          method: "gpu.setStartupBehavior",
          params: { behavior: "startupProfiles" },
        },
        {
          method: "gpu.setStartupBehavior",
          params: { behavior: "restoreLastApplied" },
        },
        {
          method: "gpu.setStartupProfile",
          params: { gpuId: "GPU-123", profileId: "quiet" },
        },
        {
          method: "gpu.setStartupProfile",
          params: { gpuId: "GPU-123", profileId: null },
        },
        { method: "gpu.dismissStartupOutcome", params: { gpuId: "GPU-123" } },
        { method: "gpu.getStartupState" },
      ]);
      await expect(
        callHandler("prism:gpu:getStartupState", trustedEvent(), {}),
      ).rejects.toThrow("Invalid getStartupState payload");
      await expect(
        callHandler("prism:gpu:setStartupBehavior", trustedEvent(), {
          behavior: "always",
        }),
      ).rejects.toThrow("Invalid setStartupBehavior payload");
      await expect(
        callHandler("prism:gpu:setStartupProfile", trustedEvent(), {
          gpuId: "GPU-123",
        }),
      ).rejects.toThrow("Invalid setStartupProfile payload");
    } finally {
      dispose();
    }
  });

  it("forwards game requests to the daemon and preserves its games and Steam apps", async () => {
    const apps = [{ appId: 570, name: "Dota 2" }];
    const child = createBackendChild((method: string) =>
      method === "games.listSteamApps" ? apps : gamesState,
    );
    mocks.spawn.mockReturnValue(child);
    const dispose = registerIpc(getMainWindow);
    const dota = { kind: "steam", appId: 570 };
    try {
      expect(
        await callHandler("prism:games:listSteamApps", trustedEvent()),
      ).toEqual(apps);
      for (const [method, payload] of [
        ["setGpuSwitchingPaused", { gpuId: "GPU-123", paused: true }],
        ["get", undefined],
        ["addSteamGame", { appId: 570 }],
        ["setProfile", { game: dota, gpuId: "GPU-123", profileId: "quiet" }],
        ["setProfile", { game: dota, gpuId: "GPU-123", profileId: null }],
        ["remove", { game: dota }],
        ["setAutomaticSwitching", { enabled: true }],
      ] as const) {
        const args = payload === undefined ? [] : [payload];
        expect(
          await callHandler(`prism:games:${method}`, trustedEvent(), ...args),
        ).toEqual(gamesState);
      }
      expect(sentRequests(child)).toEqual([
        { method: "games.listSteamApps" },
        {
          method: "games.setGpuSwitchingPaused",
          params: { gpuId: "GPU-123", paused: true },
        },
        { method: "games.get" },
        { method: "games.addSteamGame", params: { appId: 570 } },
        {
          method: "games.setProfile",
          params: { game: dota, gpuId: "GPU-123", profileId: "quiet" },
        },
        {
          method: "games.setProfile",
          params: { game: dota, gpuId: "GPU-123", profileId: null },
        },
        { method: "games.remove", params: { game: dota } },
        { method: "games.setAutomaticSwitching", params: { enabled: true } },
      ]);
    } finally {
      dispose();
    }
  });

  it("lists and adds manual programs and browses through the app-side portal recovery", async () => {
    const programs = [
      {
        pid: 123,
        name: "game.exe",
        path: "/games/game.exe",
        launcher: "Lutris",
        environment: [],
        helper: false,
        recognitionWarning: null,
      },
    ];
    const child = createBackendChild((method: string) =>
      method === "games.listRunningPrograms" ? programs : gamesState,
    );
    mocks.spawn.mockReturnValue(child);
    const dispose = registerIpc(getMainWindow);
    const game = { kind: "program", path: "/games/game.exe" };
    try {
      expect(
        await callHandler("prism:games:listRunningPrograms", trustedEvent()),
      ).toEqual(programs);
      await callHandler("prism:games:addProgram", trustedEvent(), {
        path: game.path,
      });
      await callHandler("prism:games:setProfile", trustedEvent(), {
        game,
        gpuId: "GPU-123",
        profileId: "quiet",
      });
      await callHandler("prism:games:remove", trustedEvent(), { game });
      mocks.showOpenDialog.mockResolvedValueOnce({
        canceled: true,
        filePaths: [],
      });
      expect(
        await callHandler("prism:games:browseProgram", trustedEvent()),
      ).toBeNull();
      mocks.showOpenDialog.mockResolvedValueOnce({
        canceled: false,
        filePaths: ["/run/flatpak/doc/abc/game.exe"],
      });
      expect(
        await callHandler("prism:games:browseProgram", trustedEvent()),
      ).toEqual(gamesState);
      expect(sentRequests(child)).toEqual([
        { method: "games.listRunningPrograms" },
        { method: "games.addProgram", params: { path: game.path } },
        {
          method: "games.setProfile",
          params: { game, gpuId: "GPU-123", profileId: "quiet" },
        },
        { method: "games.remove", params: { game } },
        {
          method: "games.addProgramFile",
          params: { path: "/run/flatpak/doc/abc/game.exe" },
        },
      ]);
      for (const method of [
        "listRunningPrograms",
        "addProgram",
        "browseProgram",
      ]) {
        await expect(
          callHandler(`prism:games:${method}`, untrustedEvent()),
        ).rejects.toThrow("Untrusted IPC sender");
        await expect(
          callHandler(`prism:games:${method}`, trustedEvent(), { uid: 0 }),
        ).rejects.toThrow("Invalid");
      }
      for (const path of [
        "relative",
        "/games/../private",
        "/games/bad\0path",
      ]) {
        await expect(
          callHandler("prism:games:addProgram", trustedEvent(), { path }),
        ).rejects.toThrow("Invalid");
      }
    } finally {
      dispose();
    }
  });

  it("rejects untrusted or malformed game requests before reaching the daemon", async () => {
    const dispose = registerIpc(getMainWindow);
    const dota = { kind: "steam", appId: 570 };
    try {
      for (const [method, payload] of [
        ["setGpuSwitchingPaused", { gpuId: "GPU-123", paused: false }],
        ["listSteamApps", undefined],
        ["get", undefined],
        ["addSteamGame", { appId: 570 }],
        ["setProfile", { game: dota, gpuId: "GPU-123", profileId: null }],
        ["remove", { game: dota }],
        ["setAutomaticSwitching", { enabled: false }],
      ] as const) {
        await expect(
          callHandler(`prism:games:${method}`, untrustedEvent(), payload),
        ).rejects.toThrow("Untrusted IPC sender");
      }
      const invalid: [string, unknown[]][] = [
        ["setGpuSwitchingPaused", [undefined]],
        ["setGpuSwitchingPaused", [{ gpuId: "GPU-123", paused: "yes" }]],
        ["setGpuSwitchingPaused", [{ gpuId: " ", paused: true }]],
        ["setGpuSwitchingPaused", [{ gpuId: "GPU-123" }]],
        ["setGpuSwitchingPaused", [{ gpuId: "GPU-123", paused: true, uid: 0 }]],
        ["listSteamApps", [{}]],
        ["get", [{ uid: 0 }]],
        ["addSteamGame", [undefined]],
        ["addSteamGame", [{ appId: "570" }]],
        ["addSteamGame", [{ appId: 5.5 }]],
        ["addSteamGame", [{ appId: -1 }]],
        ["addSteamGame", [{ appId: 570, name: "Dota 2" }]],
        ["setProfile", [{ game: dota, gpuId: "GPU-123" }]],
        ["setProfile", [{ game: dota, gpuId: " ", profileId: null }]],
        ["setProfile", [{ game: dota, gpuId: "GPU-123", profileId: " " }]],
        [
          "setProfile",
          [{ game: dota, gpuId: "GPU-123", profileId: null, uid: 0 }],
        ],
        [
          "setProfile",
          [
            {
              game: { kind: "program", appId: 570 },
              gpuId: "GPU-123",
              profileId: null,
            },
          ],
        ],
        ["remove", [{ game: { ...dota, uid: 0 } }]],
        ["remove", [{ game: { kind: "steam" } }]],
        ["remove", [{}]],
        ["setAutomaticSwitching", [{ enabled: "yes" }]],
        ["setAutomaticSwitching", [{ enabled: true, uid: 0 }]],
      ];
      for (const [method, args] of invalid) {
        await expect(
          callHandler(`prism:games:${method}`, trustedEvent(), ...args),
        ).rejects.toThrow(`Invalid ${method} payload`);
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

  it("forwards a fan curve preview to the backend and returns the daemon's problems", async () => {
    const child = createBackendChild((method: string, params: unknown) =>
      method === "gpu.previewFanCurve" ? fanCurvePreview.preview : params,
    );
    mocks.spawn.mockReturnValue(child);
    const dispose = registerIpc(getMainWindow);
    try {
      expect(
        await callHandler(
          "prism:gpu:previewFanCurve",
          trustedEvent(),
          fanCurvePreview.request,
        ),
      ).toEqual(fanCurvePreview.preview);
      const sent = JSON.parse(child.stdin.write.mock.lastCall![0] as string);
      expect(sent).toMatchObject({
        method: "gpu.previewFanCurve",
        params: fanCurvePreview.request,
      });
    } finally {
      dispose();
    }
  });

  it("rejects malformed or untrusted fan curve previews before the backend", async () => {
    const child = createBackendChild();
    mocks.spawn.mockReturnValue(child);
    const dispose = registerIpc(getMainWindow);
    const point = { temperatureC: 45, speedPercent: 20 };
    try {
      for (const payload of [
        null,
        { gpuId: "GPU-123" },
        { gpuId: 123, points: [point] },
        { gpuId: "GPU-123", points: point },
        { gpuId: "GPU-123", points: [{ ...point, temperatureC: 45.5 }] },
        { gpuId: "GPU-123", points: [{ ...point, speedPercent: "20" }] },
        { gpuId: "GPU-123", points: [{ temperatureC: 45 }] },
        { gpuId: "GPU-123", points: [{ ...point, mode: "fixed" }] },
        { gpuId: "GPU-123", points: [point], apply: true },
      ])
        await expect(
          callHandler("prism:gpu:previewFanCurve", trustedEvent(), payload),
        ).rejects.toThrow("Invalid previewFanCurve payload");
      await expect(
        callHandler("prism:gpu:previewFanCurve", untrustedEvent(), {
          gpuId: "GPU-123",
          points: [point],
        }),
      ).rejects.toThrow("Untrusted IPC sender");
      expect(child.stdin.write).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });

  it("forwards a cooling application to the backend and returns its per-fan outcomes", async () => {
    const child = createBackendChild((method: string, params: unknown) =>
      method === "gpu.applyCooling" ? coolingApplication.application : params,
    );
    mocks.spawn.mockReturnValue(child);
    const dispose = registerIpc(getMainWindow);
    try {
      expect(
        await callHandler(
          "prism:gpu:applyCooling",
          trustedEvent(),
          coolingApplication.request,
        ),
      ).toEqual(coolingApplication.application);
      const sent = JSON.parse(child.stdin.write.mock.lastCall![0] as string);
      expect(sent).toMatchObject({
        method: "gpu.applyCooling",
        params: coolingApplication.request,
      });
      const perFan = {
        gpuId: "GPU-123",
        configuration: coolingApplication.controls[1].applied,
      };
      await callHandler("prism:gpu:applyCooling", trustedEvent(), perFan);
      expect(
        JSON.parse(child.stdin.write.mock.lastCall![0] as string).params,
      ).toEqual(perFan);
    } finally {
      dispose();
    }
  });

  it("rejects malformed or untrusted cooling applications before the backend", async () => {
    const child = createBackendChild();
    mocks.spawn.mockReturnValue(child);
    const dispose = registerIpc(getMainWindow);
    const point = { temperatureC: 45, speedPercent: 20 };
    const shared = (setting: unknown) => ({
      gpuId: "GPU-123",
      configuration: { arrangement: "shared", setting },
    });
    try {
      for (const payload of [
        null,
        { gpuId: "GPU-123" },
        {
          gpuId: 123,
          configuration: {
            arrangement: "shared",
            setting: { mode: "automatic" },
          },
        },
        shared({ mode: "auto" }),
        shared({ mode: "automatic", speedPercent: 50 }),
        shared({ mode: "fixed", speedPercent: 50.5 }),
        shared({ mode: "curve", points: point }),
        shared({ mode: "curve", points: [{ ...point, temperatureC: "45" }] }),
        shared({ mode: "curve", points: [{ ...point, rpm: 900 }] }),
        { ...shared({ mode: "automatic" }), apply: true },
        ...[-1, 0.5, "1", null].map((expectedRevision) => ({
          ...shared({ mode: "automatic" }),
          expectedRevision,
        })),
        {
          gpuId: "GPU-123",
          configuration: { arrangement: "shared", fans: [] },
        },
        {
          gpuId: "GPU-123",
          configuration: {
            arrangement: "perFan",
            fans: [{ fanIndex: -1, setting: { mode: "automatic" } }],
          },
        },
      ])
        await expect(
          callHandler("prism:gpu:applyCooling", trustedEvent(), payload),
        ).rejects.toThrow("Invalid applyCooling payload");
      await expect(
        callHandler(
          "prism:gpu:applyCooling",
          untrustedEvent(),
          coolingApplication.request,
        ),
      ).rejects.toThrow("Untrusted IPC sender");
      expect(child.stdin.write).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
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
