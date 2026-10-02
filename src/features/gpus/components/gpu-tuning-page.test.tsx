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
import { GpuTuningPage } from "./gpu-tuning-page";

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

vi.mock("@/features/gpus/api", () => ({
  listGpuProfiles: vi.fn(async () => []),
  saveGpuProfile: vi.fn(),
  applyGpuConfig: vi.fn(),
  resetGpuClockOffsets: vi.fn(),
  resetGpuPowerLimit: vi.fn(),
  setGpuClockOffsets: vi.fn(),
  setGpuFanModeAuto: vi.fn(),
  setGpuFanSpeed: vi.fn(),
  setGpuPowerLimit: vi.fn(),
}));

vi.mock("./gpu-tuning-screen", () => ({
  GpuTuningScreen: vi.fn((props) => (
    <button type="button" onClick={props.onReapplySavedSettings}>
      reapply saved
    </button>
  )),
}));

describe("GpuTuningPage", () => {
  beforeEach(() => {
    mocks.performanceQuery.data = createPerformanceResponse();
    mocks.performanceQuery.isPending = false;
    mocks.performanceQuery.error = null;
    mocks.daemonStatusQuery.data = connectedDaemonStatus;
    vi.clearAllMocks();
  });

  it("reapplies saved settings and invalidates dependent GPU queries", async () => {
    const api = await import("@/features/gpus/api");
    vi.mocked(api.applyGpuConfig).mockResolvedValue({
      success: true,
      inventory: null,
      error: null,
    });
    const queryClient = createTestQueryClient();
    const invalidateQueries = vi.spyOn(queryClient, "invalidateQueries");

    renderWithQueryClient(<GpuTuningPage />, { queryClient });

    await userEvent.click(
      screen.getByRole("button", { name: "reapply saved" }),
    );

    await waitFor(() => {
      expect(api.applyGpuConfig).toHaveBeenCalledWith({ reapplySaved: true });
      expect(invalidateQueries).toHaveBeenCalledWith({
        queryKey: ["gpu-performance"],
      });
      expect(invalidateQueries).toHaveBeenCalledWith({
        queryKey: ["gpu-daemon-status"],
      });
      expect(invalidateQueries).toHaveBeenCalledWith({
        queryKey: ["gpu-inventory"],
      });
    });
  });
});
