import "@testing-library/jest-dom/vitest";

import type { PrismDesktopApi } from "@/types/global";

const prismStub: PrismDesktopApi = {
  gpu: {
    getInventory: async () => ({ gpus: [], warnings: [] }),
    getPerformance: async () => ({ gpus: [], warnings: [] }),
    getDaemonStatus: async () => ({
      reachable: false,
      managedByDaemon: false,
      socketPath: "/run/prism/prismd.sock",
      message: "Prism daemon is unavailable.",
    }),
    applyConfig: async () => ({ success: true, inventory: null, error: null }),
    resetConfig: async () => ({ success: true, inventory: null, error: null }),
    setPowerLimit: async () => undefined,
    resetPowerLimit: async () => undefined,
    setFanSpeed: async () => undefined,
    setFanModeAuto: async () => undefined,
    setClockOffsets: async () => undefined,
    resetClockOffsets: async () => undefined,
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
