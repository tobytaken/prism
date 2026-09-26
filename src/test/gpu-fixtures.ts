import type {
  GpuDaemonStatus,
  GpuDevice,
  GpuInventoryResponse,
  GpuPerformanceDevice,
  GpuPerformanceResponse,
} from "@/features/gpus/types";

type DeepPartial<T> = {
  [K in keyof T]?: T[K] extends Array<infer U>
    ? Array<DeepPartial<U>>
    : T[K] extends object
      ? DeepPartial<T[K]>
      : T[K];
};

export const connectedDaemonStatus: GpuDaemonStatus = {
  reachable: true,
  managedByDaemon: true,
  socketPath: "/run/prism/prismd.sock",
  message: null,
};

export const disconnectedDaemonStatus: GpuDaemonStatus = {
  reachable: false,
  managedByDaemon: false,
  socketPath: "/run/prism/prismd.sock",
  message: "Prism daemon is unavailable.",
};

export function createInventoryGpu(
  overrides: DeepPartial<GpuDevice> = {},
): GpuDevice {
  return mergeDeep(
    {
      id: "GPU-123",
      vendor: "nvidia",
      name: "NVIDIA GeForce RTX 5090",
      architecture: "Blackwell",
      driverVersion: "595.58.03",
      firmwareVersion: "98.02.2E.00.03",
      busId: "0000:01:00.0",
      memory: {
        totalMiB: 32_768,
      },
      compute: {
        api: "cuda",
        apiVersion: "13.2",
        capability: "12.0",
        coreCount: 21_760,
      },
      pcie: {
        genMax: 5,
        widthMax: 16,
      },
      power: {
        enforcedLimitWatts: 475,
        defaultLimitWatts: 575,
      },
      clocks: {
        graphicsCurrentMHz: 1_365,
        graphicsMaxMHz: 3_300,
        smMaxMHz: 3_300,
        memoryMaxMHz: 14_001,
      },
      control: {
        managedByDaemon: true,
        daemonReachable: true,
        supportedFeatures: {
          powerLimit: true,
          gpuClockLock: true,
          memoryClockLock: true,
          graphicsOffset: true,
          memoryOffset: true,
          reset: true,
        },
        powerLimit: {
          currentWatts: 475,
          defaultWatts: 575,
          minWatts: 300,
          maxWatts: 575,
          desiredWatts: 450,
        },
        gpuClockLock: {
          currentMinMHz: 1_800,
          currentMaxMHz: 2_400,
          minAllowedMHz: 1_500,
          maxAllowedMHz: 3_000,
          supported: true,
        },
        memoryClockLock: {
          currentMinMHz: 1_000,
          currentMaxMHz: 1_100,
          minAllowedMHz: 800,
          maxAllowedMHz: 1_200,
          supported: true,
        },
        offsets: {
          graphicsMHz: 100,
          memoryMHz: 500,
          minGraphicsMHz: -200,
          maxGraphicsMHz: 200,
          minMemoryMHz: -1_000,
          maxMemoryMHz: 1_000,
        },
        warnings: [],
      },
    } satisfies GpuDevice,
    overrides,
  );
}

export function createInventoryResponse(
  overrides: DeepPartial<GpuInventoryResponse> = {},
): GpuInventoryResponse {
  return mergeDeep(
    {
      gpus: [createInventoryGpu()],
      warnings: [],
    } satisfies GpuInventoryResponse,
    overrides,
  );
}

export function createPerformanceGpu(
  overrides: DeepPartial<GpuPerformanceDevice> = {},
): GpuPerformanceDevice {
  return mergeDeep(
    {
      id: "GPU-123",
      vendor: "nvidia",
      name: "NVIDIA GeForce RTX 5090",
      architecture: "Blackwell",
      busId: "0000:01:00.0",
      power: {
        support: "supported",
        reason: null,
        currentValueError: null,
        currentDrawWatts: 312,
        currentLimitWatts: 475,
        defaultLimitWatts: 575,
        enforcedLimitWatts: 475,
        minLimitWatts: 400,
        maxLimitWatts: 600,
      },
      cooling: {
        isSupported: true,
        unsupportedReason: null,
        fanCount: 1,
        fans: [
          {
            index: 0,
            currentSpeedPercent: 42,
            minSpeedPercent: 30,
            maxSpeedPercent: 100,
            mode: "auto",
          },
        ],
      },
      clocks: {
        graphics: {
          support: "supported",
          reason: null,
          currentValueError: null,
          currentOffsetMHz: 120,
          minOffsetMHz: -200,
          maxOffsetMHz: 200,
        },
        memory: {
          support: "unsupported",
          currentValueError: null,
          reason: "The memory clock offset is not supported on this GPU.",
          currentOffsetMHz: null,
          minOffsetMHz: null,
          maxOffsetMHz: null,
        },
      },
      telemetry: {
        powerDrawWatts: 312,
        coreClockMHz: 2_550,
        memoryClockMHz: 14_001,
        fanSpeedPercent: 42,
        voltageMillivolts: null,
        temperatureC: 71,
        gpuUtilizationPercent: 96,
        memoryUsedMiB: 12_288,
        memoryTotalMiB: 32_768,
        performanceState: "P0",
      },
    } satisfies GpuPerformanceDevice,
    overrides,
  );
}

export function createPerformanceResponse(
  overrides: DeepPartial<GpuPerformanceResponse> = {},
): GpuPerformanceResponse {
  return mergeDeep(
    {
      gpus: [createPerformanceGpu()],
      warnings: [],
    } satisfies GpuPerformanceResponse,
    overrides,
  );
}

function mergeDeep<T>(base: T, overrides: DeepPartial<T>): T {
  if (!isPlainObject(base) || !isPlainObject(overrides)) {
    return overrides === undefined ? base : (overrides as T);
  }

  const merged = { ...base } as Record<string, unknown>;

  for (const [key, value] of Object.entries(overrides)) {
    if (value === undefined) {
      continue;
    }

    const current = merged[key];
    merged[key] =
      isPlainObject(current) && isPlainObject(value)
        ? mergeDeep(current, value)
        : value;
  }

  return merged as T;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
