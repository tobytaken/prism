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
import fanCurvePreview from "@/test/contracts/fan-curve-preview.json";
import type { CoolingInfo, FanCurveSupport } from "@/features/gpus/types";
import { GpuPerformancePage } from "./gpu-performance-page";

/** The curve support the daemon reports for each set of limits in the shared contract. */
const [twentyToHundred, zeroToHundred] = fanCurvePreview.limits.map(
  ({ curveSupport }) => curveSupport as FanCurveSupport,
);

/** The issue's worked example: T=85°C with a 20-100% range. */
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
  curveSupport: twentyToHundred,
  control: {
    applied: null,
    requested: [],
    curveTemperatureC: null,
    suspension: null,
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

const curveEditor = () => screen.findByRole("group", { name: "Fan curve" });

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
  expect(fake.applyCooling).not.toHaveBeenCalled();
  expect(fake.liveWrites()).toBe(0);
}

describe("Editing a fan curve", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
      connectedDaemonStatus,
    );
  });

  it("shows the starter curve against the live GPU temperature and checks it without writing", async () => {
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
        "Valid curve. Apply it to have Prism follow it for every fan, even after you close the app.",
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

  it("edits graph points by keyboard in the same draft as numeric inputs, then applies it", async () => {
    const user = userEvent.setup();
    const fake = renderPerformance();
    const editor = await curveEditor();
    const handle = within(editor).getByRole("button", {
      name: "Point 2: 65°C, 60%",
    });
    within(editor).getByRole("button", { name: "Point 1: 45°C, 20%" }).focus();
    await user.tab();
    expect(handle).toHaveFocus();
    await user.keyboard("{ArrowRight}{ArrowUp}");
    expect(pointValues(editor)[1]).toEqual([66, 61]);
    expect(handle).toHaveAccessibleName("Point 2: 66°C, 61%");
    expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
    expectNoFanWrites(fake);

    const speed = within(editor).getByRole("spinbutton", {
      name: "Point 2 speed (%)",
    });
    await user.clear(speed);
    await user.type(speed, "70");
    expect(
      within(editor).getByRole("button", { name: "Point 2: 66°C, 70%" }),
    ).toBeInTheDocument();
    await within(editor).findByText(/^Valid curve/);
    await user.click(
      within(editor).getByRole("button", { name: "Apply curve" }),
    );
    await waitFor(() =>
      expect(fake.applyCooling).toHaveBeenCalledWith({
        gpuId: "GPU-123",
        configuration: {
          arrangement: "shared",
          setting: {
            mode: "curve",
            points: [
              starter[0],
              { temperatureC: 66, speedPercent: 70 },
              ...starter.slice(2),
            ],
          },
        },
      }),
    );
    expect(await screen.findByText("Cooling: Applied")).toBeInTheDocument();
  });

  it("drags a point into the shared draft, retains it through telemetry, and applies through Apply all", async () => {
    const user = userEvent.setup();
    const fake = renderPerformance();
    const editor = await curveEditor();
    const graph = within(editor).getByRole("group", {
      name: "Fan curve graph editor",
    });
    // jsdom has no layout or pointer capture; supply only these browser boundaries.
    vi.spyOn(graph, "getBoundingClientRect").mockReturnValue(
      new DOMRect(100, 50, 640, 360),
    );
    const handle = within(editor).getByRole("button", {
      name: "Point 2: 65°C, 60%",
    });
    handle.setPointerCapture = vi.fn();
    handle.releasePointerCapture = vi.fn();
    await user.pointer({
      target: handle,
      keys: "[MouseLeft>]",
      coords: { clientX: 564, clientY: 188 },
    });
    // On the doubled-size graph, this is 70°C and 70% on the labelled axes.
    await user.pointer({
      target: handle,
      coords: { clientX: 596, clientY: 159 },
    });
    await user.pointer({ keys: "[/MouseLeft]" });
    expect(pointValues(editor)[1]).toEqual([70, 70]);
    expect(handle).toHaveAccessibleName("Point 2: 70°C, 70%");
    fake.gpu.telemetry.temperatureC = 80;
    await within(editor).findByRole("img", {
      name: /GPU temperature now 80°C/,
    });
    expect(pointValues(editor)).toEqual([
      [45, 20],
      [70, 70],
      [75, 80],
      [85, 100],
    ]);
    expectNoFanWrites(fake);
    // Moving after release must not keep editing.
    await user.pointer({
      target: handle,
      coords: { clientX: 650, clientY: 100 },
    });
    expect(pointValues(editor)[1]).toEqual([70, 70]);
    await user.click(screen.getByRole("button", { name: "Apply all" }));
    await waitFor(() =>
      expect(fake.applyCooling).toHaveBeenCalledWith({
        gpuId: "GPU-123",
        configuration: {
          arrangement: "shared",
          setting: {
            mode: "curve",
            points: [
              starter[0],
              { temperatureC: 70, speedPercent: 70 },
              ...starter.slice(2),
            ],
          },
        },
      }),
    );
    expect(await screen.findByText("Cooling: Applied")).toBeInTheDocument();
  });

  it("keeps graph moves and point removal inside the supported curve constraints", async () => {
    const user = userEvent.setup();
    const fake = renderPerformance();
    const editor = await curveEditor();
    const handle = (point: number) =>
      within(editor).getByRole("button", {
        name: new RegExp(`^Point ${point}:`),
      });
    handle(1).focus();
    await user.keyboard("{ArrowDown}{ArrowLeft>50/}");
    expect(pointValues(editor)[0]).toEqual([0, 20]);
    handle(2).focus();
    await user.keyboard("{ArrowRight>20/}{ArrowUp>30/}");
    expect(pointValues(editor)[1]).toEqual([74, 80]);
    handle(4).focus();
    await user.keyboard("{ArrowRight}{ArrowDown}");
    expect(pointValues(editor)[3]).toEqual([85, 100]);
    expect(
      within(editor).getByRole("button", { name: "Remove point 4" }),
    ).toBeDisabled();
    await user.click(
      within(editor).getByRole("button", { name: "Remove point 3" }),
    );
    expect(pointValues(editor)).toEqual([
      [0, 20],
      [74, 80],
      [85, 100],
    ]);
    expect(await within(editor).findByText(/^Valid curve/)).toBeInTheDocument();
    expectNoFanWrites(fake);
  });

  it("does not add duplicate temperatures when no whole-degree gap remains", async () => {
    const user = userEvent.setup();
    const fake = renderPerformance();
    const editor = await curveEditor();
    for (const [number, temperature] of [
      [2, 46],
      [3, 47],
      [4, 48],
    ]) {
      const input = within(editor).getByRole("spinbutton", {
        name: `Point ${number} temperature (°C)`,
      });
      await user.clear(input);
      await user.type(input, String(temperature));
    }
    expect(await within(editor).findByText(/^Valid curve/)).toBeInTheDocument();
    expect(
      within(editor).getByRole("button", { name: "Add point" }),
    ).toBeDisabled();
    expect(pointValues(editor)).toEqual([
      [45, 20],
      [46, 60],
      [47, 80],
      [48, 100],
    ]);
    expectNoFanWrites(fake);
  });

  it.each([
    ["", /enter a speed/],
    ["19.5", /whole percentage/],
    ["10", /speed must be between 20% and 100%/],
  ])(
    "preserves invalid numeric speed %s through graph and point-list interaction",
    async (value, error) => {
      const user = userEvent.setup();
      const fake = renderPerformance();
      const editor = await curveEditor();
      const input = within(editor).getByRole("spinbutton", {
        name: "Point 1 speed (%)",
      });
      await user.clear(input);
      if (value) await user.type(input, value);
      await within(editor).findByText(error);
      const handle = within(editor).getByRole("button", {
        name: "Point 2: 65°C, 60%",
      });
      handle.focus();
      await user.keyboard("{ArrowLeft}{ArrowDown}");
      expect(pointValues(editor)[1]).toEqual([65, 60]);
      await user.click(
        within(editor).getByRole("button", { name: "Add point" }),
      );
      expect(input).toHaveValue(value ? Number(value) : null);
      expect(await within(editor).findByText(error)).toBeInTheDocument();
      expect(
        within(editor).getByRole("button", { name: "Apply curve" }),
      ).toBeDisabled();
      expectNoFanWrites(fake);
    },
  );

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
      curveSupport: zeroToHundred,
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
      within(editor).getByText(/Fan curves need the Prism daemon./),
    ).toBeInTheDocument();
    expect(within(editor).queryByRole("spinbutton")).not.toBeInTheDocument();
  });
});
