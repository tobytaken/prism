import type {
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
  GpuPerformanceResponse,
  GpuTuningProfile,
  GpuTuningState,
  SaveGpuProfileRequest,
  SaveToCurrentProfileRequest,
  RenameGpuProfileRequest,
  GpuProfileIdRequest,
  StartupBehavior,
  StartupState,
  MigrationState,
  SetStartupProfileRequest,
  FanCurvePreview,
  PreviewFanCurveRequest,
} from "@/features/gpus/types";
import type {
  AddSteamGameRequest,
  AutomaticSwitchingStatus,
  GameRequest,
  GamesState,
  RunningGamesState,
  SetAutomaticSwitchingRequest,
  SetGameProfileRequest,
  SteamApp,
} from "@/features/games/types";
import type {
  DriverAction,
  DriverJob,
  DriverStatusResponse,
} from "@/features/drivers/types";

export interface PrismDesktopApi {
  gpu: {
    getMigrationState(): Promise<MigrationState>;
    adoptMigratedProfile(request: {
      profileId: string;
    }): Promise<GpuTuningProfile>;
    dismissMigrationNotice(): Promise<GpuCommandResponse>;
    getStartupState(): Promise<StartupState>;
    setStartupBehavior(request: {
      behavior: StartupBehavior;
    }): Promise<GpuCommandResponse>;
    setStartupProfile(
      request: SetStartupProfileRequest,
    ): Promise<GpuCommandResponse>;
    dismissStartupOutcome(request: {
      gpuId: string;
    }): Promise<GpuCommandResponse>;
    /** The caller's own profiles. */
    listProfiles(): Promise<GpuTuningProfile[]>;
    renameProfile(request: RenameGpuProfileRequest): Promise<GpuTuningProfile>;
    deleteProfile(request: GpuProfileIdRequest): Promise<GpuCommandResponse>;
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
    /** Checks a draft shared fan curve against the GPU's current limits; nothing is applied. */
    previewFanCurve(request: PreviewFanCurveRequest): Promise<FanCurvePreview>;
    setClockOffsets(
      gpuId: string,
      graphicsOffsetMHz: OffsetInstruction,
      memoryOffsetMHz: OffsetInstruction,
    ): Promise<ClockOffsetResponse>;
    resetClockOffsets(gpuId: string): Promise<ClockOffsetResponse>;
  };
  /** The caller's own game associations, and Steam apps from their own home. */
  games: {
    /** Fully installed Steam apps, without compatibility tools or known non-game apps. */
    listSteamApps(): Promise<SteamApp[]>;
    get(): Promise<GamesState>;
    getRunning(): Promise<RunningGamesState>;
    onRunningChanged(callback: (state: RunningGamesState) => void): () => void;
    /** The caller's automatic switching status, then pushed changes. */
    getSwitchingStatus(): Promise<AutomaticSwitchingStatus>;
    onSwitchingStatusChanged(
      callback: (status: AutomaticSwitchingStatus) => void,
    ): () => void;
    addSteamGame(request: AddSteamGameRequest): Promise<GamesState>;
    setProfile(request: SetGameProfileRequest): Promise<GamesState>;
    remove(request: GameRequest): Promise<GamesState>;
    setAutomaticSwitching(
      request: SetAutomaticSwitchingRequest,
    ): Promise<GamesState>;
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
