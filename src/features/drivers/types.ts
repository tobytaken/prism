export type DriverVendor = "nvidia";
export type DriverAction = "installRecommended" | "updateInstalled";
export type DriverJobStatus =
  | "queued"
  | "running"
  | "succeeded"
  | "failed"
  | "canceled"
  | "rebootRequired"
  | "manualStepRequired";

export type PackageManager = "apt" | "dnf" | "unsupported";
export type SecureBootState = "enabled" | "disabled" | "unknown";

export interface DriverStatusResponse {
  host: DriverHostInfo;
  nvidia: NvidiaDriverStatus;
  activeJob: DriverJob | null;
  warnings: string[];
}

export interface DriverHostInfo {
  distroId: string;
  distroVersion: string;
  kernelRelease: string;
  packageManager: PackageManager;
  secureBoot: SecureBootState;
  isFlatpak: boolean;
  supported: boolean;
  unsupportedReason: string | null;
}

export interface NvidiaDriverStatus {
  detected: boolean;
  driverVersion: string | null;
  nvmlAvailable: boolean;
  nouveauLoaded: boolean;
  runfileInstallDetected: boolean;
  rpmFusionDetected: boolean;
  gpus: NvidiaGpuInfo[];
  plans: DriverPlan[];
  warnings: string[];
}

export interface NvidiaGpuInfo {
  id: string;
  name: string;
  architecture: string | null;
  busId: string | null;
}

export interface DriverPlan {
  id: string;
  action: DriverAction;
  label: string;
  recommended: boolean;
  requiresReboot: boolean;
  manualSteps: string[];
  blockers: string[];
  warnings: string[];
}

export interface DriverJob {
  id: string;
  status: DriverJobStatus;
  action: DriverAction;
  startedAtUnixMs: number;
  finishedAtUnixMs: number | null;
  currentStep: string | null;
  logTail: string[];
  error: string | null;
  requiresReboot: boolean;
}
