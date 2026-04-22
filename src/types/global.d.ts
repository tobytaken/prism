import type {
  ApplyGpuConfigRequest,
  GpuDaemonStatus,
  GpuInventoryResponse,
  GpuMutationResponse,
  GpuPerformanceResponse,
  ResetGpuConfigRequest,
} from "@/features/gpus/types";

export interface PrismDesktopApi {
  gpu: {
    getInventory(): Promise<GpuInventoryResponse>;
    getPerformance(): Promise<GpuPerformanceResponse>;
    getDaemonStatus(): Promise<GpuDaemonStatus>;
    applyConfig(request: ApplyGpuConfigRequest): Promise<GpuMutationResponse>;
    resetConfig(request: ResetGpuConfigRequest): Promise<GpuMutationResponse>;
    setPowerLimit(gpuId: string, limitWatts: number): Promise<void>;
    resetPowerLimit(gpuId: string): Promise<void>;
    setFanSpeed(gpuId: string, fanIndex: number, speedPercent: number): Promise<void>;
    setFanModeAuto(gpuId: string, fanIndex: number): Promise<void>;
    setClockOffsets(
      gpuId: string,
      graphicsOffsetMHz: number | null,
      memoryOffsetMHz: number | null,
    ): Promise<void>;
    resetClockOffsets(gpuId: string): Promise<void>;
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
