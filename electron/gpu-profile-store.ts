import { randomUUID } from "node:crypto";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import path from "node:path";

import {
  captureProfileSettings,
  PROFILE_SETTING_KEYS,
} from "@/features/gpus/lib/tuning-profiles";
import type {
  GpuPerformanceDevice,
  GpuTuningProfile,
  ProfileSettingKey,
} from "@/features/gpus/types";

// User-owned named profiles never participate in daemon startup reapplication.
export class GpuProfileStore {
  private writes: Promise<unknown> = Promise.resolve();

  constructor(private readonly filePath: string) {}

  async list(): Promise<GpuTuningProfile[]> {
    await this.writes;
    return this.read();
  }

  save(
    name: string,
    gpu: GpuPerformanceDevice,
    appliedDefaults: readonly ProfileSettingKey[] = [],
  ): Promise<GpuTuningProfile> {
    const settings = captureProfileSettings(gpu, appliedDefaults);
    const profile: GpuTuningProfile = {
      id: randomUUID(),
      name: name.trim(),
      gpuId: gpu.stableId!,
      settings,
    };
    const write = this.writes.then(async () => {
      const profiles = await this.read();
      profiles.push(profile);
      await mkdir(path.dirname(this.filePath), { recursive: true });
      const temporaryPath = `${this.filePath}.tmp`;
      await writeFile(
        temporaryPath,
        JSON.stringify({ version: 1, profiles }, null, 2),
        { mode: 0o600 },
      );
      await rename(temporaryPath, this.filePath);
      return profile;
    });
    this.writes = write.catch(() => undefined);
    return write;
  }

  private async read(): Promise<GpuTuningProfile[]> {
    let contents: string;
    try {
      contents = await readFile(this.filePath, "utf8");
    } catch (error) {
      if (error instanceof Error && "code" in error && error.code === "ENOENT")
        return [];
      throw error;
    }
    const data: unknown = JSON.parse(contents);
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
    keys.every((key) => {
      if (!(PROFILE_SETTING_KEYS as readonly string[]).includes(key))
        return false;
      const setting = settings[key];
      return (
        isRecord(setting) &&
        ((setting.kind === "default" && Object.keys(setting).length === 1) ||
          (setting.kind === "value" &&
            typeof setting.value === "number" &&
            Number.isFinite(setting.value) &&
            Object.keys(setting).length === 2))
      );
    })
  );
}
