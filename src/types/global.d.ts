import type {
  ApplyGpuConfigRequest,
  ApplyGpuProfileByIdRequest,
  ApplyGpuProfileRequest,
  CorrectGpuProfileRequest,
  CorrectedProfileApplication,
  OffsetInstruction,
  ProfileApplicationResponse,
  ClockOffsetResponse,
  GpuCommandResponse,
  GpuDaemonStatus,
  GpuInventoryResponse,
  GpuMutationResponse,
  GpuPerformanceResponse,
  ResetGpuConfigRequest,
  GpuTuningProfile,
  GpuTuningState,
  SaveGpuProfileRequest,
  SaveToCurrentProfileRequest,
} from "@/features/gpus/types";
import type {
  DriverAction,
  DriverJob,
  DriverStatusResponse,
} from "@/features/drivers/types";

export interface PrismDesktopApi {
  gpu: {
    /** The caller's own profiles. */
    listProfiles(): Promise<GpuTuningProfile[]>;
    /** Save as new, from the GPU's current values; the new profile becomes current. */
    saveProfile(request: SaveGpuProfileRequest): Promise<GpuTuningProfile>;
    /** Overwrites the GPU's current profile, if the caller owns it, with its current values. */
    saveProfileToCurrent(
      request: SaveToCurrentProfileRequest,
    ): Promise<GpuTuningProfile>;
    /** Each identified GPU's current profile and state revision. */
    getTuningStates(): Promise<GpuTuningState[]>;
    /** Current-profile and revision changes pushed by the daemon. */
    onTuningStateChanged(callback: (state: GpuTuningState) => void): () => void;
    applyProfileById(
      request: ApplyGpuProfileByIdRequest,
    ): Promise<ProfileApplicationResponse>;
    /** Saves reviewed range corrections, then applies the corrected profile. */
    correctAndApplyProfile(
      request: CorrectGpuProfileRequest,
    ): Promise<CorrectedProfileApplication>;
    /** Applies profile instructions without a saved profile, such as a retry of some of them. */
    applyProfile(
      request: ApplyGpuProfileRequest,
    ): Promise<ProfileApplicationResponse>;
    getInventory(): Promise<GpuInventoryResponse>;
    getPerformance(): Promise<GpuPerformanceResponse>;
    getDaemonStatus(): Promise<GpuDaemonStatus>;
    onDaemonStatusChanged(
      callback: (status: GpuDaemonStatus) => void,
    ): () => void;
    applyConfig(request: ApplyGpuConfigRequest): Promise<GpuMutationResponse>;
    resetConfig(request: ResetGpuConfigRequest): Promise<GpuMutationResponse>;
    setPowerLimit(
      gpuId: string,
      limitWatts: number,
    ): Promise<GpuCommandResponse>;
    resetPowerLimit(gpuId: string): Promise<GpuCommandResponse>;
    setFanSpeed(
      gpuId: string,
      fanIndex: number,
      speedPercent: number,
    ): Promise<GpuCommandResponse>;
    setFanModeAuto(
      gpuId: string,
      fanIndex: number,
    ): Promise<GpuCommandResponse>;
    setClockOffsets(
      gpuId: string,
      graphicsOffsetMHz: OffsetInstruction,
      memoryOffsetMHz: OffsetInstruction,
    ): Promise<ClockOffsetResponse>;
    resetClockOffsets(gpuId: string): Promise<ClockOffsetResponse>;
  };
  drivers: {
    getStatus(): Promise<DriverStatusResponse>;
    startAction(action: DriverAction): Promise<DriverJob>;
    getJob(jobId: string): Promise<DriverJob>;
    cancelJob(jobId: string): Promise<DriverJob>;
    requestReboot(): Promise<void>;
  };
  window: {
    minimize(): Promise<void>;
    maximize(): Promise<void>;
    unmaximize(): Promise<void>;
    close(): Promise<void>;
    isMaximized(): Promise<boolean>;
    onResized(callback: (isMaximized: boolean) => void): () => void;
  };
  shell: {
    openExternal(url: string): Promise<void>;
  };
}

declare global {
  interface Window {
    prism: PrismDesktopApi;
  }
}

export {};
