import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import {
  connectedDaemonStatus,
  createPerformanceGpu,
  createPerformanceResponse,
} from "@/test/gpu-fixtures";
import { createFakeDesktop } from "@/test/fake-desktop";
import {
  chooseProfile,
  findProfileControls,
  openProfileMenu,
  profileEntry,
  profileSelector,
  saveAsNew,
  saveAsNewAvailability,
} from "@/test/profile-selector";
import { renderWithQueryClient } from "@/test/query-client";
import type {
  GpuPerformanceResponse,
  GpuTuningProfile,
} from "@/features/gpus/types";
import { MemoryRouter } from "react-router-dom";
import { GpuPerformancePage } from "./gpu-performance-page";
import { GpuTuningPage } from "./gpu-tuning-page";

beforeEach(() => {
  vi.restoreAllMocks();
  vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
    connectedDaemonStatus,
  );
});

// The daemon-backed desktop API: current values, driver defaults, stored profiles, each GPU's
// current profile, and the last applied tuning values that saving takes explicit default
// choices from.
function fakeDesktopGpu(
  response: GpuPerformanceResponse = createPerformanceResponse(),
  profiles: GpuTuningProfile[] = [],
  currentProfiles: Record<string, string> = {},
) {
  return createFakeDesktop({ response, profiles, currentProfiles });
}

function renderPerformancePage() {
  return renderWithQueryClient(
    <MemoryRouter>
      <GpuPerformancePage />
    </MemoryRouter>,
  );
}

const powerRegion = () => screen.getByRole("region", { name: "Power" });
const clockRegion = () => screen.getByRole("region", { name: "Clock Offsets" });

async function saveProfile(
  user: ReturnType<typeof userEvent.setup>,
  name: string,
) {
  const dialog = await saveAsNew(user, name);
  await waitFor(() => expect(dialog).not.toBeInTheDocument());
}

it("keeps a numeric value equal to today's default fixed, while an explicit default follows a changed driver default", async () => {
  const user = userEvent.setup();
  const response = createPerformanceResponse();
  const fake = fakeDesktopGpu(response);
  renderPerformancePage();

  const power = await screen.findByRole("spinbutton", { name: "Power limit" });
  await user.clear(power);
  await user.type(power, "575");
  await user.click(
    within(powerRegion()).getByRole("button", { name: "Apply" }),
  );
  await screen.findByText("Power limit: Applied");
  expect(fake.setPowerLimit).toHaveBeenCalledWith("GPU-123", 575);
  expect(within(powerRegion()).queryByText("Uses default")).toBeNull();
  await saveProfile(user, "Fixed");
  await waitFor(() => expect(fake.profiles).toHaveLength(1));
  expect(fake.profiles[0].settings.powerLimitWatts).toEqual({
    kind: "value",
    value: 575,
  });

  await user.click(
    screen.getByRole("button", { name: "Use default power limit" }),
  );
  expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
  expect(screen.getByText("Use default (not applied)")).toBeInTheDocument();
  expect(power).toHaveValue(575);
  expect((await saveAsNewAvailability(user)).enabled).toBe(false);
  // An unapplied choice is not yet a difference from the current profile.
  expect(screen.queryByText("Changes not saved to profile")).toBeNull();

  await user.click(
    within(powerRegion()).getByRole("button", { name: "Apply" }),
  );
  await screen.findByText("Uses default");
  expect(fake.resetPowerLimit).toHaveBeenCalledTimes(1);
  expect(screen.queryByText("Unapplied changes")).toBeNull();
  expect(screen.getByText("Changes not saved to profile")).toBeInTheDocument();
  await saveProfile(user, "Default");
  await waitFor(() => expect(fake.profiles).toHaveLength(2));
  // The daemon takes the default choice from its own record of the applied request.
  expect(fake.saveProfile).toHaveBeenLastCalledWith({
    gpuId: "GPU-123",
    name: "Default",
    revision: expect.any(String),
  });
  expect(fake.profiles[1].settings).toEqual({
    powerLimitWatts: { kind: "default" },
    graphicsOffsetMHz: { kind: "value", value: 120 },
  });
  await waitFor(() =>
    expect(screen.queryByText("Changes not saved to profile")).toBeNull(),
  );

  // A driver update changes the reported default.
  response.gpus[0].power.defaultLimitWatts = 520;
  await chooseProfile(user, "Fixed");
  await waitFor(() =>
    expect(fake.applyProfileById).toHaveBeenLastCalledWith({
      profileId: "profile-1",
    }),
  );
  await waitFor(() =>
    expect(response.gpus[0].power.currentLimitWatts).toBe(575),
  );
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Fixed"));
  await chooseProfile(user, "Default");
  await waitFor(() => expect(power).toHaveValue(520));
  expect(response.gpus[0].power.currentLimitWatts).toBe(520);
  expect(within(powerRegion()).getByText("Uses default")).toBeInTheDocument();
  expect(screen.queryByText("Changes not saved to profile")).toBeNull();
});

it("treats switching between a number and Use default as an unapplied edit even when the resolved number matches", async () => {
  const user = userEvent.setup();
  const response = createPerformanceResponse({
    gpus: [createPerformanceGpu({ power: { currentLimitWatts: 575 } })],
  });
  const fake = fakeDesktopGpu(
    response,
    [
      {
        id: "fixed",
        name: "Fixed",
        gpuId: "GPU-123",
        settings: {
          powerLimitWatts: { kind: "value", value: 575 },
          graphicsOffsetMHz: { kind: "value", value: 120 },
        },
      },
    ],
    { "GPU-123": "fixed" },
  );
  renderWithQueryClient(<GpuTuningPage />);
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Fixed"));
  expect(screen.queryByText("Changes not saved to profile")).toBeNull();

  await user.click(
    screen.getByRole("button", { name: "Use default power limit" }),
  );
  expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
  expect((await saveAsNewAvailability(user)).text).toContain(
    "Apply or discard unapplied power-limit or clock-offset changes before saving a profile.",
  );
  await user.click(
    screen.getByRole("button", { name: "Discard power limit change" }),
  );
  expect(screen.queryByText("Unapplied changes")).toBeNull();
  expect(screen.queryByText("Use default (not applied)")).toBeNull();

  await user.click(
    screen.getByRole("button", { name: "Use default power limit" }),
  );
  await user.click(screen.getByRole("button", { name: "Apply all" }));
  await screen.findByText("Uses default");
  expect(fake.resetPowerLimit).toHaveBeenCalledTimes(1);
  expect(screen.getByText("Changes not saved to profile")).toBeInTheDocument();

  const power = screen.getByRole("spinbutton", { name: "Power limit" });
  await user.clear(power);
  await user.type(power, "575");
  expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
  expect((await saveAsNewAvailability(user)).enabled).toBe(false);
  await user.click(
    within(powerRegion()).getByRole("button", { name: "Apply" }),
  );
  await screen.findByText("Power limit: Applied");
  expect(fake.setPowerLimit).toHaveBeenCalledWith("GPU-123", 575);
  expect(screen.queryByText("Uses default")).toBeNull();
  expect(screen.queryByText("Changes not saved to profile")).toBeNull();
  await saveProfile(user, "Fixed again");
  await waitFor(() => expect(fake.profiles).toHaveLength(2));
  expect(fake.profiles[1].settings.powerLimitWatts).toEqual({
    kind: "value",
    value: 575,
  });
});

it("preserves explicit default choices after applying, saving, and reloading, without treating a current profile as applied", async () => {
  const user = userEvent.setup();
  const response = createPerformanceResponse({
    gpus: [createPerformanceGpu({ power: { currentLimitWatts: 575 } })],
  });
  response.gpus[0].clocks.graphics.currentOffsetMHz = 0;
  const fake = fakeDesktopGpu(
    response,
    [
      {
        id: "defaults",
        name: "Defaults",
        gpuId: "GPU-123",
        settings: {
          powerLimitWatts: { kind: "default" },
          graphicsOffsetMHz: { kind: "default" },
        },
      },
    ],
    { "GPU-123": "defaults" },
  );
  const first = renderPerformancePage();
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Defaults"));
  await openProfileMenu(user);
  expect(profileEntry("Defaults")).toHaveTextContent(
    "Power limit Use default · Graphics offset Use default",
  );
  await user.keyboard("{Escape}");
  // Equal numbers never establish default intent.
  expect(screen.getByText("Changes not saved to profile")).toBeInTheDocument();
  expect(screen.queryByText("Uses default")).toBeNull();

  await chooseProfile(user, "Defaults");
  await screen.findByText("Power limit: Applied");
  expect(fake.applyProfileById).toHaveBeenCalledWith({
    profileId: "defaults",
  });
  expect(screen.getAllByText("Uses default")).toHaveLength(2);
  await waitFor(() =>
    expect(screen.queryByText("Changes not saved to profile")).toBeNull(),
  );

  await saveProfile(user, "Copy");
  await waitFor(() => expect(fake.profiles).toHaveLength(2));
  expect(fake.profiles[1].settings).toEqual({
    powerLimitWatts: { kind: "default" },
    graphicsOffsetMHz: { kind: "default" },
  });
  first.unmount();

  renderPerformancePage();
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Copy"));
  await openProfileMenu(user);
  expect(profileEntry("Copy")).toHaveTextContent(
    "Power limit Use default · Graphics offset Use default",
  );
  await user.keyboard("{Escape}");
  expect(screen.getByText("Changes not saved to profile")).toBeInTheDocument();
});

it("blocks applying when a requested default is unavailable, without changing included or omitted settings", async () => {
  const user = userEvent.setup();
  const response = createPerformanceResponse({
    gpus: [createPerformanceGpu({ power: { defaultLimitWatts: null } })],
  });
  const fake = fakeDesktopGpu(response, [
    {
      id: "defaults",
      name: "Defaults",
      gpuId: "GPU-123",
      settings: {
        powerLimitWatts: { kind: "default" },
        graphicsOffsetMHz: { kind: "value", value: 150 },
      },
    },
  ]);
  renderPerformancePage();
  await findProfileControls();
  expect(
    await screen.findByRole("button", { name: "Use default power limit" }),
  ).toBeDisabled();
  await chooseProfile(user, "Defaults");
  const reason =
    "Power limit: the default is unavailable. Refresh the GPU readings and try again.";
  await screen.findByText(`Power limit: Not attempted — ${reason}`);
  expect(
    screen.getByText(`Graphics clock offset: Not attempted — ${reason}`),
  ).toBeInTheDocument();
  expect(response.gpus[0].power.currentLimitWatts).toBe(475);
  expect(response.gpus[0].clocks.graphics.currentOffsetMHz).toBe(120);
  expect(fake.setPowerLimit).not.toHaveBeenCalled();
  expect(fake.resetPowerLimit).not.toHaveBeenCalled();
  expect(fake.setClockOffsets).not.toHaveBeenCalled();
  expect(screen.getByText("Use default (not applied)")).toBeInTheDocument();
  expect((await saveAsNewAvailability(user)).enabled).toBe(false);

  // Retrying revalidates the retained default request through the profile boundary.
  await user.click(
    screen.getByRole("button", { name: "Retry power limit change" }),
  );
  await waitFor(() => expect(fake.applyProfile).toHaveBeenCalledTimes(1));
  expect(fake.applyProfileById).toHaveBeenCalledTimes(1);
  expect(fake.applyProfile).toHaveBeenLastCalledWith({
    gpuId: "GPU-123",
    settings: { powerLimitWatts: { kind: "default" } },
  });
  await user.click(
    screen.getByRole("button", { name: "Discard power limit change" }),
  );
  await user.click(
    screen.getByRole("button", {
      name: "Discard graphics clock offset change",
    }),
  );
  expect(screen.queryByText("Unapplied changes")).toBeNull();
  expect((await saveAsNewAvailability(user)).enabled).toBe(true);
});

it("keeps failed default offset edits for retry through Apply all while failed fan edits do not block saving", async () => {
  const user = userEvent.setup();
  const response = createPerformanceResponse();
  const fake = fakeDesktopGpu(response);
  fake.setClockOffsets.mockResolvedValueOnce({
    graphics: { status: "failed", error: "Graphics write denied" },
    memory: null,
  });
  fake.setFanSpeed.mockResolvedValue({
    success: false,
    error: "Fan write denied",
  });
  renderWithQueryClient(<GpuTuningPage />);
  await user.click(
    await screen.findByRole("button", {
      name: "Use default graphics clock offset",
    }),
  );
  expect(
    screen.getByRole("spinbutton", { name: "Graphics clock offset" }),
  ).toHaveValue(0);
  expect(
    within(clockRegion()).getByText("Use default (not applied)"),
  ).toBeInTheDocument();
  const fan = screen.getByRole("spinbutton", { name: "Fan 1 manual target" });
  await user.clear(fan);
  await user.type(fan, "65");

  await user.click(screen.getByRole("button", { name: "Apply all" }));
  await screen.findByText(
    "Graphics clock offset: Failed — Graphics write denied",
  );
  expect(
    screen.getByText("Fan 1: Failed — Fan write denied"),
  ).toBeInTheDocument();
  expect(fake.setClockOffsets).toHaveBeenCalledWith("GPU-123", "default", null);
  expect(response.gpus[0].clocks.graphics.currentOffsetMHz).toBe(120);
  expect(
    within(clockRegion()).getByText("Use default (not applied)"),
  ).toBeInTheDocument();
  expect((await saveAsNewAvailability(user)).enabled).toBe(false);

  await user.click(
    screen.getByRole("button", { name: "Retry graphics clock offset change" }),
  );
  await screen.findByText("Graphics clock offset: Applied");
  expect(within(clockRegion()).getByText("Uses default")).toBeInTheDocument();
  expect(
    screen.getByText("Fan 1: Failed — Fan write denied"),
  ).toBeInTheDocument();
  expect((await saveAsNewAvailability(user)).enabled).toBe(true);
  await saveProfile(user, "Q");
  await waitFor(() => expect(fake.profiles).toHaveLength(1));
  expect(fake.profiles[0].settings).toEqual({
    powerLimitWatts: { kind: "value", value: 475 },
    graphicsOffsetMHz: { kind: "default" },
  });
});

it("does not request a power default that falls outside the current range after the edit was made", async () => {
  const user = userEvent.setup();
  const response = createPerformanceResponse();
  const fake = fakeDesktopGpu(response);
  renderWithQueryClient(<GpuTuningPage />);
  await user.click(
    await screen.findByRole("button", { name: "Use default power limit" }),
  );
  response.gpus[0].power.maxLimitWatts = 500;
  await waitFor(() =>
    expect(within(powerRegion()).getByText("400-500 W allowed")).toBeVisible(),
  );
  await user.click(screen.getByRole("button", { name: "Apply all" }));
  await screen.findByText(
    /Power limit: Not attempted — The default for this control is unavailable or outside its current range/,
  );
  expect(fake.resetPowerLimit).not.toHaveBeenCalled();
  expect(response.gpus[0].power.currentLimitWatts).toBe(475);
  expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
});
