import { screen, waitFor, act, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { it, expect, beforeEach, vi } from "vitest";
import { MemoryRouter } from "react-router-dom";
import { connectedDaemonStatus } from "@/test/gpu-fixtures";
import { createFakeDesktop } from "@/test/fake-desktop";
import { renderWithQueryClient } from "@/test/query-client";
import type {
  CoolingConfiguration,
  FanCurveSupport,
} from "@/features/gpus/types";
import curvePreview from "@/test/contracts/fan-curve-preview.json";
import {
  saveAsNew,
  saveAsNewAvailability,
  chooseProfile,
  profileSelector,
} from "@/test/profile-selector";
import { GpuPerformancePage } from "./gpu-performance-page";

beforeEach(() => {
  vi.restoreAllMocks();
  vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
    connectedDaemonStatus,
  );
});

it("captures automatic cooling and blocks saving unapplied fan edits", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop();
  renderWithQueryClient(
    <MemoryRouter>
      <GpuPerformancePage />
    </MemoryRouter>,
  );
  await saveAsNew(user, "Automatic");
  await waitFor(() => expect(fake.profiles).toHaveLength(1));
  expect(fake.profiles[0].settings).toHaveProperty("cooling", {
    arrangement: "shared",
    setting: { mode: "automatic" },
  });
  const fan = screen.getByRole("spinbutton", { name: "Fan 1 manual target" });
  await waitFor(() => expect(fan).toBeEnabled());
  await user.clear(fan);
  await user.type(fan, "65");
  expect((await saveAsNewAvailability(user)).enabled).toBe(false);
});

it("applies a curve profile at current temperature and keeps it clean while output changes", async () => {
  const user = userEvent.setup();
  const curve: CoolingConfiguration = {
    arrangement: "shared",
    setting: {
      mode: "curve",
      points: [
        { temperatureC: 45, speedPercent: 30 },
        { temperatureC: 85, speedPercent: 100 },
      ],
    },
  };
  const fake = createFakeDesktop({
    profiles: [
      {
        id: "curve",
        name: "Curve",
        gpuId: "GPU-123",
        settings: { cooling: curve },
      },
    ],
  });
  fake.gpu.cooling.maxOperatingTemperatureC = 90;
  fake.gpu.cooling.curveSupport = {
    ...curvePreview.limits[0].curveSupport,
    minSpeedPercent: 30,
  } as FanCurveSupport;
  fake.gpu.telemetry.temperatureC = 65;
  const { queryClient } = renderWithQueryClient(
    <MemoryRouter>
      <GpuPerformancePage />
    </MemoryRouter>,
  );
  await chooseProfile(user, "Curve");
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Curve"));
  expect(fake.gpu.cooling.fans[0].currentSpeedPercent).toBe(65);
  fake.gpu.telemetry.temperatureC = 85;
  fake.followCurves();
  await act(async () => {
    await queryClient.invalidateQueries({ queryKey: ["gpu-performance"] });
  });
  expect(fake.gpu.cooling.fans[0].currentSpeedPercent).toBe(100);
  expect(
    screen.queryByText("Changes not saved to profile"),
  ).not.toBeInTheDocument();
  await saveAsNew(user, "Curve copy");
  await waitFor(() => expect(fake.profiles).toHaveLength(2));
  expect(fake.profiles[1].settings.cooling).toEqual(curve);
});

it("retains failed cooling profile edits and blocks saving a partial application", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    profiles: [
      {
        id: "fixed",
        name: "Fixed",
        gpuId: "GPU-123",
        settings: {
          powerLimitWatts: { kind: "value", value: 500 },
          cooling: {
            arrangement: "shared",
            setting: { mode: "fixed", speedPercent: 70 },
          },
        },
      },
    ],
  });
  fake.fanFailures[0] = "Fan denied";
  renderWithQueryClient(
    <MemoryRouter>
      <GpuPerformancePage />
    </MemoryRouter>,
  );
  await chooseProfile(user, "Fixed");
  await screen.findByText(/Cooling: Failed/);
  expect(fake.gpu.power.currentLimitWatts).toBe(500);
  expect(profileSelector()).toHaveTextContent("No current profile");
  expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
  expect((await saveAsNewAvailability(user)).enabled).toBe(false);
});

it("reviews cooling corrections, leaves cancellation unchanged, and retains failed corrected edits", async () => {
  const user = userEvent.setup();
  const saved: CoolingConfiguration = {
    arrangement: "shared",
    setting: { mode: "fixed", speedPercent: 10 },
  };
  const proposed: CoolingConfiguration = {
    arrangement: "shared",
    setting: { mode: "fixed", speedPercent: 30 },
  };
  const fake = createFakeDesktop({
    profiles: [
      {
        id: "fixed",
        name: "Fixed",
        gpuId: "GPU-123",
        settings: { cooling: saved },
      },
    ],
  });
  fake.applyProfileById.mockResolvedValue({
    power: null,
    graphics: null,
    memory: null,
    cooling: {
      revision: 0,
      outcome: {
        status: "unattempted",
        error: "Cooling is outside the current range",
      },
      fans: [],
    },
    corrections: [
      { setting: "cooling", savedValue: saved, proposedValue: proposed },
    ],
  });
  fake.fanFailures[0] = "Fan denied";
  renderWithQueryClient(
    <MemoryRouter>
      <GpuPerformancePage />
    </MemoryRouter>,
  );
  await chooseProfile(user, "Fixed");
  let review = await screen.findByRole("region", {
    name: "Review profile corrections",
  });
  expect(review).toHaveTextContent("Shared: Fixed 10% → Shared: Fixed 30%");
  await user.click(within(review).getByRole("button", { name: "Cancel" }));
  expect(fake.profiles[0].settings.cooling).toEqual(saved);
  expect(fake.gpu.cooling.fans[0].mode).toBe("auto");
  await chooseProfile(user, "Fixed");
  review = await screen.findByRole("region", {
    name: "Review profile corrections",
  });
  await user.click(
    within(review).getByRole("button", { name: "Adjust profile and apply" }),
  );
  await screen.findByText(/Cooling: Failed/);
  expect(fake.profiles[0].settings.cooling).toEqual(proposed);
  expect((await saveAsNewAvailability(user)).enabled).toBe(false);
});

it("requires review when another session changes cooling after a failed profile application", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    profiles: [
      {
        id: "fixed",
        name: "Fixed",
        gpuId: "GPU-123",
        settings: {
          cooling: {
            arrangement: "shared",
            setting: { mode: "fixed", speedPercent: 70 },
          },
        },
      },
    ],
  });
  fake.gpu.cooling.control = {
    revision: 0,
    applied: null,
    requested: [],
    curveTemperatureC: null,
    suspension: null,
  };
  fake.fanFailures[0] = "Fan denied";
  const { queryClient } = renderWithQueryClient(
    <MemoryRouter>
      <GpuPerformancePage />
    </MemoryRouter>,
  );
  await chooseProfile(user, "Fixed");
  await screen.findByText(/Cooling: Failed/);
  expect(
    screen.queryByRole("button", { name: "Review cooling change" }),
  ).not.toBeInTheDocument();
  await act(async () => {
    await fake.applyCooling({
      gpuId: "GPU-123",
      configuration: { arrangement: "shared", setting: { mode: "automatic" } },
    });
    await queryClient.invalidateQueries();
  });
  expect(
    await screen.findByRole("button", { name: "Review cooling change" }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Retry cooling change" }),
  ).toBeDisabled();
});

it("does not acknowledge competing cooling when profile cooling was never attempted", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    profiles: [
      {
        id: "fixed",
        name: "Fixed",
        gpuId: "GPU-123",
        settings: {
          cooling: {
            arrangement: "shared",
            setting: { mode: "fixed", speedPercent: 70 },
          },
        },
      },
    ],
  });
  fake.gpu.cooling.control = {
    revision: 0,
    applied: null,
    requested: [],
    curveTemperatureC: null,
    suspension: null,
  };
  fake.applyProfileById.mockImplementationOnce(async () => {
    const external = await fake.applyCooling({
      gpuId: "GPU-123",
      configuration: { arrangement: "shared", setting: { mode: "automatic" } },
    });
    return {
      power: null,
      graphics: null,
      memory: null,
      cooling: {
        revision: external.revision,
        outcome: { status: "unattempted", error: "Profile validation failed" },
        fans: [],
      },
    };
  });
  renderWithQueryClient(
    <MemoryRouter>
      <GpuPerformancePage />
    </MemoryRouter>,
  );
  await chooseProfile(user, "Fixed");
  await screen.findByText(/Cooling: Not attempted/);
  expect(
    await screen.findByRole("button", { name: "Review cooling change" }),
  ).toBeInTheDocument();
});
