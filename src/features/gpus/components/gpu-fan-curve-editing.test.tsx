import { MemoryRouter } from "react-router-dom";
import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
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
    recovery: null,
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

describe("Remembered fan curves", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
      connectedDaemonStatus,
    );
  });

  it("recalls the remembered curve from Automatic as an unapplied editable draft", async () => {
    const cooling = structuredClone(sharedCooling);
    cooling.control!.rememberedCurve = {
      arrangement: "shared",
      setting: {
        mode: "curve",
        points: [
          { temperatureC: 40, speedPercent: 30 },
          { temperatureC: 80, speedPercent: 100 },
        ],
      },
    };
    const fake = renderPerformance(cooling);
    const user = userEvent.setup();
    await user.selectOptions(
      await screen.findByRole("combobox", { name: "Control mode" }),
      "curve",
    );
    const editor = await curveEditor();
    expect(pointValues(editor)).toEqual([
      [40, 30],
      [80, 100],
    ]);
    expect(within(editor).getByText("Unapplied")).toBeInTheDocument();
    expectNoFanWrites(fake);
    const apply = within(editor).getByRole("button", { name: "Apply curve" });
    await waitFor(() => expect(apply).toBeEnabled());
    await user.click(apply);
    await waitFor(() =>
      expect(fake.gpu.cooling.control?.applied).toEqual(
        cooling.control!.rememberedCurve,
      ),
    );
  });

  it("keeps the last applied curve after Fixed, reopening, and discarding later edits", async () => {
    const fake = renderPerformance();
    const user = userEvent.setup();
    let editor = await curveEditor();
    const input = within(editor).getByRole("spinbutton", {
      name: "Point 1 speed (%)",
    });
    await user.clear(input);
    await user.type(input, "35");
    let apply = within(editor).getByRole("button", { name: "Apply curve" });
    await waitFor(() => expect(apply).toBeEnabled());
    await user.click(apply);
    await waitFor(() =>
      expect(fake.gpu.cooling.control?.rememberedCurve).toBeDefined(),
    );
    const remembered = structuredClone(
      fake.gpu.cooling.control!.rememberedCurve,
    );

    await user.selectOptions(
      screen.getByRole("combobox", { name: "Control mode" }),
      "fixed",
    );
    await user.type(
      screen.getByRole("spinbutton", { name: "Fixed target (%)" }),
      "55",
    );
    await user.click(screen.getByRole("button", { name: "Apply cooling" }));
    await waitFor(() =>
      expect(fake.gpu.cooling.control?.applied).toEqual({
        arrangement: "shared",
        setting: { mode: "fixed", speedPercent: 55 },
      }),
    );
    const reopen = () => {
      cleanup();
      renderWithQueryClient(
        <MemoryRouter>
          <GpuPerformancePage />
        </MemoryRouter>,
      );
    };
    reopen();
    await user.selectOptions(
      await screen.findByRole("combobox", { name: "Control mode" }),
      "curve",
    );
    editor = await curveEditor();
    expect(pointValues(editor)).toEqual([
      [45, 35],
      [65, 60],
      [75, 80],
      [85, 100],
    ]);
    expect(fake.applyCooling).toHaveBeenCalledTimes(2);
    const edit = within(editor).getByRole("spinbutton", {
      name: "Point 1 speed (%)",
    });
    await user.clear(edit);
    await user.type(edit, "45");
    // Closing discards the draft, keeping the last successfully applied curve.
    reopen();
    await user.selectOptions(
      await screen.findByRole("combobox", { name: "Control mode" }),
      "curve",
    );
    editor = await curveEditor();
    expect(pointValues(editor)[0]).toEqual([45, 35]);
    expect(fake.gpu.cooling.control?.rememberedCurve).toEqual(remembered);
    expect(fake.applyCooling).toHaveBeenCalledTimes(2);
    apply = within(editor).getByRole("button", { name: "Apply curve" });
    await waitFor(() => expect(apply).toBeEnabled());
    await user.click(apply);
    await waitFor(() =>
      expect(fake.gpu.cooling.control?.applied).toEqual(remembered),
    );
  });

  it("recalls the complete separate arrangement and mixed modes before Apply", async () => {
    const cooling = structuredClone(sharedCooling);
    cooling.control!.rememberedCurve = {
      arrangement: "perFan",
      fans: [
        { fanIndex: 0, setting: { mode: "curve", points: starter } },
        { fanIndex: 1, setting: { mode: "fixed", speedPercent: 57 } },
      ],
    };
    const fake = renderPerformance(cooling);
    const user = userEvent.setup();
    await user.selectOptions(
      await screen.findByRole("combobox", { name: "Control mode" }),
      "curve",
    );
    const first = await screen.findByRole("group", {
      name: "Fan 1 configuration",
    });
    const second = screen.getByRole("group", { name: "Fan 2 configuration" });
    expect(
      within(first).getByRole("combobox", { name: "Control mode" }),
    ).toHaveValue("curve");
    expect(
      within(second).getByRole("combobox", { name: "Control mode" }),
    ).toHaveValue("fixed");
    expect(
      within(second).getByRole("spinbutton", { name: "Fixed target (%)" }),
    ).toHaveValue(57);
    expectNoFanWrites(fake);
    const apply = screen.getByRole("button", { name: "Apply cooling" });
    await waitFor(() => expect(apply).toBeEnabled());
    await user.click(apply);
    await waitFor(() =>
      expect(fake.gpu.cooling.control?.applied).toEqual(
        cooling.control!.rememberedCurve,
      ),
    );
  });

  it("validates a recalled curve against current limits and preserves it after a failed apply", async () => {
    const cooling = structuredClone(sharedCooling);
    cooling.control!.rememberedCurve = {
      arrangement: "shared",
      setting: {
        mode: "curve",
        points: [
          { temperatureC: 40, speedPercent: 0 },
          { temperatureC: 80, speedPercent: 100 },
        ],
      },
    };
    const fake = renderPerformance(cooling);
    const user = userEvent.setup();
    await user.selectOptions(
      await screen.findByRole("combobox", { name: "Control mode" }),
      "curve",
    );
    const editor = await curveEditor();
    expect(pointValues(editor)).toEqual([
      [40, 0],
      [80, 100],
    ]);
    const apply = within(editor).getByRole("button", { name: "Apply curve" });
    await within(editor).findByText(/speed must be between 20% and 100%/);
    expect(apply).toBeDisabled();
    expectNoFanWrites(fake);
    await user.type(
      within(editor).getByRole("spinbutton", { name: "Point 1 speed (%)" }),
      "30",
    );
    await waitFor(() => expect(apply).toBeEnabled());
    fake.fanFailures[1] = "fan unavailable";
    await user.click(apply);
    await waitFor(() => expect(fake.applyCooling).toHaveBeenCalledOnce());
    expect(fake.gpu.cooling.control?.rememberedCurve).toEqual(
      cooling.control!.rememberedCurve,
    );
  });

  it("requires review when the remembered arrangement targets fans that are no longer present", async () => {
    const cooling = structuredClone(sharedCooling);
    cooling.control!.rememberedCurve = {
      arrangement: "perFan",
      fans: [
        { fanIndex: 0, setting: { mode: "curve", points: starter } },
        { fanIndex: 7, setting: { mode: "automatic" } },
      ],
    };
    const fake = renderPerformance(cooling);
    const user = userEvent.setup();
    await user.selectOptions(
      await screen.findByRole("combobox", { name: "Control mode" }),
      "curve",
    );
    expect(
      await screen.findByText(/The GPU or targeted fans changed/),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Apply cooling" }),
    ).toBeDisabled();
    expectNoFanWrites(fake);
  });

  it("lets a previously fixed fan choose Curve while preserving the other fan's draft", async () => {
    const cooling = structuredClone(sharedCooling);
    cooling.control!.rememberedCurve = {
      arrangement: "perFan",
      fans: [
        { fanIndex: 0, setting: { mode: "curve", points: starter } },
        { fanIndex: 1, setting: { mode: "fixed", speedPercent: 57 } },
      ],
    };
    cooling.control!.applied = structuredClone(
      cooling.control!.rememberedCurve,
    );
    const fake = renderPerformance(cooling);
    const user = userEvent.setup();
    const first = await screen.findByRole("group", {
      name: "Fan 1 configuration",
    });
    const second = screen.getByRole("group", { name: "Fan 2 configuration" });
    await user.selectOptions(
      within(first).getByRole("combobox", { name: "Control mode" }),
      "fixed",
    );
    await user.type(
      within(first).getByRole("spinbutton", { name: "Fixed target (%)" }),
      "66",
    );
    await waitFor(() =>
      expect(
        within(second).getByRole("option", { name: "Curve" }),
      ).toBeEnabled(),
    );
    await user.selectOptions(
      within(second).getByRole("combobox", { name: "Control mode" }),
      "curve",
    );
    expect(
      within(first).getByRole("combobox", { name: "Control mode" }),
    ).toHaveValue("fixed");
    expect(
      within(first).getByRole("spinbutton", { name: "Fixed target (%)" }),
    ).toHaveValue(66);
    expect(
      within(second).getByRole("combobox", { name: "Control mode" }),
    ).toHaveValue("curve");
    expectNoFanWrites(fake);
    const apply = screen.getByRole("button", { name: "Apply cooling" });
    await waitFor(() => expect(apply).toBeEnabled());
    await user.click(apply);
    await waitFor(() =>
      expect(fake.gpu.cooling.control?.applied).toEqual({
        arrangement: "perFan",
        fans: [
          { fanIndex: 0, setting: { mode: "fixed", speedPercent: 66 } },
          { fanIndex: 1, setting: { mode: "curve", points: starter } },
        ],
      }),
    );
  });
});

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
        expectedRevision: 0,
        fanIndexes: [0, 1],
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
    await within(editor).findByText(/^Valid curve/);
    const validate = fake.previewFanCurve.getMockImplementation()!;
    let release!: () => void;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    fake.previewFanCurve.mockImplementation(async (request) => {
      await pending;
      return validate(request);
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
    await user.pointer({
      target: handle,
      coords: { clientX: 583, clientY: 174 },
    });
    expect(pointValues(editor)[1]).toEqual([68, 65]);
    // Keep dragging while the previous position's preview is still pending.
    // On the doubled-size graph, this is 70°C and 70% on the labeled axes.
    await user.pointer({
      target: handle,
      coords: { clientX: 596, clientY: 159 },
    });
    await user.pointer({ keys: "[/MouseLeft]" });
    expect(pointValues(editor)[1]).toEqual([70, 70]);
    expect(handle).toHaveAccessibleName("Point 2: 70°C, 70%");
    await act(async () => release());
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
        expectedRevision: 0,
        fanIndexes: [0, 1],
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

  it("waits for the current numeric draft's validation before allowing graph edits", async () => {
    const user = userEvent.setup();
    const fake = renderPerformance();
    const editor = await curveEditor();
    await within(editor).findByText(/^Valid curve/);
    const validate = fake.previewFanCurve.getMockImplementation()!;
    let release!: () => void;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    fake.previewFanCurve.mockImplementation(async (request) => {
      await pending;
      return validate(request);
    });
    const speed = within(editor).getByRole("spinbutton", {
      name: "Point 1 speed (%)",
    });
    await user.clear(speed);
    await user.type(speed, "10");
    within(editor).getByRole("button", { name: "Point 1: 45°C, 10%" }).focus();
    await user.keyboard("{ArrowRight}");
    expect(pointValues(editor)[0]).toEqual([45, 10]);
    await act(async () => release());
    expect(
      await within(editor).findByText(/speed must be between 20% and 100%/),
    ).toBeInTheDocument();
    expect(
      within(editor).getByRole("button", { name: "Apply curve" }),
    ).toBeDisabled();
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

  it("needs the daemon's judgment before offering a curve", async () => {
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
    await within(first).findByText(/^Valid curve/);
    within(first).getByRole("button", { name: "Point 1: 45°C, 30%" }).focus();
    await user.keyboard("{ArrowUp}");
    expect(pointValues(first)[0]).toEqual([45, 31]);
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
                { temperatureC: 45, speedPercent: 31 },
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
