import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import path from "node:path";
import readline from "node:readline";
import { GpuProfileStore } from "./gpu-profile-store";
import type {
  CorrectGpuProfileRequest,
  GpuPerformanceResponse,
} from "@/features/gpus/types";

import {
  app,
  BrowserWindow,
  ipcMain,
  shell,
  type IpcMainInvokeEvent,
} from "electron";

type MainWindowGetter = () => BrowserWindow | null;

type SidecarRequest = {
  id: number;
  method: string;
  params?: unknown;
};

type SidecarError = {
  code: string;
  message: string;
};

type SidecarResponse = {
  id: number;
  result?: unknown;
  error?: SidecarError;
};

type PendingRequest = {
  resolve: (value: unknown) => void;
  reject: (reason?: unknown) => void;
  timeout: NodeJS.Timeout;
};

type MutationActionPayload =
  | { gpuId: string; limitWatts: number }
  | { gpuId: string }
  | { gpuId: string; fanIndex: number; speedPercent: number }
  | { gpuId: string; fanIndex: number }
  | {
      gpuId: string;
      graphicsOffsetMHz: number | null;
      memoryOffsetMHz: number | null;
    }
  | Record<string, unknown>;

type DriverActionPayload = { action: "installRecommended" | "updateInstalled" };
type DriverJobPayload = { jobId: string };

class RustBackendClient {
  private child: ChildProcessWithoutNullStreams | null = null;
  private reader: readline.Interface | null = null;
  private nextId = 0;
  private pending = new Map<number, PendingRequest>();

  async invoke<T>(method: string, params?: unknown): Promise<T> {
    this.ensureStarted();

    const child = this.child;
    if (!child) {
      throw new Error("Prism backend failed to start");
    }

    const id = ++this.nextId;
    const payload: SidecarRequest = { id, method, params };

    return new Promise<T>((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(id);
        reject(
          new Error(`Timed out waiting for Prism backend method ${method}`),
        );
      }, 5_000);

      this.pending.set(id, {
        resolve: resolve as (value: unknown) => void,
        reject,
        timeout,
      });
      child.stdin.write(`${JSON.stringify(payload)}\n`);
    });
  }

  dispose() {
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timeout);
      pending.reject(new Error("Prism backend stopped"));
    }
    this.pending.clear();
    this.reader?.close();
    this.reader = null;
    this.child?.kill();
    this.child = null;
  }

  private ensureStarted() {
    if (this.child && this.child.exitCode === null && !this.child.killed) {
      return;
    }

    const backend = this.spawnBackend();
    this.child = backend;

    this.reader = readline.createInterface({ input: backend.stdout });
    this.reader.on("line", (line) => this.handleLine(line));

    backend.stderr.on("data", (chunk) => {
      process.stderr.write(`[prism_backend] ${chunk}`);
    });

    backend.on("exit", (code, signal) => {
      const reason = new Error(
        `Prism backend exited unexpectedly (code=${code ?? "null"} signal=${signal ?? "null"})`,
      );
      for (const pending of this.pending.values()) {
        clearTimeout(pending.timeout);
        pending.reject(reason);
      }
      this.pending.clear();
      this.reader?.close();
      this.reader = null;
      this.child = null;
    });
  }

  private spawnBackend() {
    const isFlatpak = Boolean(process.env.FLATPAK_ID);

    if (app.isPackaged) {
      const binaryPath = path.join(process.resourcesPath, "prism_backend");
      return spawn(binaryPath, [], {
        stdio: ["pipe", "pipe", "pipe"],
        env: {
          ...process.env,
          ...(isFlatpak ? { PRISM_BACKEND_REQUIRE_DAEMON: "1" } : {}),
        },
      });
    }

    const projectRoot = path.resolve(__dirname, "..", "..");
    return spawn(
      "cargo",
      [
        "run",
        "--manifest-path",
        path.join(projectRoot, "rust/Cargo.toml"),
        "--bin",
        "prism_backend",
      ],
      {
        cwd: projectRoot,
        stdio: ["pipe", "pipe", "pipe"],
        env: {
          ...process.env,
          ...(isFlatpak ? { PRISM_BACKEND_REQUIRE_DAEMON: "1" } : {}),
        },
      },
    );
  }

  private handleLine(line: string) {
    if (!line.trim()) {
      return;
    }

    let response: SidecarResponse;
    try {
      response = JSON.parse(line) as SidecarResponse;
    } catch (error) {
      process.stderr.write(
        `[prism_backend] invalid JSON response: ${String(error)}\n`,
      );
      return;
    }

    const pending = this.pending.get(response.id);
    if (!pending) {
      return;
    }

    clearTimeout(pending.timeout);
    this.pending.delete(response.id);

    if (response.error) {
      pending.reject(new Error(response.error.message));
      return;
    }

    pending.resolve(response.result);
  }
}

function assertTrustedSender(
  event: IpcMainInvokeEvent,
  getMainWindow: MainWindowGetter,
) {
  const window = getMainWindow();
  if (!window) {
    throw new Error("Main window is not available");
  }

  if (event.sender.id !== window.webContents.id) {
    throw new Error("Untrusted IPC sender");
  }
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isString(value: unknown): value is string {
  return typeof value === "string";
}

function isNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isNullableNumber(value: unknown): value is number | null {
  return value === null || isNumber(value);
}

function validateGpuTarget(payload: unknown): payload is { gpuId: string } {
  return isObject(payload) && isString(payload.gpuId);
}

function validatePowerLimitPayload(
  payload: unknown,
): payload is Extract<
  MutationActionPayload,
  { gpuId: string; limitWatts: number }
> {
  return (
    isObject(payload) && isString(payload.gpuId) && isNumber(payload.limitWatts)
  );
}

function validateFanSpeedPayload(
  payload: unknown,
): payload is Extract<
  MutationActionPayload,
  { gpuId: string; fanIndex: number; speedPercent: number }
> {
  return (
    isObject(payload) &&
    isString(payload.gpuId) &&
    isNumber(payload.fanIndex) &&
    isNumber(payload.speedPercent)
  );
}

function validateFanModePayload(
  payload: unknown,
): payload is Extract<
  MutationActionPayload,
  { gpuId: string; fanIndex: number }
> {
  return (
    isObject(payload) && isString(payload.gpuId) && isNumber(payload.fanIndex)
  );
}

function validateClockOffsetPayload(payload: unknown): payload is Extract<
  MutationActionPayload,
  {
    gpuId: string;
    graphicsOffsetMHz: number | null;
    memoryOffsetMHz: number | null;
  }
> {
  return (
    isObject(payload) &&
    isString(payload.gpuId) &&
    isNullableNumber(payload.graphicsOffsetMHz) &&
    isNullableNumber(payload.memoryOffsetMHz)
  );
}

function validateApplyConfigPayload(payload: unknown) {
  if (!isObject(payload)) {
    return false;
  }

  return (
    (payload.gpuId === undefined || isString(payload.gpuId)) &&
    (payload.powerLimitWatts === undefined ||
      isNumber(payload.powerLimitWatts)) &&
    (payload.graphicsOffsetMHz === undefined ||
      isNumber(payload.graphicsOffsetMHz)) &&
    (payload.memoryOffsetMHz === undefined ||
      isNumber(payload.memoryOffsetMHz)) &&
    (payload.reapplySaved === undefined ||
      typeof payload.reapplySaved === "boolean")
  );
}

function validateDriverActionPayload(
  payload: unknown,
): payload is DriverActionPayload {
  return (
    isObject(payload) &&
    (payload.action === "installRecommended" ||
      payload.action === "updateInstalled")
  );
}

function validateDriverJobPayload(
  payload: unknown,
): payload is DriverJobPayload {
  return isObject(payload) && isString(payload.jobId);
}

// A correction only moves a saved value to the bound it exceeded; it is never a free-form edit.
function validateCorrectProfilePayload(
  payload: unknown,
): payload is CorrectGpuProfileRequest {
  if (
    !isObject(payload) ||
    Object.keys(payload).some(
      (key) => key !== "profileId" && key !== "corrections",
    ) ||
    !isString(payload.profileId) ||
    !payload.profileId.trim() ||
    !Array.isArray(payload.corrections) ||
    payload.corrections.length === 0
  )
    return false;
  const settings = new Set<unknown>();
  return payload.corrections.every((correction: unknown) => {
    if (
      !isObject(correction) ||
      Object.keys(correction).length !== 4 ||
      !["powerLimitWatts", "graphicsOffsetMHz", "memoryOffsetMHz"].includes(
        correction.setting as string,
      ) ||
      settings.has(correction.setting) ||
      !Number.isSafeInteger(correction.savedValue) ||
      !Number.isSafeInteger(correction.proposedValue)
    )
      return false;
    settings.add(correction.setting);
    const saved = correction.savedValue as number;
    const proposed = correction.proposedValue as number;
    return (
      (correction.bound === "maximum" && proposed < saved) ||
      (correction.bound === "minimum" && proposed > saved)
    );
  });
}

function isAllowedExternalUrl(url: string) {
  return url.startsWith("https://") || url.startsWith("mailto:");
}

export function registerIpc(getMainWindow: MainWindowGetter) {
  const backend = new RustBackendClient();
  let profileStore: GpuProfileStore | undefined;
  const getProfileStore = () =>
    (profileStore ??= new GpuProfileStore(
      path.join(app.getPath("userData"), "gpu-tuning-profiles.json"),
    ));

  ipcMain.handle("prism:gpu:listProfiles", async (event) => {
    assertTrustedSender(event, getMainWindow);
    return getProfileStore().list();
  });

  ipcMain.handle("prism:gpu:applyProfile", async (event, payload: unknown) => {
    assertTrustedSender(event, getMainWindow);
    if (
      !isObject(payload) ||
      !isString(payload.gpuId) ||
      !payload.gpuId.trim() ||
      Object.keys(payload).some(
        (key) => key !== "gpuId" && key !== "settings",
      ) ||
      !isObject(payload.settings) ||
      Array.isArray(payload.settings) ||
      Object.keys(payload.settings).length === 0 ||
      Object.entries(payload.settings).some(
        ([key, value]) =>
          !["powerLimitWatts", "graphicsOffsetMHz", "memoryOffsetMHz"].includes(
            key,
          ) || !Number.isSafeInteger(value),
      )
    ) {
      throw new Error("Invalid applyProfile payload");
    }
    return backend.invoke("gpu.applyProfile", payload);
  });

  ipcMain.handle(
    "prism:gpu:correctProfile",
    async (event, payload: unknown) => {
      assertTrustedSender(event, getMainWindow);
      if (!validateCorrectProfilePayload(payload)) {
        throw new Error("Invalid correctProfile payload");
      }
      return getProfileStore().correct(payload.profileId, payload.corrections);
    },
  );

  ipcMain.handle("prism:gpu:saveProfile", async (event, payload: unknown) => {
    assertTrustedSender(event, getMainWindow);
    if (
      !isObject(payload) ||
      !isString(payload.gpuId) ||
      !payload.gpuId.trim() ||
      !isString(payload.name) ||
      !payload.name.trim()
    ) {
      throw new Error("Invalid saveProfile payload");
    }
    const performance =
      await backend.invoke<GpuPerformanceResponse>("gpu.getPerformance");
    const gpu = performance.gpus.find((device) => device.id === payload.gpuId);
    if (!gpu)
      throw new Error(
        "The owning GPU is unavailable. Refresh the GPU readings and try again.",
      );
    return getProfileStore().save(payload.name, gpu);
  });

  ipcMain.handle("prism:gpu:getInventory", async (event) => {
    assertTrustedSender(event, getMainWindow);
    return backend.invoke("gpu.getInventory");
  });

  ipcMain.handle("prism:gpu:getPerformance", async (event) => {
    assertTrustedSender(event, getMainWindow);
    return backend.invoke("gpu.getPerformance");
  });

  ipcMain.handle("prism:gpu:getDaemonStatus", async (event) => {
    assertTrustedSender(event, getMainWindow);
    return backend.invoke("gpu.getDaemonStatus");
  });

  ipcMain.handle("prism:gpu:applyConfig", async (event, payload) => {
    assertTrustedSender(event, getMainWindow);
    if (!validateApplyConfigPayload(payload)) {
      throw new Error("Invalid applyConfig payload");
    }
    return backend.invoke("gpu.applyConfig", payload);
  });

  ipcMain.handle("prism:gpu:resetConfig", async (event, payload) => {
    assertTrustedSender(event, getMainWindow);
    if (!validateGpuTarget(payload)) {
      throw new Error("Invalid resetConfig payload");
    }
    return backend.invoke("gpu.resetConfig", payload);
  });

  ipcMain.handle("prism:gpu:setPowerLimit", async (event, payload) => {
    assertTrustedSender(event, getMainWindow);
    if (!validatePowerLimitPayload(payload)) {
      throw new Error("Invalid setPowerLimit payload");
    }
    return backend.invoke("gpu.setPowerLimit", payload);
  });

  ipcMain.handle("prism:gpu:resetPowerLimit", async (event, payload) => {
    assertTrustedSender(event, getMainWindow);
    if (!validateGpuTarget(payload)) {
      throw new Error("Invalid resetPowerLimit payload");
    }
    return backend.invoke("gpu.resetPowerLimit", payload);
  });

  ipcMain.handle("prism:gpu:setFanSpeed", async (event, payload) => {
    assertTrustedSender(event, getMainWindow);
    if (!validateFanSpeedPayload(payload)) {
      throw new Error("Invalid setFanSpeed payload");
    }
    return backend.invoke("gpu.setFanSpeed", payload);
  });

  ipcMain.handle("prism:gpu:setFanModeAuto", async (event, payload) => {
    assertTrustedSender(event, getMainWindow);
    if (!validateFanModePayload(payload)) {
      throw new Error("Invalid setFanModeAuto payload");
    }
    return backend.invoke("gpu.setFanModeAuto", payload);
  });

  ipcMain.handle("prism:gpu:setClockOffsets", async (event, payload) => {
    assertTrustedSender(event, getMainWindow);
    if (!validateClockOffsetPayload(payload)) {
      throw new Error("Invalid setClockOffsets payload");
    }
    return backend.invoke("gpu.setClockOffsets", payload);
  });

  ipcMain.handle("prism:gpu:resetClockOffsets", async (event, payload) => {
    assertTrustedSender(event, getMainWindow);
    if (!validateGpuTarget(payload)) {
      throw new Error("Invalid resetClockOffsets payload");
    }
    return backend.invoke("gpu.resetClockOffsets", payload);
  });

  ipcMain.handle("prism:drivers:getStatus", async (event) => {
    assertTrustedSender(event, getMainWindow);
    return backend.invoke("drivers.getStatus");
  });

  ipcMain.handle("prism:drivers:startAction", async (event, payload) => {
    assertTrustedSender(event, getMainWindow);
    if (!validateDriverActionPayload(payload)) {
      throw new Error("Invalid startAction payload");
    }
    return backend.invoke("drivers.startAction", payload);
  });

  ipcMain.handle("prism:drivers:getJob", async (event, payload) => {
    assertTrustedSender(event, getMainWindow);
    if (!validateDriverJobPayload(payload)) {
      throw new Error("Invalid getJob payload");
    }
    return backend.invoke("drivers.getJob", payload);
  });

  ipcMain.handle("prism:drivers:cancelJob", async (event, payload) => {
    assertTrustedSender(event, getMainWindow);
    if (!validateDriverJobPayload(payload)) {
      throw new Error("Invalid cancelJob payload");
    }
    return backend.invoke("drivers.cancelJob", payload);
  });

  ipcMain.handle("prism:drivers:requestReboot", async (event) => {
    assertTrustedSender(event, getMainWindow);
    return backend.invoke("drivers.requestReboot");
  });

  ipcMain.handle("prism:window:minimize", async (event) => {
    assertTrustedSender(event, getMainWindow);
    getMainWindow()?.minimize();
  });

  ipcMain.handle("prism:window:maximize", async (event) => {
    assertTrustedSender(event, getMainWindow);
    getMainWindow()?.maximize();
  });

  ipcMain.handle("prism:window:unmaximize", async (event) => {
    assertTrustedSender(event, getMainWindow);
    getMainWindow()?.unmaximize();
  });

  ipcMain.handle("prism:window:close", async (event) => {
    assertTrustedSender(event, getMainWindow);
    getMainWindow()?.close();
  });

  ipcMain.handle("prism:window:isMaximized", async (event) => {
    assertTrustedSender(event, getMainWindow);
    return getMainWindow()?.isMaximized() ?? false;
  });

  ipcMain.handle("prism:shell:openExternal", async (event, url: unknown) => {
    assertTrustedSender(event, getMainWindow);
    if (!isString(url) || !isAllowedExternalUrl(url)) {
      throw new Error("Blocked external URL");
    }
    await shell.openExternal(url);
  });

  return () => backend.dispose();
}
