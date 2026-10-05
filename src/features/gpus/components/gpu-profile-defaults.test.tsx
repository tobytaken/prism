import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import {
  connectedDaemonStatus,
  createPerformanceGpu,
  createPerformanceResponse,
} from "@/test/gpu-fixtures";
import { createFakeDesktop } from "@/test/fake-desktop";
import { renderWithQueryClient } from "@/test/query-client";
import type {
  GpuPerformanceResponse,
  GpuTuningProfile,
} from "@/features/gpus/types";
import { GpuTuningPage } from "./gpu-tuning-page";

beforeEach(() => {
  vi.restoreAllMocks();
  vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
    connectedDaemonStatus,
  );
});

// The daemon-backed desktop API: current values, driver defaults, stored profiles, and the
// last applied tuning values that saving takes explicit default choices from.
function fakeDesktopGpu(
  response: GpuPerformanceResponse = createPerformanceResponse(),
  profiles: GpuTuningProfile[] = [],
) {
  return createFakeDesktop({ response, profiles });
}

const powerRegion = () => screen.getByRole("region", { name: "Power" });
const clockRegion = () => screen.getByRole("region", { name: "Clock Offsets" });
const profilePanel = () => screen.getByRole("region", { name: /profiles/ });

async function saveProfile(
  user: ReturnType<typeof userEvent.setup>,
  name: string,
) {
  const input = screen.getByRole("textbox", { name: "Profile name" });
  await user.clear(input);
  await user.type(input, name);
  await user.click(screen.getByRole("button", { name: "Save profile" }));
}

it("keeps a numeric value equal to today's default fixed, while an explicit default follows a changed driver default", async () => {
  const user = userEvent.setup();
  const response = createPerformanceResponse();
  const fake = fakeDesktopGpu(response);
  renderWithQueryClient(<GpuTuningPage />);

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
  expect(screen.getByRole("button", { name: "Save profile" })).toBeDisabled();
  // An unapplied choice is not yet a difference from the selected profile.
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
  await user.selectOptions(
    screen.getByRole("combobox", { name: "Saved profile" }),
    "profile-1",
  );
  await user.click(screen.getByRole("button", { name: "Load profile" }));
  await waitFor(() =>
    expect(fake.applyProfileById).toHaveBeenLastCalledWith({
      profileId: "profile-1",
    }),
  );
  await waitFor(() =>
    expect(response.gpus[0].power.currentLimitWatts).toBe(575),
  );
  await user.selectOptions(
    screen.getByRole("combobox", { name: "Saved profile" }),
    "profile-2",
  );
  await user.click(screen.getByRole("button", { name: "Load profile" }));
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
  const fake = fakeDesktopGpu(response, [
    {
      id: "fixed",
      name: "Fixed",
      gpuId: "GPU-123",
      settings: {
        powerLimitWatts: { kind: "value", value: 575 },
        graphicsOffsetMHz: { kind: "value", value: 120 },
      },
    },
  ]);
  renderWithQueryClient(<GpuTuningPage />);
  await screen.findByRole("option", { name: "Fixed" });
  await user.selectOptions(screen.getByRole("combobox"), "fixed");
  expect(screen.queryByText("Changes not saved to profile")).toBeNull();

  await user.click(
    screen.getByRole("button", { name: "Use default power limit" }),
  );
  expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
  expect(
    screen.getByText(
      "Apply or discard unapplied power-limit or clock-offset changes before saving a profile.",
    ),
  ).toBeInTheDocument();
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
  expect(screen.getByRole("button", { name: "Save profile" })).toBeDisabled();
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

it("preserves explicit default choices after loading, saving, and reloading, without treating selection as applied", async () => {
  const user = userEvent.setup();
  const response = createPerformanceResponse({
    gpus: [createPerformanceGpu({ power: { currentLimitWatts: 575 } })],
  });
  response.gpus[0].clocks.graphics.currentOffsetMHz = 0;
  const fake = fakeDesktopGpu(response, [
    {
      id: "defaults",
      name: "Defaults",
      gpuId: "GPU-123",
      settings: {
        powerLimitWatts: { kind: "default" },
        graphicsOffsetMHz: { kind: "default" },
      },
    },
  ]);
  const first = renderWithQueryClient(<GpuTuningPage />);
  await screen.findByRole("option", { name: "Defaults" });
  await user.selectOptions(screen.getByRole("combobox"), "defaults");
  expect(
    within(profilePanel()).getAllByText("Use default", { exact: false }),
  ).toHaveLength(2);
  // Equal numbers never establish default intent.
  expect(screen.getByText("Changes not saved to profile")).toBeInTheDocument();
  expect(screen.queryByText("Uses default")).toBeNull();

  await user.click(screen.getByRole("button", { name: "Load profile" }));
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

  renderWithQueryClient(<GpuTuningPage />);
  await screen.findByRole("option", { name: "Copy" });
  await user.selectOptions(screen.getByRole("combobox"), "profile-2");
  expect(
    within(profilePanel()).getAllByText("Use default", { exact: false }),
  ).toHaveLength(2);
  expect(screen.getByText("Changes not saved to profile")).toBeInTheDocument();
});

it("blocks loading when a requested default is unavailable, without changing included or omitted settings", async () => {
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
  renderWithQueryClient(<GpuTuningPage />);
  await screen.findByRole("option", { name: "Defaults" });
  expect(
    screen.getByRole("button", { name: "Use default power limit" }),
  ).toBeDisabled();
  await user.selectOptions(screen.getByRole("combobox"), "defaults");
  await user.click(screen.getByRole("button", { name: "Load profile" }));
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
  expect(screen.getByRole("button", { name: "Save profile" })).toBeDisabled();

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
  expect(screen.getByRole("button", { name: "Save profile" })).toBeDisabled();
  await user.type(screen.getByRole("textbox", { name: "Profile name" }), "X");
  expect(screen.getByRole("button", { name: "Save profile" })).toBeEnabled();
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
  await user.type(screen.getByRole("textbox", { name: "Profile name" }), "Q");
  expect(screen.getByRole("button", { name: "Save profile" })).toBeDisabled();

  await user.click(
    screen.getByRole("button", { name: "Retry graphics clock offset change" }),
  );
  await screen.findByText("Graphics clock offset: Applied");
  expect(within(clockRegion()).getByText("Uses default")).toBeInTheDocument();
  expect(
    screen.getByText("Fan 1: Failed — Fan write denied"),
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save profile" })).toBeEnabled();
  await user.click(screen.getByRole("button", { name: "Save profile" }));
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
