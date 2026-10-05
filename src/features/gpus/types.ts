export type GpuVendor = "nvidia" | "amd" | "intel";
export type ComputeApi = "cuda" | "rocm";
export type GpuCommandErrorCode =
  | "daemonUnavailable"
  | "noPermission"
  | "invalidArgument"
  | "notSupported"
  | "gpuLost"
  | "nvmlUnavailable"
  | "io"
  | "internal"
  | "notFound";

export interface GpuInventoryResponse {
  gpus: GpuDevice[];
  warnings: string[];
}

export interface GpuPerformanceResponse {
  gpus: GpuPerformanceDevice[];
  warnings: string[];
}

export interface GpuDevice {
  id: string;
  vendor: GpuVendor;
  name: string;
  architecture: string | null;
  driverVersion: string | null;
  firmwareVersion: string | null;
  busId: string | null;
  memory: {
    totalMiB: number | null;
  };
  compute: {
    api: ComputeApi | null;
    apiVersion: string | null;
    capability: string | null;
    coreCount: number | null;
  };
  pcie: {
    genMax: number | null;
    widthMax: number | null;
  };
  power: {
    enforcedLimitWatts: number | null;
    defaultLimitWatts: number | null;
  };
  clocks: {
    graphicsCurrentMHz: number | null;
    graphicsMaxMHz: number | null;
    smMaxMHz: number | null;
    memoryMaxMHz: number | null;
  };
  control: {
    managedByDaemon: boolean;
    daemonReachable: boolean;
    supportedFeatures: {
      powerLimit: boolean;
      gpuClockLock: boolean;
      memoryClockLock: boolean;
      graphicsOffset: boolean;
      memoryOffset: boolean;
      reset: boolean;
    };
    powerLimit: {
      currentWatts: number | null;
      defaultWatts: number | null;
      minWatts: number | null;
      maxWatts: number | null;
      desiredWatts: number | null;
    };
    gpuClockLock: {
      currentMinMHz: number | null;
      currentMaxMHz: number | null;
      minAllowedMHz: number | null;
      maxAllowedMHz: number | null;
      supported: boolean;
    };
    memoryClockLock: {
      currentMinMHz: number | null;
      currentMaxMHz: number | null;
      minAllowedMHz: number | null;
      maxAllowedMHz: number | null;
      supported: boolean;
    };
    offsets: {
      graphicsMHz: number | null;
      memoryMHz: number | null;
      minGraphicsMHz: number | null;
      maxGraphicsMHz: number | null;
      minMemoryMHz: number | null;
      maxMemoryMHz: number | null;
    };
    warnings: string[];
  };
}

export interface ClockRangeInput {
  minMHz: number;
  maxMHz: number;
}

export interface ApplyGpuConfigRequest {
  gpuId?: string;
  powerLimitWatts?: number;
  gpuClockLock?: ClockRangeInput;
  memoryClockLock?: ClockRangeInput;
  graphicsOffsetMHz?: number;
  memoryOffsetMHz?: number;
  reapplySaved?: boolean;
}

export interface ResetGpuConfigRequest {
  gpuId: string;
}

export interface GpuCommandError {
  code: GpuCommandErrorCode;
  message: string;
}

export interface GpuMutationResponse {
  success: boolean;
  inventory: GpuInventoryResponse | null;
  error: GpuCommandError | null;
}

export interface GpuDaemonStatus {
  reachable: boolean;
  managedByDaemon: boolean;
  socketPath: string;
  message: string | null;
  /** Unknown while the daemon can't be reached. */
  tuningStore: TuningStoreHealth | null;
}

/** Why the daemon can't use its tuning store. */
export type TuningStoreProblemKind = "unreadable" | "newerVersion";

/**
 * Whether the daemon can use its tuning store. An unhealthy store is
 * preserved untouched; manual live tuning keeps working.
 */
export type TuningStoreHealth =
  | { status: "healthy" }
  | {
      status: "unhealthy";
      problem: TuningStoreProblemKind;
      path: string;
      detail: string;
    };

export interface GpuPerformanceDevice {
  id: string;
  stableId: string | null;
  vendor: GpuVendor;
  name: string;
  architecture: string | null;
  busId: string | null;
  power: PowerControlInfo;
  cooling: CoolingInfo;
  clocks: ClockOffsetsInfo;
  telemetry: GpuTelemetryInfo;
}

export interface GpuTelemetryInfo {
  powerDrawWatts: number | null;
  coreClockMHz: number | null;
  memoryClockMHz: number | null;
  fanSpeedPercent: number | null;
  voltageMillivolts: number | null;
  temperatureC: number | null;
  gpuUtilizationPercent: number | null;
  memoryUsedMiB: number | null;
  memoryTotalMiB: number | null;
  performanceState: string | null;
}

export type ControlSupport = "supported" | "unsupported" | "unknown";

export interface PowerControlInfo {
  support: ControlSupport;
  reason: string | null;
  currentValueError: string | null;
  currentDrawWatts: number | null;
  currentLimitWatts: number | null;
  defaultLimitWatts: number | null;
  enforcedLimitWatts: number | null;
  minLimitWatts: number | null;
  maxLimitWatts: number | null;
}

export interface CoolingInfo {
  isSupported: boolean;
  unsupportedReason: string | null;
  fanCount: number;
  fans: FanControlInfo[];
}

export interface FanControlInfo {
  index: number;
  currentSpeedPercent: number | null;
  minSpeedPercent: number | null;
  maxSpeedPercent: number | null;
  mode: FanControlMode | null;
}

export type FanControlMode = "auto" | "manual";

export interface ClockOffsetsInfo {
  graphics: ClockOffsetInfo;
  memory: ClockOffsetInfo;
}

export interface ClockOffsetInfo {
  support: ControlSupport;
  reason: string | null;
  currentValueError: string | null;
  currentOffsetMHz: number | null;
  minOffsetMHz: number | null;
  maxOffsetMHz: number | null;
}

export interface TuningOutcome {
  status: "succeeded" | "failed" | "unattempted" | "unknown";
  error: string | null;
}

export interface ClockOffsetResponse {
  graphics: TuningOutcome | null;
  memory: TuningOutcome | null;
}

export interface GpuCommandResponse {
  success: boolean;
  error: string | null;
}

export type ProfileSetting =
  | { kind: "value"; value: number }
  | { kind: "default" };

export interface GpuProfileSettings {
  powerLimitWatts?: ProfileSetting;
  graphicsOffsetMHz?: ProfileSetting;
  memoryOffsetMHz?: ProfileSetting;
}

export interface GpuTuningProfile {
  /** Opaque, assigned by the daemon. */
  id: string;
  name: string;
  /** The owning GPU's UUID. */
  gpuId: string;
  settings: GpuProfileSettings;
}

export type ProfileSettingKey = keyof GpuProfileSettings;

export interface SaveGpuProfileRequest {
  /** The owning GPU's UUID. The daemon captures its current values. */
  gpuId: string;
  name: string;
}

export interface ApplyGpuProfileByIdRequest {
  profileId: string;
}

export interface ApplyGpuProfileRequest {
  /** Verified stable identity of the owning GPU. */
  gpuId: string;
  settings: GpuProfileSettings;
}

export interface ProfileApplicationResponse extends ClockOffsetResponse {
  power: TuningOutcome | null;
  /** Present only when out-of-range values alone blocked application; nothing was written. */
  corrections?: ProfileRangeCorrection[];
}

export interface ProfileRangeCorrection {
  setting: keyof GpuProfileSettings;
  savedValue: number;
  proposedValue: number;
  bound: RangeBound;
}

export type RangeBound = "minimum" | "maximum";

export interface CorrectGpuProfileRequest {
  profileId: string;
  corrections: ProfileRangeCorrection[];
}

export interface CorrectedProfileApplication {
  /** The profile as saved with the corrections, whatever the application's outcome. */
  profile: GpuTuningProfile;
  application: ProfileApplicationResponse;
}

/** A live clock offset: MHz, an explicit request for the default, or null to leave it unchanged. */
export type OffsetInstruction = number | "default" | null;
