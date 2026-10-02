import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  connectedDaemonStatus,
  createPerformanceGpu,
  createPerformanceResponse,
} from "@/test/gpu-fixtures";
import type {
  GpuPerformanceResponse,
  TuningOutcome,
} from "@/features/gpus/types";
import { renderWithQueryClient } from "@/test/query-client";
import { GpuTuningPage } from "./gpu-tuning-page";

describe("Apply all tuning changes", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
      connectedDaemonStatus,
    );
  });

  it("applies outstanding power, clock, and fan edits without a selected profile", async () => {
    const user = userEvent.setup();
    const response = createPerformanceResponse();
    const fake = fakeDesktopGpuApi(response);
    renderWithQueryClient(<GpuTuningPage />);
    const power = await screen.findByRole("spinbutton", {
      name: "Power limit",
    });
    const applyAll = screen.getByRole("button", { name: "Apply all" });
    expect(applyAll).toBeDisabled();
    await user.clear(power);
    await user.type(power, "500");
    const graphics = screen.getByRole("spinbutton", {
      name: "Graphics clock offset",
    });
    await user.clear(graphics);
    await user.type(graphics, "150");
    const fan = screen.getByRole("spinbutton", { name: "Fan 1 manual target" });
    await user.clear(fan);
    await user.type(fan, "65");
    await user.click(applyAll);

    await screen.findByText("Fan 1: Applied");
    expect(screen.getByText("Power limit: Applied")).toBeInTheDocument();
    expect(
      screen.getByText("Graphics clock offset: Applied"),
    ).toBeInTheDocument();
    expect(screen.getByText("Current limit 500 W")).toBeInTheDocument();
    expect(screen.getByText("Current offset +150 MHz")).toBeInTheDocument();
    expect(screen.getByText("Current speed 65%")).toBeInTheDocument();
    expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
    await waitFor(() => expect(applyAll).toBeDisabled());
    expect(fake.applyConfig).not.toHaveBeenCalled();
  });

  it("retains failed and unattempted clock edits for retry or discard", async () => {
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
    const fake = fakeDesktopGpuApi(response);
    fake.failures.set("GPU-123:graphics", {
      status: "failed",
      error: "Graphics denied",
    });
    renderWithQueryClient(<GpuTuningPage />);
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
    await user.click(screen.getByRole("button", { name: "Apply all" }));
    await screen.findByText("Graphics clock offset: Failed — Graphics denied");
    expect(
      screen.getByText(/Memory clock offset: Not attempted/),
    ).toBeInTheDocument();
    expect(response.gpus[0].clocks.memory.currentOffsetMHz).toBe(40);
    expect(memory).toHaveValue(80);
    await user.click(
      screen.getByRole("button", {
        name: "Discard memory clock offset change",
      }),
    );
    expect(memory).toHaveValue(40);
    fake.failures.clear();
    await user.click(screen.getByRole("button", { name: "Apply all" }));
    await screen.findByText("Graphics clock offset: Applied");
    expect(response.gpus[0].clocks.graphics.currentOffsetMHz).toBe(150);
    expect(response.gpus[0].clocks.memory.currentOffsetMHz).toBe(40);
    expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
  });

  it("keeps only a failed fan unapplied through polling, retry, and discard", async () => {
    const user = userEvent.setup();
    const response = createPerformanceResponse();
    const fake = fakeDesktopGpuApi(response);
    fake.failures.set("GPU-123:fan:0", {
      status: "failed",
      error: "Fan denied",
    });
    renderWithQueryClient(<GpuTuningPage />);
    const power = await screen.findByRole("spinbutton", {
      name: "Power limit",
    });
    await user.clear(power);
    await user.type(power, "500");
    const graphics = screen.getByRole("spinbutton", {
      name: "Graphics clock offset",
    });
    await user.clear(graphics);
    await user.type(graphics, "150");
    const fan = screen.getByRole("spinbutton", { name: "Fan 1 manual target" });
    await user.clear(fan);
    await user.type(fan, "65");
    await user.click(screen.getByRole("button", { name: "Apply all" }));
    await screen.findByText("Fan 1: Failed — Fan denied");
    expect(screen.getByText("Power limit: Applied")).toBeInTheDocument();
    expect(
      screen.getByText("Graphics clock offset: Applied"),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Discard power limit change" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", {
        name: "Discard graphics clock offset change",
      }),
    ).not.toBeInTheDocument();
    response.gpus[0].cooling.fans[0].currentSpeedPercent = 48;
    await screen.findByText("Current speed 48%");
    expect(fan).toHaveValue(65);
    fake.failures.clear();
    await user.click(screen.getByRole("button", { name: "Apply all" }));
    await screen.findByText("Fan 1: Applied");
    expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
    expect(fake.setPower).toHaveBeenCalledTimes(1);
    expect(fake.setOffsets).toHaveBeenCalledTimes(1);
    fake.failures.set("GPU-123:fan:0", {
      status: "failed",
      error: "Fan denied",
    });
    await user.clear(fan);
    await user.type(fan, "70");
    await user.click(screen.getByRole("button", { name: "Apply all" }));
    await screen.findByText("Fan 1: Failed — Fan denied");
    await user.click(
      screen.getByRole("button", { name: "Discard fan 1 change" }),
    );
    expect(fan).toHaveValue(65);
    expect(response.gpus[0].cooling.fans[0].currentSpeedPercent).toBe(65);
    expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
  });

  it("reports an invalid edit as unattempted while applying other valid edits", async () => {
    const user = userEvent.setup();
    const response = createPerformanceResponse();
    const fake = fakeDesktopGpuApi(response);
    renderWithQueryClient(<GpuTuningPage />);
    const power = await screen.findByRole("spinbutton", {
      name: "Power limit",
    });
    await user.clear(power);
    await user.type(power, "700");
    const fan = screen.getByRole("spinbutton", { name: "Fan 1 manual target" });
    await user.clear(fan);
    await user.type(fan, "65");
    await user.click(screen.getByRole("button", { name: "Apply all" }));
    await screen.findByText("Fan 1: Applied");
    expect(screen.getByText(/Power limit: Not attempted/)).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Retry power limit change" }),
    ).toBeDisabled();
    expect(power).toHaveValue(700);
    expect(response.gpus[0].power.currentLimitWatts).toBe(475);
    expect(fake.setPower).not.toHaveBeenCalled();
    await user.click(
      screen.getByRole("button", { name: "Discard power limit change" }),
    );
    expect(power).toHaveValue(475);
  });

  it("locks Apply all, individual controls, and saved reapplication until the batch settles", async () => {
    const user = userEvent.setup();
    const response = createPerformanceResponse();
    const fake = fakeDesktopGpuApi(response);
    let finishPower!: () => void;
    const powerGate = new Promise<void>((resolve) => {
      finishPower = resolve;
    });
    fake.setPower.mockImplementation(async (id, watts) => {
      await powerGate;
      response.gpus.find((gpu) => gpu.id === id)!.power.currentLimitWatts =
        watts;
      return { success: true, error: null };
    });
    renderWithQueryClient(<GpuTuningPage />);
    const power = await screen.findByRole("spinbutton", {
      name: "Power limit",
    });
    await user.clear(power);
    await user.type(power, "500");
    const fan = screen.getByRole("spinbutton", { name: "Fan 1 manual target" });
    await user.clear(fan);
    await user.type(fan, "65");
    const applyAll = screen.getByRole("button", { name: "Apply all" });
    await user.click(applyAll);
    expect(applyAll).toBeDisabled();
    expect(power).toBeDisabled();
    expect(fan).toBeDisabled();
    expect(screen.getByRole("button", { name: "Set Manual" })).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Discard fan 1 change" }),
    ).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Reapply Saved Settings" }),
    ).toBeDisabled();
    await user.click(applyAll);
    await user.click(screen.getByRole("button", { name: "Set Manual" }));
    expect(fake.setPower).toHaveBeenCalledTimes(1);
    expect(fake.setFan).not.toHaveBeenCalled();
    finishPower();
    await screen.findByText("Fan 1: Applied");
    expect(fake.setFan).toHaveBeenCalledTimes(1);
    expect(fake.applyConfig).not.toHaveBeenCalled();
    await waitFor(() => expect(power).toBeEnabled());
  });

  it("retains absent GPU edits and targets the correct GPU and fan after reordering", async () => {
    const user = userEvent.setup();
    const first = createPerformanceGpu();
    const second = createPerformanceGpu({
      id: "GPU-456",
      busId: "0000:02:00.0",
    });
    second.cooling.fans.push({ ...second.cooling.fans[0], index: 1 });
    const untouched = createPerformanceGpu({
      id: "GPU-789",
      busId: "0000:03:00.0",
    });
    const response = createPerformanceResponse({
      gpus: [first, second, untouched],
    });
    const fake = fakeDesktopGpuApi(response);
    renderWithQueryClient(<GpuTuningPage />);
    const powers = await screen.findAllByRole("spinbutton", {
      name: "Power limit",
    });
    await user.clear(powers[0]);
    await user.type(powers[0], "500");
    const secondFan = screen.getByRole("spinbutton", {
      name: "Fan 2 manual target",
    });
    await user.clear(secondFan);
    await user.type(secondFan, "70");
    second.cooling.fans.reverse();
    response.gpus = [untouched, second];
    await screen.findByText(/Edits for GPU-123 are retained/);
    await user.click(screen.getByRole("button", { name: "Apply all" }));
    await screen.findByText("Fan 2: Applied");
    expect(
      second.cooling.fans.find((fan) => fan.index === 1)!.currentSpeedPercent,
    ).toBe(70);
    expect(
      second.cooling.fans.find((fan) => fan.index === 0)!.currentSpeedPercent,
    ).toBe(42);
    expect(untouched.power.currentLimitWatts).toBe(475);
    expect(fake.setPower).not.toHaveBeenCalled();
    expect(fake.setOffsets).not.toHaveBeenCalled();
    response.gpus = [untouched, second, first];
    await screen.findByText("0000:01:00.0");
    expect(screen.getByText(/Power limit: Not attempted/)).toBeInTheDocument();
    expect(
      screen.getAllByRole("spinbutton", { name: "Power limit" })[2],
    ).toHaveValue(500);
    await user.click(screen.getByRole("button", { name: "Apply all" }));
    await screen.findByText("Power limit: Applied");
    expect(first.power.currentLimitWatts).toBe(500);
    expect(second.power.currentLimitWatts).toBe(475);
    expect(untouched.power.currentLimitWatts).toBe(475);
    expect(fake.setFan).toHaveBeenCalledTimes(1);
    expect(fake.applyConfig).not.toHaveBeenCalled();
  });

  it("keeps a lost acknowledgement unknown and continues applying independent edits", async () => {
    const user = userEvent.setup();
    const response = createPerformanceResponse();
    const fake = fakeDesktopGpuApi(response);
    fake.setPower.mockImplementation(async (id, watts) => {
      response.gpus.find((gpu) => gpu.id === id)!.power.currentLimitWatts =
        watts;
      throw new Error("Acknowledgement lost");
    });
    renderWithQueryClient(<GpuTuningPage />);
    const power = await screen.findByRole("spinbutton", {
      name: "Power limit",
    });
    await user.clear(power);
    await user.type(power, "500");
    const fan = screen.getByRole("spinbutton", { name: "Fan 1 manual target" });
    await user.clear(fan);
    await user.type(fan, "65");
    await user.click(screen.getByRole("button", { name: "Apply all" }));
    await screen.findByText("Fan 1: Applied");
    expect(
      screen.getByText("Power limit: Outcome unknown — Acknowledgement lost"),
    ).toBeInTheDocument();
    expect(screen.getByText("Current limit 500 W")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Discard power limit change" }),
    ).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Discard power limit change" }),
    );
    expect(power).toHaveValue(500);
    expect(fake.setPower).toHaveBeenCalledTimes(1);
    expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
  });

  it("retries retained power default and automatic fan-mode intentions", async () => {
    const user = userEvent.setup();
    const response = createPerformanceResponse();
    response.gpus[0].cooling.fans[0].mode = "manual";
    const fake = fakeDesktopGpuApi(response);
    let fail = true;
    const resetPower = vi
      .spyOn(window.prism.gpu, "resetPowerLimit")
      .mockImplementation(async (id) => {
        if (fail) return { success: false, error: "Power denied" };
        const gpu = response.gpus.find((gpu) => gpu.id === id)!;
        gpu.power.currentLimitWatts = gpu.power.defaultLimitWatts;
        return { success: true, error: null };
      });
    const setAuto = vi
      .spyOn(window.prism.gpu, "setFanModeAuto")
      .mockImplementation(async (id, index) => {
        if (fail) return { success: false, error: "Fan denied" };
        response.gpus
          .find((gpu) => gpu.id === id)!
          .cooling.fans.find((fan) => fan.index === index)!.mode = "auto";
        return { success: true, error: null };
      });
    renderWithQueryClient(<GpuTuningPage />);
    const power = await screen.findByRole("spinbutton", {
      name: "Power limit",
    });
    await user.click(screen.getAllByRole("button", { name: "Reset" })[0]);
    await screen.findByText("Power limit: Failed — Power denied");
    await user.click(screen.getByRole("button", { name: "Restore Auto" }));
    await screen.findByText("Fan 1: Failed — Fan denied");
    response.gpus[0].power.defaultLimitWatts = 550;
    await waitFor(() => expect(power).toHaveValue(550));
    fail = false;
    await user.click(screen.getByRole("button", { name: "Apply all" }));
    await screen.findByText("Fan 1: Applied");
    expect(screen.getByText("Power limit: Applied")).toBeInTheDocument();
    expect(screen.getByText("Current limit 550 W")).toBeInTheDocument();
    expect(screen.getByText("Auto mode")).toBeInTheDocument();
    expect(fake.setPower).not.toHaveBeenCalled();
    expect(fake.setFan).not.toHaveBeenCalled();
    expect(resetPower).toHaveBeenCalledTimes(2);
    expect(setAuto).toHaveBeenCalledTimes(2);
    expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
  });
});

function fakeDesktopGpuApi(response: GpuPerformanceResponse) {
  const failures = new Map<string, TuningOutcome>();
  vi.spyOn(window.prism.gpu, "getPerformance").mockImplementation(async () =>
    structuredClone(response),
  );
  const setPower = vi
    .spyOn(window.prism.gpu, "setPowerLimit")
    .mockImplementation(async (id, watts) => {
      const failure = failures.get(`${id}:power`);
      if (failure) return { success: false, error: failure.error };
      response.gpus.find((gpu) => gpu.id === id)!.power.currentLimitWatts =
        watts;
      return { success: true, error: null };
    });
  const setOffsets = vi
    .spyOn(window.prism.gpu, "setClockOffsets")
    .mockImplementation(async (id, graphics, memory) => {
      const gpu = response.gpus.find((gpu) => gpu.id === id)!;
      const graphicsOutcome =
        graphics == null
          ? null
          : (failures.get(`${id}:graphics`) ?? {
              status: "succeeded" as const,
              error: null,
            });
      const memoryOutcome =
        memory == null
          ? null
          : graphicsOutcome?.status === "failed"
            ? {
                status: "unattempted" as const,
                error: "Earlier graphics failure",
              }
            : (failures.get(`${id}:memory`) ?? {
                status: "succeeded" as const,
                error: null,
              });
      if (graphics != null && graphicsOutcome?.status === "succeeded")
        gpu.clocks.graphics.currentOffsetMHz = graphics;
      if (memory != null && memoryOutcome?.status === "succeeded")
        gpu.clocks.memory.currentOffsetMHz = memory;
      return { graphics: graphicsOutcome, memory: memoryOutcome };
    });
  const setFan = vi
    .spyOn(window.prism.gpu, "setFanSpeed")
    .mockImplementation(async (id, index, speed) => {
      const failure = failures.get(`${id}:fan:${index}`);
      if (failure) return { success: false, error: failure.error };
      const fan = response.gpus
        .find((gpu) => gpu.id === id)!
        .cooling.fans.find((fan) => fan.index === index)!;
      fan.currentSpeedPercent = speed;
      fan.mode = "manual";
      return { success: true, error: null };
    });
  const applyConfig = vi.spyOn(window.prism.gpu, "applyConfig");
  return { failures, setPower, setOffsets, setFan, applyConfig };
}
