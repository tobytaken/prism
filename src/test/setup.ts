import "@testing-library/jest-dom/vitest";

import type { PrismDesktopApi } from "@/types/global";

const prismStub: PrismDesktopApi = {
  gpu: {
    getMigrationState: async () => ({
      profiles: [],
      canAdopt: false,
      notice: null,
    }),
    adoptMigratedProfile: async () => {
      throw new Error("No migrated profile");
    },
    dismissMigrationNotice: async () => ({ success: true, error: null }),
    getStartupState: async () => ({
      behavior: "off",
      profiles: [],
      outcomes: [],
      possiblyStale: [],
    }),
    setStartupBehavior: async () => ({ success: true, error: null }),
    setStartupProfile: async () => ({ success: true, error: null }),
    dismissStartupOutcome: async () => ({ success: true, error: null }),
    listProfiles: async () => [],
    renameProfile: async () => {
      throw new Error("Profile renaming is not configured in this test");
    },
    deleteProfile: async () => {
      throw new Error("Profile deletion is not configured in this test");
    },
    applyProfile: async () => {
      throw new Error("Profile application is not configured in this test");
    },
    saveProfile: async () => {
      throw new Error("Profile saving is not configured in this test");
    },
    saveProfileToCurrent: async () => {
      throw new Error("Profile saving is not configured in this test");
    },
    getTuningStates: async () => [],
    onTuningStateChanged: () => () => undefined,
    applyProfileById: async () => {
      throw new Error("Profile application is not configured in this test");
    },
    correctAndApplyProfile: async () => {
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
    setPowerLimit: async () => ({ success: true, error: null }),
    resetPowerLimit: async () => ({ success: true, error: null }),
    setFanSpeed: async () => ({ success: true, error: null }),
    setFanModeAuto: async () => ({ success: true, error: null }),
    previewFanCurve: async () => ({ problems: [] }),
    // Older tuning workflow fixtures supply individual fan command fakes. Expose their
    // behavior through the shared cooling API too, so those workflows test the same boundary.
    applyCooling: async ({ gpuId, configuration }) => {
      const fans = [];
      if (configuration.arrangement === "perFan") {
        for (const { fanIndex, setting } of configuration.fans) {
          const result =
            setting.mode === "fixed"
              ? await window.prism.gpu.setFanSpeed(
                  gpuId,
                  fanIndex,
                  setting.speedPercent,
                )
              : await window.prism.gpu.setFanModeAuto(gpuId, fanIndex);
          fans.push({
            fanIndex,
            outcome: {
              status: result.success
                ? ("succeeded" as const)
                : ("failed" as const),
              error: result.error,
            },
          });
        }
      }
      return {
        revision: 0,
        outcome: fans.find((fan) => fan.outcome.status === "failed")
          ?.outcome ?? { status: "succeeded", error: null },
        fans,
      };
    },
    setClockOffsets: async () => ({
      graphics: { status: "succeeded", error: null },
      memory: null,
    }),
    resetClockOffsets: async () => ({
      graphics: { status: "succeeded", error: null },
      memory: null,
    }),
  },
  games: {
    getRunning: async () => ({ status: "available", games: [] }),
    onRunningChanged: () => () => undefined,
    getSwitchingStatus: async () => ({
      condition: { status: "off" },
      gpus: [],
    }),
    onSwitchingStatusChanged: () => () => undefined,
    listSteamApps: async () => [],
    get: async () => ({ automaticSwitching: false, games: [] }),
    addSteamGame: async () => {
      throw new Error("Adding games is not configured in this test");
    },
    setProfile: async () => {
      throw new Error("Game associations are not configured in this test");
    },
    remove: async () => {
      throw new Error("Removing games is not configured in this test");
    },
    setAutomaticSwitching: async () => {
      throw new Error("Automatic switching is not configured in this test");
    },
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
