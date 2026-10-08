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

export interface GpuCommandError {
  code: GpuCommandErrorCode;
  message: string;
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
  /** Known only when the GPU reports a trustworthy absolute temperature. */
  maxOperatingTemperatureC: number | null;
  /** Why the maximum operating temperature is unknown. */
  maxOperatingTemperatureReason: string | null;
  /** The daemon's judgement of shared fan curves; null when the readings bypassed the daemon. */
  curveSupport: FanCurveSupport | null;
  /** Prism's control of the fans; null when the readings bypassed the daemon. */
  control: CoolingControl | null;
}

/** One fan's control mode. A curve is followed by the daemon as the temperature changes. */
export type FanSetting =
  | { mode: "automatic" }
  | { mode: "fixed"; speedPercent: number }
  | { mode: "curve"; points: FanCurvePoint[] };

/** One setting shared by every fan, or a setting per fan; omitted fans are left as they are. */
export type CoolingConfiguration =
  | { arrangement: "shared"; setting: FanSetting }
  | {
      arrangement: "perFan";
      fans: { fanIndex: number; setting: FanSetting }[];
    };

export interface ApplyCoolingRequest {
  fanIndexes?: number[];
  expectedRevision?: number;
  /** The GPU's UUID. */
  gpuId: string;
  configuration: CoolingConfiguration;
}

/** Succeeded only when every targeted fan took its setting. */
export interface CoolingApplication {
  revision: number;
  outcome: TuningOutcome;
  fans: { fanIndex: number; outcome: TuningOutcome }[];
}

/**
 * Prism's control of a GPU's fans: the applied configuration, the speeds a curve requests now,
 * and any curve suspension, all distinct from the modes and speeds the driver reports.
 */
export interface CoolingControl {
  /** Configuration/ownership revision; temperature and driver observations do not change it. */
  revision: number;
  /** Null when Prism hasn't applied cooling to this GPU. A suspended curve stays applied. */
  applied: CoolingConfiguration | null;
  /** Each fan following a curve, with the speed the curve requests now. */
  requested: { fanIndex: number; speedPercent: number }[];
  /** The GPU temperature the curve last followed. */
  curveTemperatureC: number | null;
  /** Why curve control stopped; only a new application resumes the kept curve. */
  suspension: CurveSuspension | null;
  /**
   * What the daemon did with the applied curves when it restarted within this host boot, until
   * an application or a later suspension replaces it.
   */
  recovery: CoolingRecovery | null;
}

/**
 * A restarted daemon's attempt to resume the curves it was following when it stopped. A curve
 * that couldn't resume is suspended and waits for a retry.
 */
export interface CoolingRecovery {
  status: "resumed" | "retryRequired";
  /** Each interrupted curve fan's resumption. */
  fans: { fanIndex: number; outcome: TuningOutcome }[];
}

export interface CurveSuspension {
  kind: "fault" | "externalControl";
  reason: string;
  /** Each curve fan's attempt to return to the GPU's default automatic cooling. */
  restoration: FanRestoration[];
}

export interface FanRestoration {
  fanIndex: number;
  /** Restored: the driver reports automatic control. Unverified: accepted, but not reported. */
  status: "restored" | "unverified" | "failed";
  detail: string | null;
}

/** A requested fan speed at a GPU temperature, in whole °C and whole percent. */
export interface FanCurvePoint {
  temperatureC: number;
  speedPercent: number;
}

/** Whether one shared curve can control every fan on a GPU, and within which limits. */
export type FanCurveSupport =
  | {
      status: "available";
      /** The highest temperature a point can use. */
      maxOperatingTemperatureC: number;
      /** The temperature by which the curve must request the maximum speed. */
      fullSpeedTemperatureC: number;
      /** Zero only when every fan supports it. */
      minSpeedPercent: number;
      maxSpeedPercent: number;
      /** The starter fan curve: editable, and not the GPU's own firmware curve. */
      starter: FanCurvePoint[];
    }
  | { status: "unavailable"; reason: string };

export interface PreviewFanCurveRequest {
  fanIndex?: number;
  gpuId: string;
  points: FanCurvePoint[];
}

/** A reason a curve can't be used; point is the zero-based index of the point at fault. */
export interface FanCurveProblem {
  point: number | null;
  message: string;
}

/** The daemon's check of a draft curve; nothing is applied. */
export interface FanCurvePreview {
  support?: FanCurveSupport;
  problems: FanCurveProblem[];
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
  /** The GPU's state revision the values were seen at; a stale one is rejected. */
  revision: string;
}

export interface SaveToCurrentProfileRequest {
  /** The GPU's UUID. The daemon overwrites its current profile with the current values. */
  gpuId: string;
  /** The GPU's state revision the values were seen at; a stale one is rejected. */
  revision: string;
}

/** A GPU's current profile, shown by name to every user whoever owns it. */
export interface CurrentProfile {
  id: string;
  name: string;
}

/** A GPU's current profile and the revision of its reported and recorded tuning state. */
export interface GpuTuningState {
  /** The GPU's UUID. */
  gpuId: string;
  revision: string;
  /** The current tuning values the revision covers; null where unreadable. */
  reported: Record<ProfileSettingKey, number | null>;
  currentProfile: CurrentProfile | null;
}

export interface GpuProfileIdRequest {
  profileId: string;
}

export type ApplyGpuProfileByIdRequest = GpuProfileIdRequest;

export interface RenameGpuProfileRequest extends GpuProfileIdRequest {
  name: string;
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

/** The host-wide choice of what the daemon puts into effect when the host starts. */
export type StartupBehavior = "off" | "startupProfiles" | "restoreLastApplied";
export interface SetStartupProfileRequest {
  gpuId: string;
  profileId: string | null;
}
export interface StartupOutcome {
  gpuId: string;
  /** The startup behavior this boot ran. */
  behavior: StartupBehavior;
  /** The startup profile applied, or null when restoring last applied tuning values. */
  profile: CurrentProfile | null;
  status: "applied" | "failed" | "notAttempted";
  reason: string | null;
  application: ProfileApplicationResponse | null;
}
export interface StartupState {
  behavior: StartupBehavior;
  profiles: Array<{ gpuId: string; profile: CurrentProfile }>;
  outcomes: StartupOutcome[];
  /** GPU UUIDs whose last applied tuning values miss a manual write that couldn't be recorded. */
  possiblyStale: string[];
}

/** Host-held profiles and the caller's one-time upgrade notice. */
export interface MigrationState {
  profiles: GpuTuningProfile[];
  canAdopt: boolean;
  notice: {
    dropped: { key: string; values: Record<string, unknown>; reason: string }[];
  } | null;
}
