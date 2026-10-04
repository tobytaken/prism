import "@testing-library/jest-dom/vitest";

import type { PrismDesktopApi } from "@/types/global";

const prismStub: PrismDesktopApi = {
  gpu: {
    listProfiles: async () => [],
    applyProfile: async () => {
      throw new Error("Profile application is not configured in this test");
    },
    saveProfile: async () => {
      throw new Error("Profile saving is not configured in this test");
    },
    correctProfile: async () => {
      throw new Error("Profile correction is not configured in this test");
    },
    getInventory: async () => ({ gpus: [], warnings: [] }),
    getPerformance: async () => ({ gpus: [], warnings: [] }),
    getDaemonStatus: async () => ({
      reachable: false,
      managedByDaemon: false,
      socketPath: "/run/prism/prismd.sock",
      message: "Prism daemon is unavailable.",
      tuningStore: null,
    }),
    onDaemonStatusChanged: () => () => undefined,
    applyConfig: async () => ({ success: true, inventory: null, error: null }),
    resetConfig: async () => ({ success: true, inventory: null, error: null }),
    setPowerLimit: async () => ({ success: true, error: null }),
    resetPowerLimit: async () => ({ success: true, error: null }),
    setFanSpeed: async () => ({ success: true, error: null }),
    setFanModeAuto: async () => ({ success: true, error: null }),
    setClockOffsets: async () => ({
      graphics: { status: "succeeded", error: null },
      memory: null,
    }),
    resetClockOffsets: async () => ({
      graphics: { status: "succeeded", error: null },
      memory: null,
    }),
  },
  drivers: {
    getStatus: async () => ({
      host: {
        distroId: "ubuntu",
        distroVersion: "24.04",
        kernelRelease: "6.8.0",
        packageManager: "apt",
        secureBoot: "disabled",
        isFlatpak: false,
        supported: false,
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
    }),
    startAction: async (action) => ({
      id: "driver-test",
      status: "queued",
      action,
      startedAtUnixMs: 0,
      finishedAtUnixMs: null,
      currentStep: "Queued",
      logTail: [],
      error: null,
      requiresReboot: true,
    }),
    getJob: async (jobId) => ({
      id: jobId,
      status: "queued",
      action: "installRecommended",
      startedAtUnixMs: 0,
      finishedAtUnixMs: null,
      currentStep: "Queued",
      logTail: [],
      error: null,
      requiresReboot: true,
    }),
    cancelJob: async (jobId) => ({
      id: jobId,
      status: "cancelled",
      action: "installRecommended",
      startedAtUnixMs: 0,
      finishedAtUnixMs: 1,
      currentStep: null,
      logTail: [],
      error: null,
      requiresReboot: true,
    }),
    requestReboot: async () => undefined,
  },
  window: {
    minimize: async () => undefined,
    maximize: async () => undefined,
    unmaximize: async () => undefined,
    close: async () => undefined,
    isMaximized: async () => false,
    onResized: () => () => undefined,
  },
  shell: {
    openExternal: async () => undefined,
  },
};

Object.defineProperty(window, "prism", {
  writable: true,
  configurable: true,
  value: prismStub,
});
