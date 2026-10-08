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
    revision: 0,
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
      expect(fake.applyCooling).toHaveBeenCalledWith({
        gpuId: "GPU-123",
        expectedRevision: 0,
        configuration: {
          arrangement: "perFan",
          fans: [{ fanIndex: 0, setting: { mode: "fixed", speedPercent: 65 } }],
        },
      }),
    );
    expect(
      fake.previewFanCurve.mock.calls.every(
        ([request]) => request.points.length === 0,
      ),
    ).toBe(true);
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

describe("Shared and separate cooling arrangements", () => {
  it("copies a shared curve, edits mixed modes, and asks which setting to keep on joining", async () => {
    const fake = renderPerformance();
    const user = userEvent.setup();
    const curve = await curveEditor();
    await user.clear(
      within(curve).getByRole("spinbutton", { name: "Point 1 speed (%)" }),
    );
    await user.type(
      within(curve).getByRole("spinbutton", { name: "Point 1 speed (%)" }),
      "30",
    );
    await user.click(screen.getByRole("button", { name: "Separate fans" }));
    const first = screen.getByRole("group", { name: "Fan 1 configuration" });
    const second = screen.getByRole("group", { name: "Fan 2 configuration" });
    expect(pointValues(first)[0]).toEqual([45, 30]);
    expect(pointValues(second)[0]).toEqual([45, 30]);
    await user.selectOptions(
      within(second).getByRole("combobox", { name: "Control mode" }),
      "fixed",
    );
    await user.clear(
      within(second).getByRole("spinbutton", { name: "Fixed target (%)" }),
    );
    await user.type(
      within(second).getByRole("spinbutton", { name: "Fixed target (%)" }),
      "55",
    );
    expectNoFanWrites(fake);
    await user.click(screen.getByRole("button", { name: "Apply cooling" }));
    await waitFor(() =>
      expect(fake.gpu.cooling.control?.applied).toEqual({
        arrangement: "perFan",
        fans: [
          {
            fanIndex: 0,
            setting: {
              mode: "curve",
              points: [
                { temperatureC: 45, speedPercent: 30 },
                ...starter.slice(1),
              ],
            },
          },
          { fanIndex: 1, setting: { mode: "fixed", speedPercent: 55 } },
        ],
      }),
    );
    await user.click(
      screen.getByRole("button", { name: "Share one configuration" }),
    );
    const dialog = screen.getByRole("dialog", {
      name: "Choose a fan configuration",
    });
    await user.click(
      within(dialog).getByRole("button", { name: "Keep Fan 2" }),
    );
    expect(
      screen.getByRole("spinbutton", { name: "Fixed target (%)" }),
    ).toHaveValue(55);
    expect(fake.applyCooling).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole("button", { name: "Apply cooling" }));
    await waitFor(() =>
      expect(fake.gpu.cooling.control?.applied).toEqual({
        arrangement: "shared",
        setting: { mode: "fixed", speedPercent: 55 },
      }),
    );
  });
});

describe("Applying separate cooling", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
      connectedDaemonStatus,
    );
  });

  it("keeps a mixed draft through fan reordering, polling, and partial failure", async () => {
    const fake = renderPerformance({
      ...sharedCooling,
      fanCount: 3,
      fans: [0, 1, 2].map((index) => ({ ...sharedCooling.fans[0], index })),
    });
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", { name: "Separate fans" }),
    );
    const first = screen.getByRole("group", { name: "Fan 1 configuration" });
    const second = screen.getByRole("group", { name: "Fan 2 configuration" });
    await waitFor(() =>
      expect(
        within(first).getByRole("option", { name: "Curve" }),
      ).toBeEnabled(),
    );
    await user.selectOptions(within(first).getByRole("combobox"), "curve");
    await user.selectOptions(within(second).getByRole("combobox"), "fixed");
    await user.type(
      within(second).getByRole("spinbutton", { name: "Fixed target (%)" }),
      "55",
    );
    fake.gpu.cooling.fans.reverse();
    fake.gpu.telemetry.temperatureC = 80;
    await within(first).findByRole("img", { name: /GPU temperature now 80°C/ });
    fake.fanFailures[1] = "Rejected";
    await user.click(screen.getByRole("button", { name: "Apply all" }));
    expect(
      await screen.findByText("Fan 2: Failed — Rejected"),
    ).toBeInTheDocument();
    expect(screen.getByText(/Fan 3: Not attempted/)).toBeInTheDocument();
    expect(
      screen.getByText(/Curve control suspended: Fan 2/),
    ).toBeInTheDocument();
    expect(
      within(second).getByRole("spinbutton", { name: "Fixed target (%)" }),
    ).toHaveValue(55);
    expect(
      fake.gpu.cooling.fans.find((fan) => fan.index === 1)?.currentSpeedPercent,
    ).toBe(40);
    expect(screen.getByRole("button", { name: "Retry curve" })).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Use automatic cooling" }),
    ).toBeDisabled();
    expect(
      screen.queryByRole("spinbutton", { name: "Fan 1 manual target" }),
    ).not.toBeInTheDocument();
    delete fake.fanFailures[1];
    await user.click(
      screen.getByRole("button", { name: "Retry cooling change" }),
    );
    await screen.findByText("Cooling: Applied");
    expect(fake.gpu.cooling.control?.applied).toEqual({
      arrangement: "perFan",
      fans: [
        { fanIndex: 0, setting: { mode: "curve", points: starter } },
        { fanIndex: 1, setting: { mode: "fixed", speedPercent: 55 } },
        { fanIndex: 2, setting: { mode: "automatic" } },
      ],
    });
    expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
  });

  it("does not apply an open draft to a replacement GPU or newly discovered fan", async () => {
    const fake = renderPerformance();
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", { name: "Separate fans" }),
    );
    fake.gpu.stableId = "GPU-replacement";
    fake.gpu.cooling.fans.push({ ...sharedCooling.fans[0], index: 7 });
    await screen.findByText(/The GPU or targeted fans changed/);
    expect(
      screen.getByRole("button", { name: "Apply cooling" }),
    ).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Apply all" }));
    expectNoFanWrites(fake);
    expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
  });

  it("uses each fan's own curve limits when a shared curve is unavailable", async () => {
    const fake = renderPerformance({
      ...sharedCooling,
      fans: [
        { ...sharedCooling.fans[0], index: 7, maxSpeedPercent: 80 },
        sharedCooling.fans[1],
      ],
      curveSupport: {
        status: "unavailable",
        reason: "Fans have different maximum speeds.",
      },
    });
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", { name: "Separate fans" }),
    );
    const fan = screen.getByRole("group", { name: "Fan 8 configuration" });
    await waitFor(() =>
      expect(within(fan).getByRole("option", { name: "Curve" })).toBeEnabled(),
    );
    await user.selectOptions(within(fan).getByRole("combobox"), "curve");
    expect(pointValues(fan)).toEqual([
      [45, 20],
      [65, 50],
      [75, 65],
      [85, 80],
    ]);
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Apply cooling" }),
      ).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: "Apply cooling" }));
    await screen.findByText("Cooling: Applied");
    expect(
      fake.gpu.cooling.fans.find((fan) => fan.index === 7)?.currentSpeedPercent,
    ).toBe(59);
    expect(fake.gpu.cooling.fans.find((fan) => fan.index === 1)?.mode).toBe(
      "auto",
    );
  });
});

it("preserves unfinished individual fan edits before changing arrangements", async () => {
  vi.restoreAllMocks();
  vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
    connectedDaemonStatus,
  );
  const fake = renderPerformance();
  const user = userEvent.setup();
  const target = await screen.findByRole("spinbutton", {
    name: "Fan 1 manual target",
  });
  await user.clear(target);
  await user.type(target, "55");
  expect(
    screen.queryByRole("button", { name: "Separate fans" }),
  ).not.toBeInTheDocument();
  expect(
    screen.getByText(/Apply or discard individual fan changes/),
  ).toBeInTheDocument();
  expect(target).toHaveValue(55);
  expectNoFanWrites(fake);
  await user.click(
    screen.getByRole("button", { name: "Discard fan 1 change" }),
  );
  await user.click(screen.getByRole("button", { name: "Separate fans" }));
  expect(
    screen.queryByRole("spinbutton", { name: "Fan 1 manual target" }),
  ).not.toBeInTheDocument();
  expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
  expectNoFanWrites(fake);
});

it("applies a cooling draft only to its GPU when identical models are reordered", async () => {
  vi.restoreAllMocks();
  vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
    connectedDaemonStatus,
  );
  const first = createPerformanceGpu({
    cooling: structuredClone(sharedCooling),
  });
  const second = createPerformanceGpu({
    id: "GPU-456",
    stableId: "GPU-456",
    busId: "0000:02:00.0",
    cooling: structuredClone(sharedCooling),
  });
  const response = createPerformanceResponse({ gpus: [first, second] });
  const fake = createFakeDesktop({ response });
  renderWithQueryClient(
    <MemoryRouter>
      <GpuPerformancePage />
    </MemoryRouter>,
  );
  const user = userEvent.setup();
  await user.click(
    (await screen.findAllByRole("button", { name: "Separate fans" }))[1],
  );
  const fan = screen.getByRole("group", { name: "Fan 2 configuration" });
  await user.selectOptions(within(fan).getByRole("combobox"), "fixed");
  await user.type(
    within(fan).getByRole("spinbutton", { name: "Fixed target (%)" }),
    "55",
  );
  response.gpus.reverse();
  second.cooling.fans.reverse();
  second.telemetry.temperatureC = 80;
  await waitFor(() =>
    expect(screen.getAllByText(/0000:02:00.0/).length).toBeGreaterThan(0),
  );
  await user.click(screen.getByRole("button", { name: "Apply all" }));
  await screen.findByText("Cooling: Applied");
  expect(fake.applyCooling).toHaveBeenCalledWith({
    gpuId: "GPU-456",
    expectedRevision: 0,
    fanIndexes: [0, 1],
    configuration: {
      arrangement: "perFan",
      fans: [
        { fanIndex: 0, setting: { mode: "automatic" } },
        { fanIndex: 1, setting: { mode: "fixed", speedPercent: 55 } },
      ],
    },
  });
  expect(first.cooling.control?.applied).toBeNull();
  expect(
    first.cooling.fans.every((fan) => fan.currentSpeedPercent === 40),
  ).toBe(true);
  expect(
    second.cooling.fans.find((fan) => fan.index === 1)?.currentSpeedPercent,
  ).toBe(55);
});
