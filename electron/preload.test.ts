import startupState from "@/test/contracts/startup-state.json";
import { beforeEach, describe, expect, it, vi } from "vitest";

import tuningOutcomes from "@/test/contracts/tuning-outcomes.json";
import profileOutcomes from "@/test/contracts/profile-outcomes.json";
import profileCorrections from "@/test/contracts/profile-corrections.json";
import tuningObservations from "@/test/contracts/tuning-observations.json";
import unhealthyStoreStatus from "@/test/contracts/daemon-status-unhealthy-store.json";

import correctedProfileApplication from "@/test/contracts/corrected-profile-application.json";
import gpuTuningProfiles from "@/test/contracts/gpu-tuning-profiles.json";
import gpuTuningStates from "@/test/contracts/gpu-tuning-states.json";
import type { CorrectGpuProfileRequest } from "@/features/gpus/types";
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

  it("subscribes to daemon status events and removes the exact listener on unsubscribe", async () => {
    const prism = await loadPrismApi();
    const callback = vi.fn();
    const unsubscribe = prism.gpu.onDaemonStatusChanged(callback);
    expect(mocks.invoke).toHaveBeenCalledWith("prism:daemon:subscribe");
    const [channel, handler] = mocks.on.mock.calls[0];
    expect(channel).toBe("prism:daemon:statusChanged");
    handler({}, unhealthyStoreStatus);
    expect(callback).toHaveBeenCalledWith(unhealthyStoreStatus);
    handler(
      {},
      { ...unhealthyStoreStatus, tuningStore: { status: "unhealthy" } },
    );
    expect(callback).toHaveBeenCalledTimes(1);
    unsubscribe();
    expect(mocks.removeListener).toHaveBeenCalledWith(channel, handler);
    expect(mocks.invoke).toHaveBeenLastCalledWith("prism:daemon:unsubscribe");
  });

  it("keeps other daemon listeners subscribed and makes cleanup idempotent", async () => {
    const prism = await loadPrismApi();
    const first = vi.fn();
    const second = vi.fn();
    const unsubscribeFirst = prism.gpu.onDaemonStatusChanged(first);
    const unsubscribeSecond = prism.gpu.onDaemonStatusChanged(second);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    unsubscribeFirst();
    unsubscribeFirst();
    expect(mocks.removeListener).toHaveBeenCalledTimes(1);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    const handler = mocks.on.mock.calls[1][1];
    handler({}, unhealthyStoreStatus);
    expect(second).toHaveBeenCalledWith(unhealthyStoreStatus);
    expect(first).not.toHaveBeenCalled();
    unsubscribeSecond();
    expect(mocks.invoke).toHaveBeenLastCalledWith("prism:daemon:unsubscribe");
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

  it("transports proposed range corrections and approved profile corrections", async () => {
    const prism = await loadPrismApi();
    mocks.invoke.mockResolvedValue(profileCorrections);
    expect(await prism.gpu.applyProfileById({ profileId: "quiet" })).toEqual(
      profileCorrections,
    );
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "prism:gpu:applyProfileById",
      { profileId: "quiet" },
    );
    mocks.invoke.mockResolvedValue(correctedProfileApplication);
    const request: CorrectGpuProfileRequest = {
      profileId: "quiet",
      corrections: [
        {
          setting: "powerLimitWatts",
          savedValue: 300,
          proposedValue: 280,
          bound: "maximum",
        },
      ],
    };
    expect(await prism.gpu.correctAndApplyProfile(request)).toEqual(
      correctedProfileApplication,
    );
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "prism:gpu:correctAndApplyProfile",
      request,
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
    await prism.gpu.setClockOffsets("GPU-123", "default", -50);
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
      [
        "prism:gpu:setClockOffsets",
        {
          gpuId: "GPU-123",
          graphicsOffsetMHz: "default",
          memoryOffsetMHz: -50,
        },
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

  it("exposes profile rename and delete by id and preserves their responses and errors", async () => {
    const prism = await loadPrismApi();
    const renamed = { ...gpuTuningProfiles[0], name: "Silent" };
    const rename = { profileId: renamed.id, name: "Silent" };
    mocks.invoke.mockResolvedValueOnce(renamed);
    expect(await prism.gpu.renameProfile(rename)).toEqual(renamed);
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "prism:gpu:renameProfile",
      rename,
    );
    const remove = { profileId: renamed.id };
    const deleted = { success: true, error: null };
    mocks.invoke.mockResolvedValueOnce(deleted);
    expect(await prism.gpu.deleteProfile(remove)).toEqual(deleted);
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "prism:gpu:deleteProfile",
      remove,
    );
    mocks.invoke.mockRejectedValue(
      new Error("The GPU tuning profile no longer exists."),
    );
    await expect(prism.gpu.renameProfile(rename)).rejects.toThrow(
      "no longer exists",
    );
    await expect(prism.gpu.deleteProfile(remove)).rejects.toThrow(
      "no longer exists",
    );
  });

  it("exposes profile save/list commands and preserves omitted, numeric, and default setting meanings", async () => {
    const prism = await loadPrismApi();
    const [profile] = gpuTuningProfiles;
    mocks.invoke.mockResolvedValue(profile);
    const save = { gpuId: "GPU-123", name: "Quiet", revision: "r1" };
    expect(await prism.gpu.saveProfile(save)).toEqual(profile);
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "prism:gpu:saveProfile",
      save,
    );
    const saveToCurrent = { gpuId: "GPU-123", revision: "r1" };
    expect(await prism.gpu.saveProfileToCurrent(saveToCurrent)).toEqual(
      profile,
    );
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "prism:gpu:saveProfileToCurrent",
      saveToCurrent,
    );
    mocks.invoke.mockResolvedValue(gpuTuningProfiles);
    expect(await prism.gpu.listProfiles()).toEqual(gpuTuningProfiles);
    expect(mocks.invoke).toHaveBeenLastCalledWith("prism:gpu:listProfiles");
    mocks.invoke.mockResolvedValue(gpuTuningStates);
    expect(await prism.gpu.getTuningStates()).toEqual(gpuTuningStates);
    expect(mocks.invoke).toHaveBeenLastCalledWith("prism:gpu:getTuningStates");
  });

  it("forwards host-wide startup choices, explicit clearing, and outcome dismissals", async () => {
    const prism = await loadPrismApi();
    mocks.invoke.mockResolvedValueOnce(startupState);
    expect(await prism.gpu.getStartupState()).toEqual(startupState);
    expect(mocks.invoke).toHaveBeenLastCalledWith("prism:gpu:getStartupState");
    mocks.invoke.mockResolvedValue({ success: true, error: null });
    await prism.gpu.setStartupBehavior({ behavior: "startupProfiles" });
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "prism:gpu:setStartupBehavior",
      { behavior: "startupProfiles" },
    );
    await prism.gpu.setStartupProfile({ gpuId: "GPU-123", profileId: null });
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "prism:gpu:setStartupProfile",
      { gpuId: "GPU-123", profileId: null },
    );
    await prism.gpu.dismissStartupOutcome({ gpuId: "GPU-123" });
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "prism:gpu:dismissStartupOutcome",
      { gpuId: "GPU-123" },
    );
  });

  it("subscribes to tuning state changes alongside daemon status, keeping the daemon subscription until both leave", async () => {
    const prism = await loadPrismApi();
    const states = vi.fn();
    const unsubscribeStates = prism.gpu.onTuningStateChanged(states);
    expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith(
      "prism:daemon:subscribe",
    );
    const [channel, handler] = mocks.on.mock.calls[0];
    expect(channel).toBe("prism:gpu:tuningStateChanged");
    handler({}, gpuTuningStates[0]);
    handler({}, { ...gpuTuningStates[0], revision: 7 });
    expect(states).toHaveBeenCalledExactlyOnceWith(gpuTuningStates[0]);
    const unsubscribeStatus = prism.gpu.onDaemonStatusChanged(vi.fn());
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    unsubscribeStates();
    unsubscribeStates();
    expect(mocks.removeListener).toHaveBeenCalledExactlyOnceWith(
      channel,
      handler,
    );
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    unsubscribeStatus();
    expect(mocks.invoke).toHaveBeenLastCalledWith("prism:daemon:unsubscribe");
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

  it("preserves the tuning store's health in the daemon status across preload", async () => {
    const prism = await loadPrismApi();
    mocks.invoke.mockResolvedValue(unhealthyStoreStatus);
    expect(await prism.gpu.getDaemonStatus()).toEqual(unhealthyStoreStatus);
    expect(mocks.invoke).toHaveBeenCalledWith("prism:gpu:getDaemonStatus");
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
