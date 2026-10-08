import { MemoryRouter } from "react-router-dom";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { createFakeDesktop, type FakeDesktop } from "@/test/fake-desktop";
import {
  connectedDaemonStatus,
  createPerformanceGpu,
  createPerformanceResponse,
} from "@/test/gpu-fixtures";
import { renderWithQueryClient } from "@/test/query-client";
import type { CoolingInfo } from "@/features/gpus/types";
import { GpuPerformancePage } from "./gpu-performance-page";

const starter = [
  { temperatureC: 45, speedPercent: 20 },
  { temperatureC: 65, speedPercent: 60 },
  { temperatureC: 75, speedPercent: 80 },
  { temperatureC: 85, speedPercent: 100 },
];

/** Two fans sharing a 20-100% range under a 90°C maximum operating temperature. */
const sharedCooling: CoolingInfo = {
  isSupported: true,
  unsupportedReason: null,
  fanCount: 2,
  fans: [0, 1].map((index) => ({
    index,
    currentSpeedPercent: 40,
    minSpeedPercent: 20,
    maxSpeedPercent: 100,
    mode: "auto" as const,
  })),
  maxOperatingTemperatureC: 90,
  maxOperatingTemperatureReason: null,
  curveSupport: {
    status: "available",
    maxOperatingTemperatureC: 90,
    fullSpeedTemperatureC: 85,
    minSpeedPercent: 20,
    maxSpeedPercent: 100,
    starter,
  },
};

function renderPerformance(cooling: CoolingInfo = sharedCooling) {
  const fake = createFakeDesktop({
    response: createPerformanceResponse({
      gpus: [createPerformanceGpu({ cooling: structuredClone(cooling) })],
    }),
  });
  renderWithQueryClient(
    <MemoryRouter>
      <GpuPerformancePage />
    </MemoryRouter>,
  );
  return fake;
}

const curveEditor = () =>
  screen.findByRole("group", { name: "Fan curve preview" });

/** Each point's temperature and speed inputs, in order. */
function pointValues(editor: HTMLElement) {
  return within(editor)
    .getAllByRole("spinbutton", { name: /^Point \d+ temperature/ })
    .map((temperature, index) => [
      (temperature as HTMLInputElement).valueAsNumber,
      within(editor).getByRole<HTMLInputElement>("spinbutton", {
        name: `Point ${index + 1} speed (%)`,
      }).valueAsNumber,
    ]);
}

function expectNoFanWrites(fake: FakeDesktop) {
  expect(fake.setFanSpeed).not.toHaveBeenCalled();
  expect(fake.setFanModeAuto).not.toHaveBeenCalled();
  expect(fake.liveWrites()).toBe(0);
}

describe("Fan curve preview", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
      connectedDaemonStatus,
    );
  });

  it("shows the starter curve against the live GPU temperature and previews it without writing", async () => {
    const fake = renderPerformance();
    const editor = await curveEditor();

    expect(pointValues(editor)).toEqual([
      [45, 20],
      [65, 60],
      [75, 80],
      [85, 100],
    ]);
    const graph = within(editor).getByRole("img", { name: /Fan curve graph/ });
    expect(graph).toHaveAccessibleName(
      "Fan curve graph: 45°C 20%, 65°C 60%, 75°C 80%, 85°C 100%. GPU temperature now 71°C.",
    );
    expect(
      within(editor).getByText(
        "Reaches 100% by 85°C, 5°C below the 90°C maximum operating temperature. Speeds 20-100% for both fans.",
      ),
    ).toBeInTheDocument();
    expect(
      await within(editor).findByText(
        "Valid curve. Preview only: Prism doesn't apply fan curves yet, so your fans keep their current mode.",
      ),
    ).toBeInTheDocument();
    expect(fake.previewFanCurve).toHaveBeenLastCalledWith({
      gpuId: "GPU-123",
      points: starter,
    });

    fake.gpu.telemetry.temperatureC = 74;
    await waitFor(() =>
      expect(graph).toHaveAccessibleName(
        "Fan curve graph: 45°C 20%, 65°C 60%, 75°C 80%, 85°C 100%. GPU temperature now 74°C.",
      ),
    );
    expectNoFanWrites(fake);
  });

  it("keeps invalid edits visible with the daemon's reasons while readings refresh", async () => {
    const user = userEvent.setup();
    const fake = renderPerformance();
    const editor = await curveEditor();
    const input = (name: string) =>
      within(editor).getByRole("spinbutton", { name });

    await user.clear(input("Point 2 temperature (°C)"));
    await user.type(input("Point 2 temperature (°C)"), "40");
    await user.clear(input("Point 1 speed (%)"));
    await user.type(input("Point 1 speed (%)"), "0");

    expect(
      await within(editor).findByText(
        "Point 2: temperature must be higher than point 1's 45°C.",
      ),
    ).toBeInTheDocument();
    expect(
      within(editor).getByText("Point 1: speed must be between 20% and 100%."),
    ).toBeInTheDocument();
    expect(input("Point 2 temperature (°C)")).toHaveAttribute(
      "aria-invalid",
      "true",
    );
    expect(input("Point 3 temperature (°C)")).toHaveAttribute(
      "aria-invalid",
      "false",
    );
    expect(within(editor).queryByText(/^Valid curve/)).not.toBeInTheDocument();

    // The daemon checks only complete curves.
    await user.clear(input("Point 3 speed (%)"));
    expect(
      within(editor).getByText("Point 3: enter a speed."),
    ).toBeInTheDocument();
    expect(
      within(editor).queryByText(/temperature must be higher/),
    ).not.toBeInTheDocument();

    fake.gpu.telemetry.temperatureC = 80;
    fake.gpu.cooling.fans[0].currentSpeedPercent = 55;
    await within(editor).findByRole("img", {
      name: /GPU temperature now 80°C/,
    });
    expect(pointValues(editor)).toEqual([
      [45, 0],
      [40, 60],
      [75, NaN],
      [85, 100],
    ]);
    expect(
      within(editor).getByText("Point 3: enter a speed."),
    ).toBeInTheDocument();

    await user.type(input("Point 3 speed (%)"), "80");
    expect(
      await within(editor).findByText(
        "Point 2: temperature must be higher than point 1's 45°C.",
      ),
    ).toBeInTheDocument();
    expectNoFanWrites(fake);
  });

  it("previews a curve starting at 0% only when every fan supports it", async () => {
    const user = userEvent.setup();
    const fake = renderPerformance({
      ...sharedCooling,
      fans: sharedCooling.fans.map((fan) => ({ ...fan, minSpeedPercent: 0 })),
      curveSupport: {
        status: "available",
        maxOperatingTemperatureC: 90,
        fullSpeedTemperatureC: 85,
        minSpeedPercent: 0,
        maxSpeedPercent: 100,
        starter: [
          { temperatureC: 45, speedPercent: 0 },
          { temperatureC: 65, speedPercent: 50 },
          { temperatureC: 75, speedPercent: 75 },
          { temperatureC: 85, speedPercent: 100 },
        ],
      },
    });
    const editor = await curveEditor();
    expect(
      within(editor).getByText(/Speeds 0-100% for both fans/),
    ).toBeInTheDocument();

    await user.clear(
      within(editor).getByRole("spinbutton", { name: "Point 2 speed (%)" }),
    );
    await user.type(
      within(editor).getByRole("spinbutton", { name: "Point 2 speed (%)" }),
      "0",
    );

    expect(await within(editor).findByText(/^Valid curve/)).toBeInTheDocument();
    expect(fake.previewFanCurve).toHaveBeenLastCalledWith({
      gpuId: "GPU-123",
      points: [
        { temperatureC: 45, speedPercent: 0 },
        { temperatureC: 65, speedPercent: 0 },
        { temperatureC: 75, speedPercent: 75 },
        { temperatureC: 85, speedPercent: 100 },
      ],
    });
    expectNoFanWrites(fake);
  });

  it("adds and removes points within 2 to 16, and returns to the starter curve", async () => {
    const user = userEvent.setup();
    const fake = renderPerformance();
    const editor = await curveEditor();
    const button = (name: string) =>
      within(editor).getByRole("button", { name });

    await user.click(button("Remove point 2"));
    await user.click(button("Remove point 2"));
    expect(pointValues(editor)).toEqual([
      [45, 20],
      [85, 100],
    ]);
    expect(button("Remove point 1")).toBeDisabled();
    expect(button("Remove point 2")).toBeDisabled();
    expect(await within(editor).findByText(/^Valid curve/)).toBeInTheDocument();

    for (let count = 2; count < 16; count++)
      await user.click(button("Add point"));
    const points = pointValues(editor);
    expect(points).toHaveLength(16);
    expect(button("Add point")).toBeDisabled();
    expect(points[0]).toEqual([45, 20]);
    expect(points[15]).toEqual([85, 100]);
    for (let index = 1; index < points.length; index++) {
      expect(points[index][0]).toBeGreaterThan(points[index - 1][0]);
      expect(points[index][1]).toBeGreaterThanOrEqual(points[index - 1][1]);
    }
    expect(await within(editor).findByText(/^Valid curve/)).toBeInTheDocument();
    expect(fake.previewFanCurve.mock.lastCall![0].points).toHaveLength(16);

    await user.click(button("Use starter curve"));
    expect(pointValues(editor)).toEqual([
      [45, 20],
      [65, 60],
      [75, 80],
      [85, 100],
    ]);
    expect(button("Use starter curve")).toBeDisabled();
    expectNoFanWrites(fake);
  });

  it("explains why curves are unavailable while automatic and fixed control keep working", async () => {
    const user = userEvent.setup();
    const fake = renderPerformance({
      ...sharedCooling,
      maxOperatingTemperatureC: null,
      maxOperatingTemperatureReason:
        "This GPU reports its thermal limit only as a T.Limit margin, not as a temperature.",
      curveSupport: {
        status: "unavailable",
        reason:
          "Fan curves need the GPU's maximum operating temperature. This GPU reports its thermal limit only as a T.Limit margin, not as a temperature.",
      },
    });
    const editor = await curveEditor();

    expect(
      within(editor).getByText(
        /This GPU reports its thermal limit only as a T.Limit margin, not as a temperature. Automatic and fixed fan control stay available./,
      ),
    ).toBeInTheDocument();
    expect(within(editor).queryByRole("spinbutton")).not.toBeInTheDocument();
    expect(within(editor).queryByRole("img")).not.toBeInTheDocument();

    const fixed = screen.getByRole("spinbutton", {
      name: "Fan 1 manual target",
    });
    await user.clear(fixed);
    await user.type(fixed, "65");
    await user.click(screen.getAllByRole("button", { name: "Set Manual" })[0]);
    await waitFor(() =>
      expect(fake.setFanSpeed).toHaveBeenCalledWith("GPU-123", 0, 65),
    );
    expect(fake.previewFanCurve).not.toHaveBeenCalled();
  });

  it("needs the daemon's judgement before offering a curve", async () => {
    renderPerformance({ ...sharedCooling, curveSupport: null });
    const editor = await curveEditor();

    expect(
      within(editor).getByText(/Fan curve previews need the Prism daemon./),
    ).toBeInTheDocument();
    expect(within(editor).queryByRole("spinbutton")).not.toBeInTheDocument();
  });
});
