import type {
  ApplyGpuConfigRequest,
  ApplyGpuProfileRequest,
  CorrectGpuProfileRequest,
  ProfileApplicationResponse,
  ClockOffsetResponse,
  GpuCommandResponse,
  GpuDaemonStatus,
  GpuInventoryResponse,
  GpuMutationResponse,
  GpuPerformanceResponse,
  ResetGpuConfigRequest,
  GpuTuningProfile,
  SaveGpuProfileRequest,
} from "@/features/gpus/types";
import type {
  DriverAction,
  DriverJob,
  DriverStatusResponse,
} from "@/features/drivers/types";

export interface PrismDesktopApi {
  gpu: {
    listProfiles(): Promise<GpuTuningProfile[]>;
    saveProfile(request: SaveGpuProfileRequest): Promise<GpuTuningProfile>;
    correctProfile(
      request: CorrectGpuProfileRequest,
    ): Promise<GpuTuningProfile>;
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
      graphicsOffsetMHz: number | null,
      memoryOffsetMHz: number | null,
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
