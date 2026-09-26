import type { DriverAction, DriverJob, DriverStatusResponse } from "./types";

export function getDriverStatus() {
  return window.prism.drivers.getStatus() as Promise<DriverStatusResponse>;
}

export function startDriverAction(action: DriverAction) {
  return window.prism.drivers.startAction(action) as Promise<DriverJob>;
}

export function getDriverJob(jobId: string) {
  return window.prism.drivers.getJob(jobId) as Promise<DriverJob>;
}

export function cancelDriverJob(jobId: string) {
  return window.prism.drivers.cancelJob(jobId) as Promise<DriverJob>;
}

export function requestHostReboot() {
  return window.prism.drivers.requestReboot();
}
