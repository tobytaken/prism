import { contextBridge, ipcRenderer } from "electron";
import { isDaemonStatus, isTuningState } from "./daemon-events";

import type {
  DriverAction,
  DriverJob,
  DriverStatusResponse,
} from "@/features/drivers/types";
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

let daemonSubscribers = 0;

/** Listens for a daemon event, keeping the daemon subscription while any listener remains. */
function onDaemonEvent<T>(
  channel: string,
  isEvent: (value: unknown) => value is T,
  callback: (event: T) => void,
) {
  const handler = (_event: Electron.IpcRendererEvent, value: unknown) => {
    if (isEvent(value)) callback(value);
  };
  ipcRenderer.on(channel, handler);
  if (++daemonSubscribers === 1) {
    void Promise.resolve(ipcRenderer.invoke("prism:daemon:subscribe")).catch(
      () => undefined,
    );
  }
  let subscribed = true;
  return () => {
    if (!subscribed) return;
    subscribed = false;
    ipcRenderer.removeListener(channel, handler);
    if (--daemonSubscribers === 0) {
      void Promise.resolve(
        ipcRenderer.invoke("prism:daemon:unsubscribe"),
      ).catch(() => undefined);
    }
  };
}

function onDaemonStatusChanged(callback: (status: GpuDaemonStatus) => void) {
  return onDaemonEvent("prism:daemon:statusChanged", isDaemonStatus, callback);
}

function onTuningStateChanged(callback: (state: GpuTuningState) => void) {
  return onDaemonEvent("prism:gpu:tuningStateChanged", isTuningState, callback);
}

const prismApi = {
  gpu: {
    onDaemonStatusChanged,
    onTuningStateChanged,
    getTuningStates: () =>
      ipcRenderer.invoke("prism:gpu:getTuningStates") as Promise<
        GpuTuningState[]
      >,
    applyProfile: (request: ApplyGpuProfileRequest) =>
      ipcRenderer.invoke(
        "prism:gpu:applyProfile",
        request,
      ) as Promise<ProfileApplicationResponse>,
    listProfiles: () =>
      ipcRenderer.invoke("prism:gpu:listProfiles") as Promise<
        GpuTuningProfile[]
      >,
    applyProfileById: (request: ApplyGpuProfileByIdRequest) =>
      ipcRenderer.invoke(
        "prism:gpu:applyProfileById",
        request,
      ) as Promise<ProfileApplicationResponse>,
    correctAndApplyProfile: (request: CorrectGpuProfileRequest) =>
      ipcRenderer.invoke(
        "prism:gpu:correctAndApplyProfile",
        request,
      ) as Promise<CorrectedProfileApplication>,
    saveProfile: (request: SaveGpuProfileRequest) =>
      ipcRenderer.invoke(
        "prism:gpu:saveProfile",
        request,
      ) as Promise<GpuTuningProfile>,
    saveProfileToCurrent: (request: SaveToCurrentProfileRequest) =>
      ipcRenderer.invoke(
        "prism:gpu:saveProfileToCurrent",
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
      graphicsOffsetMHz: OffsetInstruction,
      memoryOffsetMHz: OffsetInstruction,
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
