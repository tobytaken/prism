import type {
  DriverJob,
  DriverPlan,
  DriverStatusResponse,
  NvidiaGpuInfo,
} from "@/features/drivers/types";

export function createDriverStatus(
  overrides: Partial<DriverStatusResponse> = {},
): DriverStatusResponse {
  return {
    host: {
      distroId: "ubuntu",
      distroVersion: "24.04",
      kernelRelease: "6.8.0-87-generic",
      packageManager: "apt",
      secureBoot: "disabled",
      isFlatpak: false,
      supported: true,
      unsupportedReason: null,
      ...overrides.host,
    },
    nvidia: {
      detected: true,
      driverVersion: null,
      nvmlAvailable: false,
      nouveauLoaded: true,
      runfileInstallDetected: false,
      rpmFusionDetected: false,
      gpus: [createDriverGpu()],
      plans: [createDriverPlan()],
      warnings: [],
      ...overrides.nvidia,
    },
    activeJob: null,
    warnings: [],
    ...overrides,
  };
}

export function createDriverGpu(
  overrides: Partial<NvidiaGpuInfo> = {},
): NvidiaGpuInfo {
  return {
    id: "0000:01:00.0",
    name: "NVIDIA GeForce RTX 5090",
    architecture: "Blackwell",
    busId: "0000:01:00.0",
    ...overrides,
  };
}

export function createDriverPlan(
  overrides: Partial<DriverPlan> = {},
): DriverPlan {
  return {
    id: "ubuntu-install-recommended",
    action: "installRecommended",
    label: "Install recommended NVIDIA driver",
    recommended: true,
    requiresReboot: true,
    manualSteps: [
      "Reboot after installation before expecting the new driver to load.",
    ],
    blockers: [],
    warnings: [],
    ...overrides,
  };
}

export function createDriverJob(overrides: Partial<DriverJob> = {}): DriverJob {
  return {
    id: "driver-1",
    status: "running",
    action: "installRecommended",
    startedAtUnixMs: 1,
    finishedAtUnixMs: null,
    currentStep: "Refreshing APT package metadata",
    logTail: ["Driver job started."],
    error: null,
    requiresReboot: true,
    ...overrides,
  };
}
