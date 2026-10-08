import { MemoryRouter } from "react-router-dom";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { createFakeDesktop } from "@/test/fake-desktop";
import {
  connectedDaemonStatus,
  createPerformanceGpu,
  createPerformanceResponse,
} from "@/test/gpu-fixtures";
import { renderWithQueryClient } from "@/test/query-client";
import fanCurvePreview from "@/test/contracts/fan-curve-preview.json";
import type { CoolingInfo, FanCurveSupport } from "@/features/gpus/types";
import { GpuPerformancePage } from "./gpu-performance-page";

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
  curveSupport: fanCurvePreview.limits[0].curveSupport as FanCurveSupport,
  control: {
    applied: null,
    requested: [],
    curveTemperatureC: null,
    suspension: null,
  },
};

const starter = [
  { temperatureC: 45, speedPercent: 20 },
  { temperatureC: 65, speedPercent: 60 },
  { temperatureC: 75, speedPercent: 80 },
  { temperatureC: 85, speedPercent: 100 },
];

const sharedCurve = (points = starter) => ({
  arrangement: "shared" as const,
  setting: { mode: "curve" as const, points },
});

function renderPerformance() {
  const fake = createFakeDesktop({
    response: createPerformanceResponse({
      gpus: [createPerformanceGpu({ cooling: structuredClone(sharedCooling) })],
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
const coolingStatus = () =>
  screen.findByRole("group", { name: "Cooling control" });

describe("Applying a shared fan curve", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
      connectedDaemonStatus,
    );
  });

  it("applies the curve on request and keeps the applied configuration, requested speed and driver report apart", async () => {
    const user = userEvent.setup();
    const fake = renderPerformance();
    const editor = await curveEditor();
    expect(
      within(await coolingStatus()).getByText(
        "Prism hasn't applied cooling to this GPU.",
      ),
    ).toBeInTheDocument();

    await user.click(
      await within(editor).findByRole("button", { name: "Apply curve" }),
    );

    await waitFor(() =>
      expect(fake.applyCooling).toHaveBeenCalledWith({
        gpuId: "GPU-123",
        fanIndexes: [0, 1],
        configuration: sharedCurve(),
      }),
    );
    const status = await coolingStatus();
    expect(
      await within(status).findByText("Applied: fan curve for all fans"),
    ).toBeInTheDocument();
    expect(
      within(status).getByText("The curve requests 72% at 71°C."),
    ).toBeInTheDocument();
    expect(screen.getAllByText("Driver reports manual mode")).toHaveLength(2);
    expect(within(status).queryByText(/fixed/i)).not.toBeInTheDocument();
    expect(within(editor).getByText("Applied")).toBeInTheDocument();
    expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
    expect(fake.setFanSpeed).not.toHaveBeenCalled();
  });

  it("keeps edits as unapplied changes, without writes, until Apply all", async () => {
    const user = userEvent.setup();
    const fake = renderPerformance();
    const editor = await curveEditor();
    const speed = within(editor).getByRole("spinbutton", {
      name: "Point 1 speed (%)",
    });

    await user.clear(speed);
    await user.type(speed, "30");
    expect(await screen.findByText("Unapplied changes")).toBeInTheDocument();
    expect(within(editor).getByText("Unapplied")).toBeInTheDocument();
    fake.gpu.telemetry.temperatureC = 74;
    await within(editor).findByRole("img", {
      name: /GPU temperature now 74°C/,
    });
    expect(speed).toHaveValue(30);
    expect(fake.applyCooling).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Apply all" }));

    const applied = sharedCurve([
      { temperatureC: 45, speedPercent: 30 },
      ...starter.slice(1),
    ]);
    await waitFor(() =>
      expect(fake.applyCooling).toHaveBeenCalledWith({
        gpuId: "GPU-123",
        configuration: applied,
        fanIndexes: [0, 1],
      }),
    );
    await waitFor(() =>
      expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument(),
    );
    expect(fake.gpu.cooling.control?.applied).toEqual(applied);
    expect(speed).toHaveValue(30);
  });

  it("blocks Apply for an invalid curve and keeps the daemon's rejection with the draft", async () => {
    const user = userEvent.setup();
    const fake = renderPerformance();
    const editor = await curveEditor();
    const speed = within(editor).getByRole("spinbutton", {
      name: "Point 4 speed (%)",
    });

    await user.clear(speed);
    await user.type(speed, "90");
    expect(
      await within(editor).findByText(/must reach 100% by 85°C/),
    ).toBeInTheDocument();
    expect(
      within(editor).getByRole("button", { name: "Apply curve" }),
    ).toBeDisabled();

    // Readings change after the check: the daemon validates again and writes nothing.
    await user.clear(speed);
    await user.type(speed, "100");
    await within(editor).findByText(/^Valid curve/);
    fake.gpu.telemetry.temperatureC = null;
    await user.click(
      within(editor).getByRole("button", { name: "Apply curve" }),
    );

    expect(
      await screen.findByText(
        /Cooling: Not attempted — The GPU's temperature is unavailable/,
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Retry cooling change" }),
    ).toBeEnabled();
    expect(fake.gpu.cooling.control?.applied).toBeNull();
  });

  it("shows routine curve output without treating it as a change", async () => {
    const user = userEvent.setup();
    const fake = renderPerformance();
    await user.click(
      await within(await curveEditor()).findByRole("button", {
        name: "Apply curve",
      }),
    );
    const status = await coolingStatus();
    await within(status).findByText("The curve requests 72% at 71°C.");

    fake.gpu.telemetry.temperatureC = 80;
    fake.followCurves();

    expect(
      await within(status).findByText("The curve requests 90% at 80°C."),
    ).toBeInTheDocument();
    expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
    expect(fake.applyCooling).toHaveBeenCalledTimes(1);
  });

  it("reports a suspended curve with each fan's restoration and retries the kept curve", async () => {
    const user = userEvent.setup();
    const fake = renderPerformance();
    await user.click(
      await within(await curveEditor()).findByRole("button", {
        name: "Apply curve",
      }),
    );
    const status = await coolingStatus();
    await within(status).findByText("The curve requests 72% at 71°C.");

    fake.suspendCurve(
      "The GPU's temperature is unavailable, so Prism stopped following its curve.",
      [
        { fanIndex: 0, status: "restored", detail: null },
        {
          fanIndex: 1,
          status: "unverified",
          detail:
            "The driver accepted the request but doesn't report the fan's control mode.",
        },
      ],
    );

    expect(
      await within(status).findByText(
        "Curve control suspended: The GPU's temperature is unavailable, so Prism stopped following its curve.",
      ),
    ).toBeInTheDocument();
    expect(
      within(status).getByText("Fan 1: automatic cooling restored"),
    ).toBeInTheDocument();
    expect(
      within(status).getByText(
        "Fan 2: automatic cooling not confirmed — The driver accepted the request but doesn't report the fan's control mode.",
      ),
    ).toBeInTheDocument();
    expect(
      within(status).getByText("Applied: fan curve for all fans"),
    ).toBeInTheDocument();
    expect(within(status).queryByText(/The curve requests/)).toBeNull();

    await user.click(
      within(status).getByRole("button", { name: "Retry curve" }),
    );

    await waitFor(() => expect(fake.applyCooling).toHaveBeenCalledTimes(2));
    expect(fake.applyCooling).toHaveBeenLastCalledWith({
      gpuId: "GPU-123",
      configuration: sharedCurve(),
    });
    await waitFor(() =>
      expect(within(status).queryByText(/suspended/)).not.toBeInTheDocument(),
    );
  });

  it("returns every fan to automatic cooling through the same owner", async () => {
    const user = userEvent.setup();
    const fake = renderPerformance();
    await user.click(
      await within(await curveEditor()).findByRole("button", {
        name: "Apply curve",
      }),
    );
    const status = await coolingStatus();
    await within(status).findByText("The curve requests 72% at 71°C.");

    await user.click(
      within(status).getByRole("button", { name: "Use automatic cooling" }),
    );

    await waitFor(() =>
      expect(fake.applyCooling).toHaveBeenLastCalledWith({
        gpuId: "GPU-123",
        configuration: {
          arrangement: "shared",
          setting: { mode: "automatic" },
        },
      }),
    );
    expect(
      await within(status).findByText(
        "Applied: automatic cooling for all fans",
      ),
    ).toBeInTheDocument();
    expect(screen.getAllByText("Driver reports auto mode")).toHaveLength(2);
    expect(fake.setFanModeAuto).not.toHaveBeenCalled();
  });

  it("reports each failed fan and keeps the curve to retry", async () => {
    const user = userEvent.setup();
    const fake = renderPerformance();
    fake.fanFailures[1] = "Unknown error";
    await user.click(
      await within(await curveEditor()).findByRole("button", {
        name: "Apply curve",
      }),
    );

    expect(
      await screen.findByText("Cooling: Failed — Fan 2: Unknown error"),
    ).toBeInTheDocument();
    expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
    const status = await coolingStatus();
    expect(
      await within(status).findByText(/Curve control suspended: Fan 2/),
    ).toBeInTheDocument();

    delete fake.fanFailures[1];
    await user.click(
      screen.getByRole("button", { name: "Retry cooling change" }),
    );
    expect(await screen.findByText("Cooling: Applied")).toBeInTheDocument();
    expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
  });
});
