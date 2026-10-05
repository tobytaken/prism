import type {
  ApplyGpuConfigRequest,
  ApplyGpuProfileByIdRequest,
  ApplyGpuProfileRequest,
  CorrectGpuProfileRequest,
  GpuDaemonStatus,
  GpuInventoryResponse,
  GpuMutationResponse,
  GpuPerformanceResponse,
  OffsetInstruction,
  ResetGpuConfigRequest,
  SaveGpuProfileRequest,
} from "@/features/gpus/types";

export function listGpuProfiles() {
  return window.prism.gpu.listProfiles();
}

export function applyGpuProfile(request: ApplyGpuProfileRequest) {
  return window.prism.gpu.applyProfile(request);
}

export function applyGpuProfileById(request: ApplyGpuProfileByIdRequest) {
  return window.prism.gpu.applyProfileById(request);
}

export function correctAndApplyGpuProfile(request: CorrectGpuProfileRequest) {
  return window.prism.gpu.correctAndApplyProfile(request);
}

export function saveGpuProfile(request: SaveGpuProfileRequest) {
  return window.prism.gpu.saveProfile(request);
}

export function getGpuInventory() {
  return window.prism.gpu.getInventory() as Promise<GpuInventoryResponse>;
}

export function getGpuPerformance() {
  return window.prism.gpu.getPerformance() as Promise<GpuPerformanceResponse>;
}

export function setGpuPowerLimit(gpuId: string, limitWatts: number) {
  return window.prism.gpu.setPowerLimit(gpuId, limitWatts);
}

export function resetGpuPowerLimit(gpuId: string) {
  return window.prism.gpu.resetPowerLimit(gpuId);
}

export function setGpuFanSpeed(
  gpuId: string,
  fanIndex: number,
  speedPercent: number,
) {
  return window.prism.gpu.setFanSpeed(gpuId, fanIndex, speedPercent);
}

export function setGpuFanModeAuto(gpuId: string, fanIndex: number) {
  return window.prism.gpu.setFanModeAuto(gpuId, fanIndex);
}

export function setGpuClockOffsets(
  gpuId: string,
  graphicsOffsetMHz: OffsetInstruction,
  memoryOffsetMHz: OffsetInstruction,
) {
  return window.prism.gpu.setClockOffsets(
    gpuId,
    graphicsOffsetMHz,
    memoryOffsetMHz,
  );
}

export function resetGpuClockOffsets(gpuId: string) {
  return window.prism.gpu.resetClockOffsets(gpuId);
}

export function getGpuDaemonStatus() {
  return window.prism.gpu.getDaemonStatus() as Promise<GpuDaemonStatus>;
}

export function applyGpuConfig(request: ApplyGpuConfigRequest) {
  return window.prism.gpu.applyConfig(request) as Promise<GpuMutationResponse>;
}

export function resetGpuConfig(request: ResetGpuConfigRequest) {
  return window.prism.gpu.resetConfig(request) as Promise<GpuMutationResponse>;
}
