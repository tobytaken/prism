import { contextBridge, ipcRenderer } from "electron";

import type {
  DriverAction,
  DriverJob,
  DriverStatusResponse,
} from "@/features/drivers/types";
import type {
  ApplyGpuConfigRequest,
  ApplyGpuProfileRequest,
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

const prismApi = {
  gpu: {
    applyProfile: (request: ApplyGpuProfileRequest) =>
      ipcRenderer.invoke(
        "prism:gpu:applyProfile",
        request,
      ) as Promise<ProfileApplicationResponse>,
    listProfiles: () =>
      ipcRenderer.invoke("prism:gpu:listProfiles") as Promise<
        GpuTuningProfile[]
      >,
    saveProfile: (request: SaveGpuProfileRequest) =>
      ipcRenderer.invoke(
        "prism:gpu:saveProfile",
        request,
      ) as Promise<GpuTuningProfile>,
    getInventory: () =>
      ipcRenderer.invoke(
        "prism:gpu:getInventory",
      ) as Promise<GpuInventoryResponse>,
    getPerformance: () =>
      ipcRenderer.invoke(
        "prism:gpu:getPerformance",
      ) as Promise<GpuPerformanceResponse>,
    getDaemonStatus: () =>
      ipcRenderer.invoke(
        "prism:gpu:getDaemonStatus",
      ) as Promise<GpuDaemonStatus>,
    applyConfig: (request: ApplyGpuConfigRequest) =>
      ipcRenderer.invoke(
        "prism:gpu:applyConfig",
        request,
      ) as Promise<GpuMutationResponse>,
    resetConfig: (request: ResetGpuConfigRequest) =>
      ipcRenderer.invoke(
        "prism:gpu:resetConfig",
        request,
      ) as Promise<GpuMutationResponse>,
    setPowerLimit: (gpuId: string, limitWatts: number) =>
      ipcRenderer.invoke("prism:gpu:setPowerLimit", {
        gpuId,
        limitWatts,
      }) as Promise<GpuCommandResponse>,
    resetPowerLimit: (gpuId: string) =>
      ipcRenderer.invoke("prism:gpu:resetPowerLimit", {
        gpuId,
      }) as Promise<GpuCommandResponse>,
    setFanSpeed: (gpuId: string, fanIndex: number, speedPercent: number) =>
      ipcRenderer.invoke("prism:gpu:setFanSpeed", {
        gpuId,
        fanIndex,
        speedPercent,
      }) as Promise<GpuCommandResponse>,
    setFanModeAuto: (gpuId: string, fanIndex: number) =>
      ipcRenderer.invoke("prism:gpu:setFanModeAuto", {
        gpuId,
        fanIndex,
      }) as Promise<GpuCommandResponse>,
    setClockOffsets: (
      gpuId: string,
      graphicsOffsetMHz: number | null,
      memoryOffsetMHz: number | null,
    ) =>
      ipcRenderer.invoke("prism:gpu:setClockOffsets", {
        gpuId,
        graphicsOffsetMHz,
        memoryOffsetMHz,
      }) as Promise<ClockOffsetResponse>,
    resetClockOffsets: (gpuId: string) =>
      ipcRenderer.invoke("prism:gpu:resetClockOffsets", {
        gpuId,
      }) as Promise<ClockOffsetResponse>,
  },
  drivers: {
    getStatus: () =>
      ipcRenderer.invoke(
        "prism:drivers:getStatus",
      ) as Promise<DriverStatusResponse>,
    startAction: (action: DriverAction) =>
      ipcRenderer.invoke("prism:drivers:startAction", {
        action,
      }) as Promise<DriverJob>,
    getJob: (jobId: string) =>
      ipcRenderer.invoke("prism:drivers:getJob", {
        jobId,
      }) as Promise<DriverJob>,
    cancelJob: (jobId: string) =>
      ipcRenderer.invoke("prism:drivers:cancelJob", {
        jobId,
      }) as Promise<DriverJob>,
    requestReboot: () =>
      ipcRenderer.invoke("prism:drivers:requestReboot") as Promise<void>,
  },
  window: {
    minimize: () =>
      ipcRenderer.invoke("prism:window:minimize") as Promise<void>,
    maximize: () =>
      ipcRenderer.invoke("prism:window:maximize") as Promise<void>,
    unmaximize: () =>
      ipcRenderer.invoke("prism:window:unmaximize") as Promise<void>,
    close: () => ipcRenderer.invoke("prism:window:close") as Promise<void>,
    isMaximized: () =>
      ipcRenderer.invoke("prism:window:isMaximized") as Promise<boolean>,
    onResized: (callback: (isMaximized: boolean) => void) => {
      const handler = (
        _event: Electron.IpcRendererEvent,
        isMaximized: boolean,
      ) => {
        callback(isMaximized);
      };

      ipcRenderer.on("prism:window:resized", handler);
      return () => {
        ipcRenderer.removeListener("prism:window:resized", handler);
      };
    },
  },
  shell: {
    openExternal: (url: string) =>
      ipcRenderer.invoke("prism:shell:openExternal", url) as Promise<void>,
  },
};

contextBridge.exposeInMainWorld("prism", prismApi);
