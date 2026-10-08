import type {
  AutomaticSwitchingStatus,
  RunningGamesState,
} from "@/features/games/types";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import path from "node:path";
import readline from "node:readline";
import { PROFILE_FILE_NAME, ProfileFileImport } from "./profile-file-import";
import {
  daemonStatusNotification,
  tuningStateNotification,
  runningGamesNotification,
  switchingStatusNotification,
} from "./daemon-events";
import {
  isProfileSetting,
  isProfileSettingKey,
} from "@/features/gpus/lib/tuning-profiles";
import type {
  CorrectGpuProfileRequest,
  GpuDaemonStatus,
  GpuTuningState,
  PreviewFanCurveRequest,
  ApplyCoolingRequest,
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
      graphicsOffsetMHz: number | "default" | null;
      memoryOffsetMHz: number | "default" | null;
    }
  | Record<string, unknown>;

type DriverActionPayload = { action: "installRecommended" | "updateInstalled" };
type DriverJobPayload = { jobId: string };

class RustBackendClient {
  private child: ChildProcessWithoutNullStreams | null = null;
  private reader: readline.Interface | null = null;
  private nextId = 0;
  private pending = new Map<number, PendingRequest>();
  private restartTimer: NodeJS.Timeout | undefined;
  private disposed = false;
  private editHolds: string[] = [];
  private socketPath =
    process.env.PRISM_SOCKET_PATH ?? "/run/prism/prismd.sock";

  constructor(
    private readonly onStatus: (status: GpuDaemonStatus) => void,
    private readonly onTuningState: (state: GpuTuningState) => void,
    private readonly onRunningGames: (state: RunningGamesState) => void,
    private readonly onSwitchingStatus: (
      status: AutomaticSwitchingStatus,
    ) => void,
  ) {}

  start() {
    this.ensureStarted();
  }

  setEditHolds(gpuIds: string[]) {
    // Renderer drafts outlive a restarted sidecar. Seed its first connection with this set.
    this.editHolds = gpuIds;
    return this.invoke("games.setEditHolds", { gpuIds });
  }

  async invoke<T>(method: string, params?: unknown): Promise<T> {
    this.ensureStarted();

    const child = this.child;
    if (!child) {
      throw new Error("Prism backend failed to start");
    }

    const id = ++this.nextId;
    const payload: SidecarRequest = { id, method, params };

    return new Promise<T>((resolve, reject) => {
      const timeout = setTimeout(
        () => {
          this.pending.delete(id);
          reject(
            new Error(`Timed out waiting for Prism backend method ${method}`),
          );
          // Startup can hold a GPU's operation queue for about 30 seconds.
        },
        method.startsWith("gpu.") &&
          !method.startsWith("gpu.get") &&
          !method.startsWith("gpu.list")
          ? 40_000
          : // Game requests read the caller's Steam libraries.
            method.startsWith("games.")
            ? 15_000
            : 5_000,
      );

      this.pending.set(id, {
        resolve: resolve as (value: unknown) => void,
        reject,
        timeout,
      });
      child.stdin.write(`${JSON.stringify(payload)}\n`);
    });
  }

  dispose() {
    this.disposed = true;
    clearTimeout(this.restartTimer);
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timeout);
      pending.reject(new Error("Prism backend stopped"));
    }
    this.pending.clear();
    this.reader?.close();
    this.reader = null;
    const child = this.child;
    this.child = null;
    child?.kill();
  }

  private ensureStarted() {
    if (this.disposed) throw new Error("Prism backend stopped");
    if (this.child && this.child.exitCode === null && !this.child.killed) {
      return;
    }

    clearTimeout(this.restartTimer);
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
      this.handleStopped(backend, reason);
    });
    backend.on("error", (error) => this.handleStopped(backend, error));
  }

  private handleStopped(
    backend: ChildProcessWithoutNullStreams,
    reason: Error,
  ) {
    if (this.child !== backend || this.disposed) return;
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timeout);
      pending.reject(reason);
    }
    this.pending.clear();
    this.reader?.close();
    this.reader = null;
    this.child = null;
    this.onStatus({
      reachable: false,
      managedByDaemon: false,
      socketPath: this.socketPath,
      message: reason.message,
      tuningStore: null,
    });
    this.restartTimer = setTimeout(() => this.ensureStarted(), 250);
  }

  private spawnBackend() {
    const isFlatpak = Boolean(process.env.FLATPAK_ID);

    if (app.isPackaged) {
      const binaryPath = path.join(process.resourcesPath, "prism_backend");
      return spawn(binaryPath, [], {
        stdio: ["pipe", "pipe", "pipe"],
        env: {
          ...process.env,
          PRISM_INITIAL_EDIT_HOLDS: JSON.stringify({ gpuIds: this.editHolds }),
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
          PRISM_INITIAL_EDIT_HOLDS: JSON.stringify({ gpuIds: this.editHolds }),
          ...(isFlatpak ? { PRISM_BACKEND_REQUIRE_DAEMON: "1" } : {}),
        },
      },
    );
  }

  private handleLine(line: string) {
    if (!line.trim()) {
      return;
    }

    let value: unknown;
    try {
      value = JSON.parse(line);
    } catch (error) {
      process.stderr.write(
        `[prism_backend] invalid JSON response: ${String(error)}\n`,
      );
      return;
    }

    const notification = daemonStatusNotification(value);
    if (notification) {
      this.socketPath = notification.socketPath;
      this.onStatus(notification);
      return;
    }
    const runningGames = runningGamesNotification(value);
    if (runningGames) {
      this.onRunningGames(runningGames);
      return;
    }
    const switchingStatus = switchingStatusNotification(value);
    if (switchingStatus) {
      this.onSwitchingStatus(switchingStatus);
      return;
    }
    const tuningState = tuningStateNotification(value);
    if (tuningState) {
      this.onTuningState(tuningState);
      return;
    }
    if (
      !isObject(value) ||
      !Number.isSafeInteger(value.id) ||
      typeof value.id !== "number"
    )
      return;
    const response = value as SidecarResponse;

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
  if (event.senderFrame && event.senderFrame !== window.webContents.mainFrame) {
    throw new Error("Untrusted IPC sender frame");
  }
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isString(value: unknown): value is string {
  return typeof value === "string";
}

function isAppId(value: unknown): value is number {
  return (
    Number.isInteger(value) &&
    (value as number) >= 0 &&
    (value as number) <= 0xffff_ffff
  );
}

function isGameId(value: unknown) {
  return (
    isObject(value) &&
    hasOnlyKeys(value, ["kind", "appId"]) &&
    value.kind === "steam" &&
    isAppId(value.appId)
  );
}

function isNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isOffsetInstruction(
  value: unknown,
): value is number | "default" | null {
  return value === null || value === "default" || Number.isSafeInteger(value);
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

// Whole numbers only: a preview checks exactly the curve that would be applied.
function validateFanCurvePreviewPayload(
  payload: unknown,
): payload is PreviewFanCurveRequest {
  return (
    isObject(payload) &&
    hasOnlyKeys(payload, ["gpuId", "points", "fanIndex"]) &&
    (payload.fanIndex === undefined ||
      (Number.isSafeInteger(payload.fanIndex) &&
        Number(payload.fanIndex) >= 0)) &&
    isString(payload.gpuId) &&
    Array.isArray(payload.points) &&
    payload.points.every(
      (point) =>
        isObject(point) &&
        hasOnlyKeys(point, ["temperatureC", "speedPercent"]) &&
        Number.isSafeInteger(point.temperatureC) &&
        Number.isSafeInteger(point.speedPercent),
    )
  );
}

function isFanCurvePoint(point: unknown) {
  return (
    isObject(point) &&
    hasOnlyKeys(point, ["temperatureC", "speedPercent"]) &&
    Number.isSafeInteger(point.temperatureC) &&
    Number.isSafeInteger(point.speedPercent)
  );
}

function isFanSetting(setting: unknown) {
  if (!isObject(setting)) return false;
  switch (setting.mode) {
    case "automatic":
      return hasOnlyKeys(setting, ["mode"]);
    case "fixed":
      return (
        hasOnlyKeys(setting, ["mode", "speedPercent"]) &&
        Number.isSafeInteger(setting.speedPercent)
      );
    case "curve":
      return (
        hasOnlyKeys(setting, ["mode", "points"]) &&
        Array.isArray(setting.points) &&
        setting.points.every(isFanCurvePoint)
      );
    default:
      return false;
  }
}

function validateApplyCoolingPayload(
  payload: unknown,
): payload is ApplyCoolingRequest {
  if (
    !isObject(payload) ||
    !hasOnlyKeys(payload, [
      "gpuId",
      "configuration",
      "fanIndexes",
      "expectedRevision",
    ]) ||
    (payload.fanIndexes !== undefined &&
      (!Array.isArray(payload.fanIndexes) ||
        !payload.fanIndexes.every(
          (index) => Number.isSafeInteger(index) && index >= 0,
        ))) ||
    (payload.expectedRevision !== undefined &&
      (!Number.isSafeInteger(payload.expectedRevision) ||
        (payload.expectedRevision as number) < 0)) ||
    !isString(payload.gpuId) ||
    !isObject(payload.configuration)
  )
    return false;
  return isCoolingConfiguration(payload.configuration);
}

function isCoolingConfiguration(configuration: unknown) {
  if (!isObject(configuration)) return false;
  if (configuration.arrangement === "shared")
    return (
      hasOnlyKeys(configuration, ["arrangement", "setting"]) &&
      isFanSetting(configuration.setting)
    );
  return (
    configuration.arrangement === "perFan" &&
    hasOnlyKeys(configuration, ["arrangement", "fans"]) &&
    Array.isArray(configuration.fans) &&
    configuration.fans.every(
      (fan) =>
        isObject(fan) &&
        hasOnlyKeys(fan, ["fanIndex", "setting"]) &&
        Number.isSafeInteger(fan.fanIndex) &&
        (fan.fanIndex as number) >= 0 &&
        isFanSetting(fan.setting),
    )
  );
}

function validateClockOffsetPayload(payload: unknown): payload is Extract<
  MutationActionPayload,
  {
    gpuId: string;
    graphicsOffsetMHz: number | "default" | null;
    memoryOffsetMHz: number | "default" | null;
  }
> {
  return (
    isObject(payload) &&
    isString(payload.gpuId) &&
    isOffsetInstruction(payload.graphicsOffsetMHz) &&
    isOffsetInstruction(payload.memoryOffsetMHz)
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

function hasOnlyKeys(value: Record<string, unknown>, keys: string[]) {
  return Object.keys(value).every((key) => keys.includes(key));
}

const CORRECTION_KEYS = ["setting", "savedValue", "proposedValue", "bound"];

// A correction only moves a saved value to the bound it exceeded; it is never a free-form edit.
function validateCorrectProfilePayload(
  payload: unknown,
): payload is CorrectGpuProfileRequest {
  if (
    !isObject(payload) ||
    !hasOnlyKeys(payload, ["profileId", "corrections"]) ||
    !isString(payload.profileId) ||
    !payload.profileId.trim() ||
    !Array.isArray(payload.corrections) ||
    payload.corrections.length === 0
  )
    return false;
  const settings = new Set<unknown>();
  return payload.corrections.every((correction: unknown) => {
    if (isObject(correction) && correction.setting === "cooling") {
      if (
        !hasOnlyKeys(correction, ["setting", "savedValue", "proposedValue"]) ||
        settings.has("cooling") ||
        !isCoolingConfiguration(correction.savedValue) ||
        !isCoolingConfiguration(correction.proposedValue)
      )
        return false;
      settings.add("cooling");
      return true;
    }
    if (
      !isObject(correction) ||
      !hasOnlyKeys(correction, CORRECTION_KEYS) ||
      !isProfileSettingKey(correction.setting) ||
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
  let subscribed = false;
  let latestStatus: GpuDaemonStatus | undefined;
  let profileImport: ProfileFileImport | undefined;
  // A failed import keeps the file and is attempted again on the next connection or listing.
  const importProfileFile = async () => {
    try {
      profileImport ??= new ProfileFileImport(
        path.join(app.getPath("userData"), PROFILE_FILE_NAME),
        (profiles) => backend.invoke("gpu.importProfiles", { profiles }),
      );
      await profileImport.run();
    } catch (error) {
      process.stderr.write(
        `Failed to import GPU tuning profiles into the Prism daemon: ${String(error)}\n`,
      );
    }
  };
  const backend = new RustBackendClient(
    (status) => {
      latestStatus = status;
      if (status.reachable && status.managedByDaemon) void importProfileFile();
      if (subscribed)
        getMainWindow()?.webContents.send("prism:daemon:statusChanged", status);
    },
    (state) => {
      if (subscribed)
        getMainWindow()?.webContents.send(
          "prism:gpu:tuningStateChanged",
          state,
        );
    },
    (state) => {
      if (subscribed)
        getMainWindow()?.webContents.send("prism:games:runningChanged", state);
    },
    (status) => {
      if (subscribed)
        getMainWindow()?.webContents.send(
          "prism:games:switchingStatusChanged",
          status,
        );
    },
  );

  ipcMain.handle(
    "prism:daemon:subscribe",
    async (event, ...args: unknown[]) => {
      assertTrustedSender(event, getMainWindow);
      if (args.length) throw new Error("Invalid daemon subscription payload");
      subscribed = true;
      backend.start();
      if (latestStatus)
        getMainWindow()?.webContents.send(
          "prism:daemon:statusChanged",
          latestStatus,
        );
    },
  );
  ipcMain.handle(
    "prism:daemon:unsubscribe",
    async (event, ...args: unknown[]) => {
      assertTrustedSender(event, getMainWindow);
      if (args.length) throw new Error("Invalid daemon subscription payload");
      subscribed = false;
    },
  );
  ipcMain.handle(
    "prism:gpu:listProfiles",
    async (event, ...args: unknown[]) => {
      assertTrustedSender(event, getMainWindow);
      if (args.length) throw new Error("Invalid listProfiles payload");
      // The app's own profile file joins the daemon's list before the first one is shown.
      await importProfileFile();
      return backend.invoke("gpu.listProfiles");
    },
  );

  for (const method of ["getMigrationState", "dismissMigrationNotice"]) {
    ipcMain.handle(`prism:gpu:${method}`, async (event, ...args: unknown[]) => {
      assertTrustedSender(event, getMainWindow);
      if (args.length) throw new Error(`Invalid ${method} payload`);
      return backend.invoke(`gpu.${method}`);
    });
  }
  ipcMain.handle(
    "prism:gpu:adoptMigratedProfile",
    async (event, payload: unknown) => {
      assertTrustedSender(event, getMainWindow);
      if (
        !isObject(payload) ||
        !hasOnlyKeys(payload, ["profileId"]) ||
        !isString(payload.profileId) ||
        !payload.profileId.trim()
      )
        throw new Error("Invalid adoptMigratedProfile payload");
      return backend.invoke("gpu.adoptMigratedProfile", payload);
    },
  );
  ipcMain.handle(
    "prism:gpu:getStartupState",
    async (event, ...args: unknown[]) => {
      assertTrustedSender(event, getMainWindow);
      if (args.length) throw new Error("Invalid getStartupState payload");
      return backend.invoke("gpu.getStartupState");
    },
  );
  ipcMain.handle(
    "prism:gpu:setStartupBehavior",
    async (event, payload: unknown) => {
      assertTrustedSender(event, getMainWindow);
      if (
        !isObject(payload) ||
        !hasOnlyKeys(payload, ["behavior"]) ||
        (payload.behavior !== "off" &&
          payload.behavior !== "startupProfiles" &&
          payload.behavior !== "restoreLastApplied")
      )
        throw new Error("Invalid setStartupBehavior payload");
      return backend.invoke("gpu.setStartupBehavior", payload);
    },
  );
  ipcMain.handle(
    "prism:gpu:setStartupProfile",
    async (event, payload: unknown) => {
      assertTrustedSender(event, getMainWindow);
      if (
        !isObject(payload) ||
        !hasOnlyKeys(payload, ["gpuId", "profileId"]) ||
        !isString(payload.gpuId) ||
        !payload.gpuId.trim() ||
        !(
          payload.profileId === null ||
          (isString(payload.profileId) && payload.profileId.trim())
        )
      )
        throw new Error("Invalid setStartupProfile payload");
      return backend.invoke("gpu.setStartupProfile", payload);
    },
  );
  ipcMain.handle(
    "prism:gpu:dismissStartupOutcome",
    async (event, payload: unknown) => {
      assertTrustedSender(event, getMainWindow);
      if (
        !isObject(payload) ||
        !hasOnlyKeys(payload, ["gpuId"]) ||
        !isString(payload.gpuId) ||
        !payload.gpuId.trim()
      )
        throw new Error("Invalid dismissStartupOutcome payload");
      return backend.invoke("gpu.dismissStartupOutcome", payload);
    },
  );

  ipcMain.handle(
    "prism:games:listSteamApps",
    async (event, ...args: unknown[]) => {
      assertTrustedSender(event, getMainWindow);
      if (args.length) throw new Error("Invalid listSteamApps payload");
      return backend.invoke("games.listSteamApps");
    },
  );
  ipcMain.handle(
    "prism:games:getRunning",
    async (event, ...args: unknown[]) => {
      assertTrustedSender(event, getMainWindow);
      if (args.length) throw new Error("Invalid running games payload");
      return backend.invoke("games.getRunning");
    },
  );
  ipcMain.handle(
    "prism:games:setEditHolds",
    async (event, payload: unknown) => {
      assertTrustedSender(event, getMainWindow);
      if (
        !isObject(payload) ||
        Object.keys(payload).length !== 1 ||
        !Array.isArray(payload.gpuIds) ||
        !payload.gpuIds.every(
          (id: unknown) => typeof id === "string" && id.length > 0,
        )
      ) {
        throw new Error("Invalid edit holds payload");
      }
      return backend.setEditHolds(payload.gpuIds);
    },
  );
  ipcMain.handle(
    "prism:games:getSwitchingStatus",
    async (event, ...args: unknown[]) => {
      assertTrustedSender(event, getMainWindow);
      if (args.length) throw new Error("Invalid switching status payload");
      return backend.invoke("games.getSwitchingStatus");
    },
  );
  ipcMain.handle("prism:games:get", async (event, ...args: unknown[]) => {
    assertTrustedSender(event, getMainWindow);
    if (args.length) throw new Error("Invalid get payload");
    return backend.invoke("games.get");
  });
  ipcMain.handle(
    "prism:games:addSteamGame",
    async (event, payload: unknown) => {
      assertTrustedSender(event, getMainWindow);
      if (
        !isObject(payload) ||
        !hasOnlyKeys(payload, ["appId"]) ||
        !isAppId(payload.appId)
      )
        throw new Error("Invalid addSteamGame payload");
      return backend.invoke("games.addSteamGame", payload);
    },
  );
  ipcMain.handle("prism:games:setProfile", async (event, payload: unknown) => {
    assertTrustedSender(event, getMainWindow);
    if (
      !isObject(payload) ||
      !hasOnlyKeys(payload, ["game", "gpuId", "profileId"]) ||
      !isGameId(payload.game) ||
      !isString(payload.gpuId) ||
      !payload.gpuId.trim() ||
      !(
        payload.profileId === null ||
        (isString(payload.profileId) && payload.profileId.trim())
      )
    )
      throw new Error("Invalid setProfile payload");
    return backend.invoke("games.setProfile", payload);
  });
  ipcMain.handle("prism:games:remove", async (event, payload: unknown) => {
    assertTrustedSender(event, getMainWindow);
    if (
      !isObject(payload) ||
      !hasOnlyKeys(payload, ["game"]) ||
      !isGameId(payload.game)
    )
      throw new Error("Invalid remove payload");
    return backend.invoke("games.remove", payload);
  });
  ipcMain.handle(
    "prism:games:setAutomaticSwitching",
    async (event, payload: unknown) => {
      assertTrustedSender(event, getMainWindow);
      if (
        !isObject(payload) ||
        !hasOnlyKeys(payload, ["enabled"]) ||
        typeof payload.enabled !== "boolean"
      )
        throw new Error("Invalid setAutomaticSwitching payload");
      return backend.invoke("games.setAutomaticSwitching", payload);
    },
  );

  ipcMain.handle("prism:gpu:applyProfile", async (event, payload: unknown) => {
    assertTrustedSender(event, getMainWindow);
    if (
      !isObject(payload) ||
      !isString(payload.gpuId) ||
      !payload.gpuId.trim() ||
      !hasOnlyKeys(payload, ["gpuId", "settings"]) ||
      !isObject(payload.settings) ||
      Array.isArray(payload.settings) ||
      Object.keys(payload.settings).length === 0 ||
      Object.entries(payload.settings).some(([key, setting]) =>
        key === "cooling"
          ? !isCoolingConfiguration(setting)
          : !isProfileSettingKey(key) || !isProfileSetting(setting),
      )
    ) {
      throw new Error("Invalid applyProfile payload");
    }
    return backend.invoke("gpu.applyProfile", payload);
  });

  ipcMain.handle(
    "prism:gpu:applyProfileById",
    async (event, payload: unknown) => {
      assertTrustedSender(event, getMainWindow);
      if (
        !isObject(payload) ||
        !hasOnlyKeys(payload, ["profileId"]) ||
        !isString(payload.profileId) ||
        !payload.profileId.trim()
      ) {
        throw new Error("Invalid applyProfileById payload");
      }
      return backend.invoke("gpu.applyProfileById", payload);
    },
  );

  ipcMain.handle(
    "prism:gpu:correctAndApplyProfile",
    async (event, payload: unknown) => {
      assertTrustedSender(event, getMainWindow);
      if (!validateCorrectProfilePayload(payload)) {
        throw new Error("Invalid correctAndApplyProfile payload");
      }
      return backend.invoke("gpu.correctAndApplyProfile", payload);
    },
  );

  ipcMain.handle(
    "prism:gpu:getTuningStates",
    async (event, ...args: unknown[]) => {
      assertTrustedSender(event, getMainWindow);
      if (args.length) throw new Error("Invalid getTuningStates payload");
      return backend.invoke("gpu.getTuningStates");
    },
  );

  ipcMain.handle("prism:gpu:deleteProfile", async (event, payload: unknown) => {
    assertTrustedSender(event, getMainWindow);
    if (
      !isObject(payload) ||
      !hasOnlyKeys(payload, ["profileId"]) ||
      !isString(payload.profileId) ||
      !payload.profileId.trim()
    ) {
      throw new Error("Invalid deleteProfile payload");
    }
    return backend.invoke("gpu.deleteProfile", payload);
  });

  ipcMain.handle("prism:gpu:renameProfile", async (event, payload: unknown) => {
    assertTrustedSender(event, getMainWindow);
    if (
      !isObject(payload) ||
      !hasOnlyKeys(payload, ["profileId", "name"]) ||
      !isString(payload.profileId) ||
      !payload.profileId.trim() ||
      !isString(payload.name) ||
      !payload.name.trim()
    ) {
      throw new Error("Invalid renameProfile payload");
    }
    return backend.invoke("gpu.renameProfile", payload);
  });

  ipcMain.handle("prism:gpu:saveProfile", async (event, payload: unknown) => {
    assertTrustedSender(event, getMainWindow);
    if (
      !isObject(payload) ||
      !hasOnlyKeys(payload, ["gpuId", "name", "revision"]) ||
      !isString(payload.gpuId) ||
      !payload.gpuId.trim() ||
      !isString(payload.name) ||
      !payload.name.trim() ||
      !isString(payload.revision)
    ) {
      throw new Error("Invalid saveProfile payload");
    }
    return backend.invoke("gpu.saveProfileAsNew", payload);
  });

  ipcMain.handle(
    "prism:gpu:saveProfileToCurrent",
    async (event, payload: unknown) => {
      assertTrustedSender(event, getMainWindow);
      if (
        !isObject(payload) ||
        !hasOnlyKeys(payload, ["gpuId", "revision"]) ||
        !isString(payload.gpuId) ||
        !payload.gpuId.trim() ||
        !isString(payload.revision)
      ) {
        throw new Error("Invalid saveProfileToCurrent payload");
      }
      return backend.invoke("gpu.saveProfileToCurrent", payload);
    },
  );

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

  ipcMain.handle("prism:gpu:previewFanCurve", async (event, payload) => {
    assertTrustedSender(event, getMainWindow);
    if (!validateFanCurvePreviewPayload(payload)) {
      throw new Error("Invalid previewFanCurve payload");
    }
    return backend.invoke("gpu.previewFanCurve", payload);
  });

  ipcMain.handle("prism:gpu:applyCooling", async (event, payload) => {
    assertTrustedSender(event, getMainWindow);
    if (!validateApplyCoolingPayload(payload)) {
      throw new Error("Invalid applyCooling payload");
    }
    return backend.invoke("gpu.applyCooling", payload);
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
