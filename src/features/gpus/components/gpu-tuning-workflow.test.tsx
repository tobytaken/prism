import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  connectedDaemonStatus,
  createPerformanceGpu,
  createPerformanceResponse,
} from "@/test/gpu-fixtures";
import { renderWithQueryClient } from "@/test/query-client";
import { GpuTuningPage } from "./gpu-tuning-page";

describe("GPU tuning observations", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
      connectedDaemonStatus,
    );
  });

  it("distinguishes unsupported controls, probe failures, and supported controls with unreadable values", async () => {
    const response = createPerformanceResponse({
      gpus: [
        createPerformanceGpu({
          power: {
            currentLimitWatts: null,
            currentValueError: "Power read failed: GPU is lost",
          },
          clocks: {
            graphics: {
              support: "unknown",
              reason: "Offset probe failed: insufficient permission",
              currentOffsetMHz: null,
            },
          },
        }),
      ],
    });
    vi.spyOn(window.prism.gpu, "getPerformance").mockResolvedValue(response);

    renderWithQueryClient(<GpuTuningPage />);

    expect(
      await screen.findByText("Power read failed: GPU is lost"),
    ).toBeInTheDocument();
    expect(screen.getByText("Support currently unknown")).toBeInTheDocument();
    expect(
      screen.getByText("Offset probe failed: insufficient permission"),
    ).toBeInTheDocument();
    expect(screen.getByText("Not supported")).toBeInTheDocument();
    expect(
      screen.getByText("The memory clock offset is not supported on this GPU."),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Supported · current value unavailable"),
    ).toBeInTheDocument();
  });

  it("recovers support and readbacks on polling without restart and never writes an unknown offset", async () => {
    const user = userEvent.setup();
    let response = createPerformanceResponse({
      gpus: [
        createPerformanceGpu({
          power: {
            support: "unknown",
            reason: "Power probe failed: GPU lost",
            currentLimitWatts: null,
          },
          clocks: {
            graphics: {
              currentOffsetMHz: null,
              currentValueError: "Graphics read failed",
            },
            memory: {
              support: "unknown",
              reason: "Memory probe failed",
              currentOffsetMHz: null,
            },
          },
        }),
      ],
    });
    vi.spyOn(window.prism.gpu, "getPerformance").mockImplementation(async () =>
      structuredClone(response),
    );
    const setPower = vi.spyOn(window.prism.gpu, "setPowerLimit");
    const setOffsets = vi
      .spyOn(window.prism.gpu, "setClockOffsets")
      .mockImplementation(async (_id, graphics, memory) => {
        if (graphics != null)
          response.gpus[0].clocks.graphics.currentOffsetMHz = graphics;
        if (memory != null)
          response.gpus[0].clocks.memory.currentOffsetMHz = memory;
      });
    renderWithQueryClient(<GpuTuningPage />);

    expect(
      await screen.findByText("Power probe failed: GPU lost"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("spinbutton", { name: "Graphics clock offset" }),
    ).toHaveValue(null);
    expect(
      screen.getByRole("spinbutton", { name: "Graphics clock offset" }),
    ).toBeDisabled();
    expect(setPower).not.toHaveBeenCalled();
    expect(setOffsets).not.toHaveBeenCalled();

    response = createPerformanceResponse({
      gpus: [
        createPerformanceGpu({
          clocks: {
            memory: {
              support: "supported",
              currentOffsetMHz: null,
              minOffsetMHz: -500,
              maxOffsetMHz: 500,
              reason: null,
              currentValueError: "Memory read failed",
            },
          },
        }),
      ],
    });
    await waitFor(() =>
      expect(
        screen.getByRole("spinbutton", { name: "Power limit" }),
      ).toHaveValue(475),
    );
    const graphics = screen.getByRole("spinbutton", {
      name: "Graphics clock offset",
    });
    await waitFor(() => expect(graphics).toHaveValue(120));
    const memory = screen.getByRole("spinbutton", {
      name: "Memory clock offset",
    });
    expect(memory).toHaveValue(null);
    expect(memory).toBeDisabled();
    expect(
      screen.queryByText("Power probe failed: GPU lost"),
    ).not.toBeInTheDocument();

    await user.clear(graphics);
    await user.type(graphics, "150");
    const clocks = screen.getByText("Clock Offsets").closest("section")!;
    await user.click(within(clocks).getByRole("button", { name: "Apply" }));
    await waitFor(() =>
      expect(setOffsets).toHaveBeenCalledWith("GPU-123", 150, null),
    );
    expect(response.gpus[0].clocks.memory.currentOffsetMHz).toBeNull();

    response.gpus[0].clocks.memory.currentOffsetMHz = 80;
    response.gpus[0].clocks.memory.currentValueError = null;
    await waitFor(() => expect(memory).toHaveValue(80));
    expect(memory).toBeEnabled();
    expect(screen.queryByText("Memory read failed")).not.toBeInTheDocument();
  });

  it("keeps missing configured values blank instead of substituting defaults or zero", async () => {
    const response = createPerformanceResponse();
    response.gpus[0].power.currentLimitWatts = null;
    response.gpus[0].clocks.graphics.currentOffsetMHz = null;
    vi.spyOn(window.prism.gpu, "getPerformance").mockResolvedValue(response);

    renderWithQueryClient(<GpuTuningPage />);

    await screen.findByText("NVIDIA GeForce RTX 5090");
    for (const name of ["Power limit", "Graphics clock offset"]) {
      expect(screen.getByRole("spinbutton", { name })).toHaveValue(null);
      expect(screen.getByRole("spinbutton", { name })).toBeDisabled();
    }
    for (const button of screen.getAllByRole("button", { name: "Apply" })) {
      expect(button).toBeDisabled();
    }
  });
});
