import { readFile, rename } from "node:fs/promises";

import {
  isProfileSetting,
  isProfileSettingKey,
} from "@/features/gpus/lib/tuning-profiles";
import type { GpuTuningProfile } from "@/features/gpus/types";

export const PROFILE_FILE_NAME = "gpu-tuning-profiles.json";
export const IMPORTED_SUFFIX = ".imported";

/**
 * The app's per-user profile file from before profiles moved into the daemon. Its profiles are
 * imported once under the caller; the file is then renamed and never written again. A failed
 * attempt leaves the file in place for the next one, and the daemon skips ids it already has.
 */
export class ProfileFileImport {
  private done = false;
  private running: Promise<void> | undefined;

  constructor(
    private readonly filePath: string,
    private readonly importProfiles: (
      profiles: GpuTuningProfile[],
    ) => Promise<unknown>,
  ) {}

  run(): Promise<void> {
    if (this.done) return Promise.resolve();
    this.running ??= this.attempt().finally(() => {
      this.running = undefined;
    });
    return this.running;
  }

  private async attempt() {
    const profiles = await readProfileFile(this.filePath);
    if (profiles) {
      await this.importProfiles(profiles);
      await rename(this.filePath, `${this.filePath}${IMPORTED_SUFFIX}`);
    }
    this.done = true;
  }
}

async function readProfileFile(
  filePath: string,
): Promise<GpuTuningProfile[] | undefined> {
  let contents: string;
  try {
    contents = await readFile(filePath, "utf8");
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT")
      return undefined;
    throw error;
  }
  let data: unknown;
  try {
    data = JSON.parse(contents);
  } catch {
    data = undefined;
  }
  if (
    !isRecord(data) ||
    data.version !== 1 ||
    !Array.isArray(data.profiles) ||
    !data.profiles.every(isProfile)
  ) {
    throw new Error(
      "The saved GPU tuning profiles could not be read. The profile file has been preserved.",
    );
  }
  return data.profiles;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isProfile(value: unknown): value is GpuTuningProfile {
  if (
    !isRecord(value) ||
    typeof value.id !== "string" ||
    !value.id.trim() ||
    typeof value.name !== "string" ||
    !value.name.trim() ||
    typeof value.gpuId !== "string" ||
    !value.gpuId.trim() ||
    !isRecord(value.settings)
  )
    return false;
  const settings = value.settings;
  const keys = Object.keys(settings);
  return (
    keys.length > 0 &&
    keys.every(
      (key) => isProfileSettingKey(key) && isProfileSetting(settings[key]),
    )
  );
}
