import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  connectedDaemonStatus,
  createPerformanceResponse,
} from "@/test/gpu-fixtures";
import {
  createTestQueryClient,
  renderWithQueryClient,
} from "@/test/query-client";
import { GpuPerformancePage } from "./gpu-performance-page";

const mocks = vi.hoisted(() => ({
  performanceQuery: {
    data: undefined as unknown,
    isPending: false,
    error: null as Error | null,
  },
  daemonStatusQuery: {
    data: undefined as unknown,
  },
}));

vi.mock("@/features/gpus/hooks/use-gpu-performance", () => ({
  useGpuPerformance: () => mocks.performanceQuery,
}));

vi.mock("@/features/gpus/hooks/use-gpu-daemon-status", () => ({
  useGpuDaemonStatus: () => mocks.daemonStatusQuery,
}));

vi.mock("@/features/gpus/api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/features/gpus/api")>()),
  resetGpuClockOffsets: vi.fn(),
  resetGpuPowerLimit: vi.fn(),
  setGpuClockOffsets: vi.fn(),
  setGpuFanModeAuto: vi.fn(),
  setGpuFanSpeed: vi.fn(),
  setGpuPowerLimit: vi.fn(),
}));

vi.mock("./gpu-performance-screen", () => ({
  GpuPerformanceScreen: vi.fn((props) => (
    <button
      type="button"
      onClick={() =>
        props.onMutationAction({
          kind: "setPowerLimit",
          gpuId: "GPU-123",
          limitWatts: 450,
        })
      }
    >
      mutate performance
    </button>
  )),
}));

describe("GpuPerformancePage", () => {
  beforeEach(() => {
    mocks.performanceQuery.data = createPerformanceResponse();
    mocks.performanceQuery.isPending = false;
    mocks.performanceQuery.error = null;
    mocks.daemonStatusQuery.data = connectedDaemonStatus;
    vi.clearAllMocks();
  });

  it("runs performance mutations and invalidates performance data", async () => {
    const api = await import("@/features/gpus/api");
    vi.mocked(api.setGpuPowerLimit).mockResolvedValue({
      success: true,
      error: null,
    });
    const queryClient = createTestQueryClient();
    const invalidateQueries = vi.spyOn(queryClient, "invalidateQueries");

    renderWithQueryClient(<GpuPerformancePage />, { queryClient });

    await userEvent.click(
      screen.getByRole("button", { name: "mutate performance" }),
    );

    await waitFor(() => {
      expect(api.setGpuPowerLimit).toHaveBeenCalledWith("GPU-123", 450);
      expect(invalidateQueries).toHaveBeenCalledWith({
        queryKey: ["gpu-performance"],
      });
    });
  });
});
