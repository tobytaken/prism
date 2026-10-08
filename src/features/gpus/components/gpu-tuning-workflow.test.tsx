import { MemoryRouter } from "react-router-dom";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  connectedDaemonStatus,
  createPerformanceGpu,
  createPerformanceResponse,
} from "@/test/gpu-fixtures";
import { renderWithQueryClient } from "@/test/query-client";
import { GpuPerformancePage } from "./gpu-performance-page";

describe("GPU tuning observations", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
      connectedDaemonStatus,
    );
  });

  it("preserves outstanding power and fan edits across polling and discards to refreshed values", async () => {
    const user = userEvent.setup();
    const response = createPerformanceResponse();
    vi.spyOn(window.prism.gpu, "getPerformance").mockImplementation(async () =>
      structuredClone(response),
    );
    const setPower = vi.spyOn(window.prism.gpu, "setPowerLimit");
    const setFan = vi.spyOn(window.prism.gpu, "setFanSpeed");
    renderWithQueryClient(
      <MemoryRouter>
        <GpuPerformancePage />
      </MemoryRouter>,
    );
    const power = await screen.findByRole("spinbutton", {
      name: "Power limit",
    });
    await user.clear(power);
    await user.type(power, "500");
    const fan = screen.getByRole("spinbutton", { name: "Fan 1 manual target" });
    await user.clear(fan);
    await user.type(fan, "65");
    response.gpus[0].power.currentLimitWatts = 450;
    response.gpus[0].cooling.fans[0].currentSpeedPercent = 48;
    await screen.findByText("Current limit 450 W");
    expect(power).toHaveValue(500);
    expect(fan).toHaveValue(65);
    expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Discard power limit change" }),
    );
    expect(power).toHaveValue(450);
    await user.click(
      screen.getByRole("button", { name: "Discard fan 1 change" }),
    );
    expect(fan).toHaveValue(48);
    expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
    expect(setPower).not.toHaveBeenCalled();
    expect(setFan).not.toHaveBeenCalled();
  });

  it("reports mixed clock outcomes, refreshes actual values, and retries only the unresolved offset", async () => {
    const user = userEvent.setup();
    const response = createPerformanceResponse({
      gpus: [
        createPerformanceGpu({
          clocks: {
            memory: {
              support: "supported",
              reason: null,
              currentOffsetMHz: 40,
              minOffsetMHz: -500,
              maxOffsetMHz: 500,
            },
          },
        }),
      ],
    });
    let failMemory = true;
    vi.spyOn(window.prism.gpu, "getPerformance").mockImplementation(async () =>
      structuredClone(response),
    );
    const offsets = vi
      .spyOn(window.prism.gpu, "setClockOffsets")
      .mockImplementation(async (_id, graphics, memory) => {
        if (graphics != null)
          response.gpus[0].clocks.graphics.currentOffsetMHz =
            graphics === "default" ? 0 : graphics;
        if (memory != null && !failMemory)
          response.gpus[0].clocks.memory.currentOffsetMHz =
            memory === "default" ? 0 : memory;
        return {
          graphics:
            graphics == null ? null : { status: "succeeded", error: null },
          memory:
            memory == null
              ? null
              : failMemory
                ? { status: "failed", error: "Memory write failed" }
                : { status: "succeeded", error: null },
        };
      });
    renderWithQueryClient(
      <MemoryRouter>
        <GpuPerformancePage />
      </MemoryRouter>,
    );
    const power = await screen.findByRole("spinbutton", {
      name: "Power limit",
    });
    await user.clear(power);
    await user.type(power, "500");
    const graphics = screen.getByRole("spinbutton", {
      name: "Graphics clock offset",
    });
    const memory = screen.getByRole("spinbutton", {
      name: "Memory clock offset",
    });
    await user.clear(graphics);
    await user.type(graphics, "150");
    await user.clear(memory);
    await user.type(memory, "80");
    const clocks = screen.getByText("Clock Offsets").closest("section")!;
    await user.click(within(clocks).getByRole("button", { name: "Apply" }));
    expect(
      await screen.findByText("Graphics clock offset: Applied"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Memory clock offset: Failed.*Memory write failed/),
    ).toBeInTheDocument();
    expect(screen.getByText("Current offset +150 MHz")).toBeInTheDocument();
    expect(screen.getByText("Current offset +40 MHz")).toBeInTheDocument();
    expect(memory).toHaveValue(80);
    expect(power).toHaveValue(500);
    failMemory = false;
    await user.click(within(clocks).getByRole("button", { name: "Apply" }));
    await screen.findByText("Memory clock offset: Applied");
    expect(offsets).toHaveBeenLastCalledWith("GPU-123", null, 80);
    expect(response.gpus[0].clocks.memory.currentOffsetMHz).toBe(80);
    expect(
      screen.queryByRole("button", {
        name: "Discard memory clock offset change",
      }),
    ).not.toBeInTheDocument();
    expect(power).toHaveValue(500);
  });

  it.each([{ name: "Performance", Page: GpuPerformancePage }])(
    "keeps failed power intentions while failed readback is unknown and discard never writes ($name)",
    async ({ Page }) => {
      const user = userEvent.setup();
      const response = createPerformanceResponse();
      let readbackFails = false;
      vi.spyOn(window.prism.gpu, "getPerformance").mockImplementation(
        async () => {
          if (readbackFails) throw new Error("Readback unavailable");
          return structuredClone(response);
        },
      );
      const setPower = vi
        .spyOn(window.prism.gpu, "setPowerLimit")
        .mockImplementation(async () => {
          readbackFails = true;
          throw new Error("Power write failed");
        });
      renderWithQueryClient(
        <MemoryRouter>
          <Page />
        </MemoryRouter>,
      );
      const power = await screen.findByRole("spinbutton", {
        name: "Power limit",
      });
      await user.clear(power);
      await user.type(power, "500");
      const section = power.closest("section")!;
      await user.click(within(section).getByRole("button", { name: "Apply" }));
      await screen.findByText("Current limit Unavailable");
      expect(power).toHaveValue(500);
      expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
      await user.click(
        screen.getByRole("button", { name: "Discard power limit change" }),
      );
      expect(power).toHaveValue(null);
      expect(setPower).toHaveBeenCalledTimes(1);
      readbackFails = false;
      response.gpus[0].power.currentLimitWatts = 450;
      await waitFor(() => expect(power).toHaveValue(450));
    },
  );

  it("retains failed automatic fan mode for retry without erasing another fan edit", async () => {
    const user = userEvent.setup();
    const response = createPerformanceResponse();
    const fan = response.gpus[0].cooling.fans[0];
    fan.mode = "manual";
    response.gpus[0].cooling.fans.push({ ...fan, index: 1 });
    let fail = true;
    vi.spyOn(window.prism.gpu, "getPerformance").mockImplementation(async () =>
      structuredClone(response),
    );
    const setAuto = vi
      .spyOn(window.prism.gpu, "setFanModeAuto")
      .mockImplementation(async (_gpuId, index) => {
        response.gpus[0].cooling.fans.find(
          (item) => item.index === index,
        )!.mode = "auto";
        if (fail) throw new Error("Fan acknowledgement lost");
        return { success: true, error: null };
      });
    renderWithQueryClient(
      <MemoryRouter>
        <GpuPerformancePage />
      </MemoryRouter>,
    );
    const second = await screen.findByRole("spinbutton", {
      name: "Fan 2 manual target",
    });
    await user.clear(second);
    await user.type(second, "70");
    await user.click(
      screen.getAllByRole("button", { name: "Restore Auto" })[0],
    );
    await screen.findByText("Driver reports auto mode");
    const retry = await screen.findByRole("button", {
      name: "Retry fan 1 change",
    });
    expect(second).toHaveValue(70);
    fail = false;
    await user.click(retry);
    await screen.findByText("Fan 1: Applied");
    expect(setAuto).toHaveBeenLastCalledWith("GPU-123", 0);
    expect(second).toHaveValue(70);
    expect(
      screen.getByRole("button", { name: "Discard fan 2 change" }),
    ).toBeInTheDocument();
  });

  it("keeps edits with their GPU and fan when identical-model devices disappear or reorder", async () => {
    const user = userEvent.setup();
    const first = createPerformanceGpu();
    first.cooling.fans.push({ ...first.cooling.fans[0], index: 1 });
    const second = createPerformanceGpu({
      id: "GPU-456",
      busId: "0000:02:00.0",
    });
    const response = createPerformanceResponse({ gpus: [first, second] });
    vi.spyOn(window.prism.gpu, "getPerformance").mockImplementation(async () =>
      structuredClone(response),
    );
    renderWithQueryClient(
      <MemoryRouter>
        <GpuPerformancePage />
      </MemoryRouter>,
    );
    await screen.findAllByRole("spinbutton", { name: "Power limit" });
    const firstPower = screen.getAllByRole("spinbutton", {
      name: "Power limit",
    })[0];
    await user.clear(firstPower);
    await user.type(firstPower, "500");
    const secondFan = screen.getByRole("spinbutton", {
      name: "Fan 2 manual target",
    });
    await user.clear(secondFan);
    await user.type(secondFan, "70");
    response.gpus = [];
    await screen.findByText("No supported GPUs detected");
    expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
    response.gpus = [second];
    await screen.findByText("0000:02:00.0");
    expect(screen.getByRole("spinbutton", { name: "Power limit" })).toHaveValue(
      475,
    );
    first.cooling.fans.reverse();
    response.gpus = [second, first];
    await screen.findByText("0000:01:00.0");
    expect(
      screen.getAllByRole("spinbutton", { name: "Power limit" })[1],
    ).toHaveValue(500);
    expect(
      screen.getByRole("spinbutton", { name: "Fan 2 manual target" }),
    ).toHaveValue(70);
    expect(
      screen.getAllByRole("spinbutton", { name: "Fan 1 manual target" })[1],
    ).toHaveValue(42);
  });

  it("shows unattempted memory separately and preserves it while retrying graphics", async () => {
    const user = userEvent.setup();
    const response = createPerformanceResponse({
      gpus: [
        createPerformanceGpu({
          clocks: {
            memory: {
              support: "supported",
              reason: null,
              currentOffsetMHz: 40,
              minOffsetMHz: -500,
              maxOffsetMHz: 500,
            },
          },
        }),
      ],
    });
    let failGraphics = true;
    vi.spyOn(window.prism.gpu, "getPerformance").mockImplementation(async () =>
      structuredClone(response),
    );
    vi.spyOn(window.prism.gpu, "setClockOffsets").mockImplementation(
      async (_gpuId, graphics, memory) => {
        if (failGraphics)
          return {
            graphics: { status: "failed", error: "Graphics rejected" },
            memory: { status: "unattempted", error: "Earlier failure" },
          };
        if (graphics != null)
          response.gpus[0].clocks.graphics.currentOffsetMHz =
            graphics === "default" ? 0 : graphics;
        if (memory != null)
          response.gpus[0].clocks.memory.currentOffsetMHz =
            memory === "default" ? 0 : memory;
        return {
          graphics:
            graphics == null ? null : { status: "succeeded", error: null },
          memory: memory == null ? null : { status: "succeeded", error: null },
        };
      },
    );
    renderWithQueryClient(
      <MemoryRouter>
        <GpuPerformancePage />
      </MemoryRouter>,
    );
    const graphics = await screen.findByRole("spinbutton", {
      name: "Graphics clock offset",
    });
    const memory = screen.getByRole("spinbutton", {
      name: "Memory clock offset",
    });
    await user.clear(graphics);
    await user.type(graphics, "150");
    await user.clear(memory);
    await user.type(memory, "80");
    const clocks = screen.getByText("Clock Offsets").closest("section")!;
    await user.click(within(clocks).getByRole("button", { name: "Apply" }));
    await screen.findByText(/Memory clock offset: Not attempted/);
    expect(
      screen.getByText(/Graphics clock offset: Failed/),
    ).toBeInTheDocument();
    failGraphics = false;
    await user.click(
      screen.getByRole("button", {
        name: "Retry graphics clock offset change",
      }),
    );
    await screen.findByText("Graphics clock offset: Applied");
    expect(memory).toHaveValue(80);
    expect(screen.getByText("Current offset +40 MHz")).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", {
        name: "Discard memory clock offset change",
      }),
    );
    expect(memory).toHaveValue(40);
    expect(response.gpus[0].clocks.memory.currentOffsetMHz).toBe(40);
    expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
  });

  it("refreshes power after success while preserving a manual fan intention", async () => {
    const user = userEvent.setup();
    const response = createPerformanceResponse();
    vi.spyOn(window.prism.gpu, "getPerformance").mockImplementation(async () =>
      structuredClone(response),
    );
    vi.spyOn(window.prism.gpu, "setPowerLimit").mockImplementation(
      async (_id, watts) => {
        response.gpus[0].power.currentLimitWatts = watts;
        return { success: true, error: null };
      },
    );
    vi.spyOn(window.prism.gpu, "setFanSpeed").mockImplementation(
      async (_id, index, speed) => {
        const fan = response.gpus[0].cooling.fans.find(
          (item) => item.index === index,
        )!;
        fan.currentSpeedPercent = speed;
        fan.mode = "manual";
        return { success: true, error: null };
      },
    );
    renderWithQueryClient(
      <MemoryRouter>
        <GpuPerformancePage />
      </MemoryRouter>,
    );
    const power = await screen.findByRole("spinbutton", {
      name: "Power limit",
    });
    await user.clear(power);
    await user.type(power, "500");
    const fan = screen.getByRole("spinbutton", { name: "Fan 1 manual target" });
    await user.clear(fan);
    await user.type(fan, "65");
    await user.click(
      within(power.closest("section")!).getByRole("button", { name: "Apply" }),
    );
    await screen.findByText("Power limit: Applied");
    expect(screen.getByText("Current limit 500 W")).toBeInTheDocument();
    expect(fan).toHaveValue(65);
    await user.click(screen.getByRole("button", { name: "Set Manual" }));
    await screen.findByText("Fan 1: Applied");
    expect(screen.getByText("Driver reports manual mode")).toBeInTheDocument();
    expect(screen.getByText("Current speed 65%")).toBeInTheDocument();
    expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
  });

  it("reports acknowledged power refusal as Failed and keeps it for retry", async () => {
    const user = userEvent.setup();
    vi.spyOn(window.prism.gpu, "getPerformance").mockResolvedValue(
      createPerformanceResponse(),
    );
    vi.spyOn(window.prism.gpu, "setPowerLimit").mockResolvedValue({
      success: false,
      error: "Power denied",
    });
    renderWithQueryClient(
      <MemoryRouter>
        <GpuPerformancePage />
      </MemoryRouter>,
    );
    const power = await screen.findByRole("spinbutton", {
      name: "Power limit",
    });
    await user.clear(power);
    await user.type(power, "500");
    await user.click(
      within(power.closest("section")!).getByRole("button", { name: "Apply" }),
    );
    await screen.findByText("Power limit: Failed — Power denied");
    expect(power).toHaveValue(500);
    expect(
      screen.getByRole("button", { name: "Retry power limit change" }),
    ).toBeEnabled();
  });

  it("preserves Performance tuning edits through collapsed controls and temporary GPU absence", async () => {
    const user = userEvent.setup();
    const gpu = createPerformanceGpu();
    const response = createPerformanceResponse({ gpus: [gpu] });
    vi.spyOn(window.prism.gpu, "getPerformance").mockImplementation(async () =>
      structuredClone(response),
    );
    renderWithQueryClient(
      <MemoryRouter>
        <GpuPerformancePage />
      </MemoryRouter>,
    );
    const power = await screen.findByRole("spinbutton", {
      name: "Power limit",
    });
    await user.clear(power);
    await user.type(power, "500");
    await user.click(screen.getByRole("button", { name: /Overclocking/ }));
    expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Overclocking/ }));
    expect(screen.getByRole("spinbutton", { name: "Power limit" })).toHaveValue(
      500,
    );
    response.gpus = [];
    await screen.findByText("No supported GPUs detected");
    response.gpus = [gpu];
    expect(
      await screen.findByRole("spinbutton", { name: "Power limit" }),
    ).toHaveValue(500);
  });

  it("reports a confirmed manual fan failure and preserves its target for retry", async () => {
    const user = userEvent.setup();
    const response = createPerformanceResponse();
    vi.spyOn(window.prism.gpu, "getPerformance").mockImplementation(async () =>
      structuredClone(response),
    );
    let fail = true;
    vi.spyOn(window.prism.gpu, "setFanSpeed").mockImplementation(
      async (_gpuId, index, speed) => {
        if (fail) return { success: false, error: "Fan denied" };
        const fan = response.gpus[0].cooling.fans.find(
          (item) => item.index === index,
        )!;
        fan.currentSpeedPercent = speed;
        fan.mode = "manual";
        return { success: true, error: null };
      },
    );
    renderWithQueryClient(
      <MemoryRouter>
        <GpuPerformancePage />
      </MemoryRouter>,
    );
    const fan = await screen.findByRole("spinbutton", {
      name: "Fan 1 manual target",
    });
    await user.clear(fan);
    await user.type(fan, "65");
    await user.click(screen.getByRole("button", { name: "Set Manual" }));
    await screen.findByText("Fan 1: Failed — Fan denied");
    expect(fan).toHaveValue(65);
    expect(screen.getByText("Current speed 42%")).toBeInTheDocument();
    fail = false;
    await user.click(
      screen.getByRole("button", { name: "Retry fan 1 change" }),
    );
    await screen.findByText("Fan 1: Applied");
    expect(screen.getByText("Current speed 65%")).toBeInTheDocument();
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

    renderWithQueryClient(
      <MemoryRouter>
        <GpuPerformancePage />
      </MemoryRouter>,
    );

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
          response.gpus[0].clocks.graphics.currentOffsetMHz =
            graphics === "default" ? 0 : graphics;
        if (memory != null)
          response.gpus[0].clocks.memory.currentOffsetMHz =
            memory === "default" ? 0 : memory;
        return {
          graphics:
            graphics == null ? null : { status: "succeeded", error: null },
          memory: memory == null ? null : { status: "succeeded", error: null },
        };
      });
    renderWithQueryClient(
      <MemoryRouter>
        <GpuPerformancePage />
      </MemoryRouter>,
    );

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

    renderWithQueryClient(
      <MemoryRouter>
        <GpuPerformancePage />
      </MemoryRouter>,
    );

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
