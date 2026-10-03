import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import {
  connectedDaemonStatus,
  createPerformanceGpu,
  createPerformanceResponse,
} from "@/test/gpu-fixtures";
import { renderWithQueryClient } from "@/test/query-client";
import { captureProfileSettings } from "@/features/gpus/lib/tuning-profiles";
import type {
  GpuPerformanceResponse,
  GpuTuningProfile,
  ProfileApplicationResponse,
  TuningOutcome,
} from "@/features/gpus/types";
import { GpuTuningPage } from "./gpu-tuning-page";

beforeEach(() => {
  vi.restoreAllMocks();
  vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
    connectedDaemonStatus,
  );
});

// A stateful desktop API: current values, driver defaults, and stored profiles.
function fakeDesktopGpu(
  response: GpuPerformanceResponse = createPerformanceResponse(),
  profiles: GpuTuningProfile[] = [],
) {
  const gpu = () => response.gpus[0];
  const succeeded: TuningOutcome = { status: "succeeded", error: null };
  vi.spyOn(window.prism.gpu, "getPerformance").mockImplementation(async () =>
    structuredClone(response),
  );
  vi.spyOn(window.prism.gpu, "listProfiles").mockImplementation(async () =>
    structuredClone(profiles),
  );
  const save = vi
    .spyOn(window.prism.gpu, "saveProfile")
    .mockImplementation(async ({ gpuId, name, appliedDefaults }) => {
      const owner = response.gpus.find((device) => device.id === gpuId)!;
      const profile: GpuTuningProfile = {
        id: `profile-${profiles.length + 1}`,
        name,
        gpuId: owner.stableId!,
        settings: captureProfileSettings(owner, appliedDefaults),
      };
      profiles.push(profile);
      return structuredClone(profile);
    });
  const setPower = vi
    .spyOn(window.prism.gpu, "setPowerLimit")
    .mockImplementation(async (_gpuId, watts) => {
      gpu().power.currentLimitWatts = watts;
      return { success: true, error: null };
    });
  const resetPower = vi
    .spyOn(window.prism.gpu, "resetPowerLimit")
    .mockImplementation(async () => {
      gpu().power.currentLimitWatts = gpu().power.defaultLimitWatts;
      return { success: true, error: null };
    });
  const setClocks = vi
    .spyOn(window.prism.gpu, "setClockOffsets")
    .mockImplementation(async (_gpuId, graphics, memory) => {
      if (graphics != null) gpu().clocks.graphics.currentOffsetMHz = graphics;
      if (memory != null) gpu().clocks.memory.currentOffsetMHz = memory;
      return {
        graphics: graphics == null ? null : succeeded,
        memory: memory == null ? null : succeeded,
      };
    });
  const apply = vi
    .spyOn(window.prism.gpu, "applyProfile")
    .mockImplementation(async ({ settings }) => {
      const { powerLimitWatts, graphicsOffsetMHz } = settings;
      const power =
        powerLimitWatts?.kind === "default"
          ? gpu().power.defaultLimitWatts
          : powerLimitWatts?.value;
      const graphics =
        graphicsOffsetMHz?.kind === "default" ? 0 : graphicsOffsetMHz?.value;
      const outcome = (included: unknown, value: TuningOutcome) =>
        included ? value : null;
      if (powerLimitWatts && power == null) {
        const blocked: TuningOutcome = {
          status: "unattempted",
          error:
            "Power limit: the default is unavailable. Refresh the GPU readings and try again.",
        };
        return {
          power: blocked,
          graphics: outcome(graphicsOffsetMHz, blocked),
          memory: null,
        } satisfies ProfileApplicationResponse;
      }
      if (power != null) gpu().power.currentLimitWatts = power;
      if (graphics != null) gpu().clocks.graphics.currentOffsetMHz = graphics;
      return {
        power: outcome(powerLimitWatts, succeeded),
        graphics: outcome(graphicsOffsetMHz, succeeded),
        memory: null,
      };
    });
  return { profiles, save, setPower, resetPower, setClocks, apply };
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
  expect(fake.setPower).toHaveBeenCalledWith("GPU-123", 575);
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
  expect(fake.resetPower).toHaveBeenCalledTimes(1);
  expect(screen.queryByText("Unapplied changes")).toBeNull();
  expect(screen.getByText("Changes not saved to profile")).toBeInTheDocument();
  await saveProfile(user, "Default");
  await waitFor(() => expect(fake.profiles).toHaveLength(2));
  expect(fake.save).toHaveBeenLastCalledWith({
    gpuId: "GPU-123",
    name: "Default",
    appliedDefaults: ["powerLimitWatts"],
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
    expect(fake.apply).toHaveBeenLastCalledWith({
      gpuId: "GPU-123",
      settings: {
        powerLimitWatts: { kind: "value", value: 575 },
        graphicsOffsetMHz: { kind: "value", value: 120 },
      },
    }),
  );
  expect(response.gpus[0].power.currentLimitWatts).toBe(575);
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
  expect(fake.resetPower).toHaveBeenCalledTimes(1);
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
  expect(fake.setPower).toHaveBeenCalledWith("GPU-123", 575);
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
  expect(fake.apply).toHaveBeenCalledWith({
    gpuId: "GPU-123",
    settings: {
      powerLimitWatts: { kind: "default" },
      graphicsOffsetMHz: { kind: "default" },
    },
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
  expect(fake.setPower).not.toHaveBeenCalled();
  expect(fake.resetPower).not.toHaveBeenCalled();
  expect(fake.setClocks).not.toHaveBeenCalled();
  expect(screen.getByText("Use default (not applied)")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save profile" })).toBeDisabled();

  // Retrying revalidates the retained default request through the profile boundary.
  await user.click(
    screen.getByRole("button", { name: "Retry power limit change" }),
  );
  await waitFor(() => expect(fake.apply).toHaveBeenCalledTimes(2));
  expect(fake.apply).toHaveBeenLastCalledWith({
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
  fake.setClocks.mockResolvedValueOnce({
    graphics: { status: "failed", error: "Graphics write denied" },
    memory: null,
  });
  vi.spyOn(window.prism.gpu, "setFanSpeed").mockResolvedValue({
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
  expect(fake.setClocks).toHaveBeenCalledWith("GPU-123", 0, null);
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
