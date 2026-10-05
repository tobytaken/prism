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
  profileAvailability,
  profileSelector,
  saveAsNew,
  saveAsNewAvailability,
} from "@/test/profile-selector";
import { renderWithQueryClient } from "@/test/query-client";
import type {
  GpuPerformanceDevice,
  GpuTuningProfile,
  ProfileApplicationResponse,
} from "@/features/gpus/types";
import { GpuTuningPage } from "./gpu-tuning-page";
import { GpuPerformancePage } from "./gpu-performance-page";
import { MemoryRouter, RouterProvider } from "react-router-dom";
import { appRouter } from "@/app/router";

function renderPerformancePage() {
  return renderWithQueryClient(
    <MemoryRouter>
      <GpuPerformancePage />
    </MemoryRouter>,
  );
}

function quiet(
  settings: GpuTuningProfile["settings"] = {
    powerLimitWatts: { kind: "value", value: 450 },
  },
): GpuTuningProfile {
  return { id: "quiet", name: "Quiet", gpuId: "GPU-123", settings };
}

beforeEach(() => {
  vi.restoreAllMocks();
  vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
    connectedDaemonStatus,
  );
});

it("saves a profile, changes live values, and applies the current profile again without touching omitted settings", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop();
  const savedConfig = vi.spyOn(window.prism.gpu, "applyConfig");
  renderWithQueryClient(<RouterProvider router={appRouter} />);
  await user.click(screen.getByRole("link", { name: "Performance" }));
  await saveAsNew(user, "Quiet");
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));
  const power = screen.getByRole("spinbutton", { name: "Power limit" });
  await user.clear(power);
  await user.type(power, "500");
  await user.click(
    within(screen.getByRole("region", { name: "Power" })).getByRole("button", {
      name: "Apply",
    }),
  );
  await screen.findByText("Changes not saved to profile");
  await chooseProfile(user, "Quiet");
  await waitFor(() => expect(power).toHaveValue(475));
  expect(fake.applyProfileById).toHaveBeenCalledExactlyOnceWith({
    profileId: "profile-1",
  });
  await waitFor(() =>
    expect(
      screen.queryByText("Changes not saved to profile"),
    ).not.toBeInTheDocument(),
  );
  expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
  expect(
    screen.getByText("Graphics clock offset: Applied"),
  ).toBeInTheDocument();
  expect(fake.gpu.cooling.fans[0].mode).toBe("auto");
  expect(fake.gpu.clocks.memory.currentOffsetMHz).toBeNull();
  expect(savedConfig).not.toHaveBeenCalled();
  expect(fake.profiles[0].settings.powerLimitWatts).toEqual({
    kind: "value",
    value: 475,
  });
});

it("keeps failed and unattempted profile instructions, retries through backend validation, and permits explicit discard", async () => {
  const user = userEvent.setup();
  const response = createPerformanceResponse();
  response.gpus[0].clocks.memory = {
    ...response.gpus[0].clocks.graphics,
    currentOffsetMHz: 0,
  };
  const fake = createFakeDesktop({
    response,
    profiles: [
      quiet({
        powerLimitWatts: { kind: "value", value: 450 },
        graphicsOffsetMHz: { kind: "value", value: 150 },
        memoryOffsetMHz: { kind: "value", value: 80 },
      }),
    ],
    currentProfiles: { "GPU-123": "quiet" },
  });
  fake.failures.graphics = "Graphics write denied";
  renderPerformancePage();
  await screen.findByText("Changes not saved to profile");
  await chooseProfile(user, "Quiet");
  await screen.findByText(
    "Graphics clock offset: Failed — Graphics write denied",
  );
  expect(screen.getByText("Power limit: Applied")).toBeInTheDocument();
  expect(
    screen.getByText(/Memory clock offset: Not attempted/),
  ).toBeInTheDocument();
  expect(screen.getByRole("spinbutton", { name: "Power limit" })).toHaveValue(
    450,
  );
  expect(
    screen.getByRole("spinbutton", { name: "Graphics clock offset" }),
  ).toHaveValue(150);
  // Applied only in part, the profile no longer describes the GPU.
  await waitFor(() =>
    expect(profileSelector()).toHaveTextContent("No current profile"),
  );
  expect((await saveAsNewAvailability(user)).enabled).toBe(false);
  expect((await profileAvailability(user, "Quiet")).enabled).toBe(false);
  fake.gpu.clocks.memory.maxOffsetMHz = 50;
  await user.click(
    screen.getByRole("button", { name: "Retry memory clock offset change" }),
  );
  await screen.findByText(
    /Memory clock offset: Not attempted.*outside the current allowed range -200 to 50/,
  );
  expect(fake.applyProfile).toHaveBeenLastCalledWith({
    gpuId: "GPU-123",
    settings: { memoryOffsetMHz: { kind: "value", value: 80 } },
  });
  expect(fake.gpu.clocks.memory.currentOffsetMHz).toBe(0);
  fake.gpu.clocks.memory.maxOffsetMHz = 200;
  await user.click(
    screen.getByRole("button", { name: "Retry memory clock offset change" }),
  );
  await screen.findByText("Memory clock offset: Applied");
  expect((await saveAsNewAvailability(user)).enabled).toBe(false);
  await user.click(
    screen.getByRole("button", {
      name: "Discard graphics clock offset change",
    }),
  );
  expect(
    screen.getByRole("spinbutton", { name: "Graphics clock offset" }),
  ).toHaveValue(120);
  expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
  expect(profileSelector()).toHaveTextContent("No current profile");
  expect((await saveAsNewAvailability(user)).enabled).toBe(true);
  expect(fake.profiles[0].settings.graphicsOffsetMHz).toEqual({
    kind: "value",
    value: 150,
  });
});

it.each(["power limit", "fan"])(
  "blocks applying a profile until outstanding %s edits are applied or explicitly discarded",
  async (setting) => {
    const user = userEvent.setup();
    const fake = createFakeDesktop({ profiles: [quiet()] });
    renderPerformancePage();
    await findProfileControls();
    const input = screen.getByRole("spinbutton", {
      name: setting === "fan" ? "Fan 1 manual target" : "Power limit",
    });
    await user.clear(input);
    await user.type(input, setting === "fan" ? "65" : "500");
    const blocked = await profileAvailability(user, "Quiet");
    expect(blocked.enabled).toBe(false);
    expect(blocked.text).toContain(
      "Apply or discard outstanding tuning changes before applying a profile.",
    );
    expect(input).toHaveValue(setting === "fan" ? 65 : 500);
    expect(fake.applyProfileById).not.toHaveBeenCalled();
    await user.click(
      screen.getByRole("button", {
        name:
          setting === "fan"
            ? "Discard fan 1 change"
            : "Discard power limit change",
      }),
    );
    expect((await profileAvailability(user, "Quiet")).enabled).toBe(true);
  },
);

it.each<{
  label: string;
  change: (gpu: GpuPerformanceDevice) => void;
  reason: string;
}>([
  {
    label: "unknown support",
    change: (gpu) => {
      gpu.clocks.graphics.support = "unknown";
      gpu.clocks.graphics.reason = null;
    },
    reason:
      "Graphics clock offset support is unknown. Refresh the GPU readings and try again.",
  },
  {
    label: "an unsupported setting",
    change: (gpu) => {
      gpu.clocks.graphics.support = "unsupported";
      gpu.clocks.graphics.reason = null;
    },
    reason:
      "Graphics clock offset is unsupported. Choose a compatible profile.",
  },
  {
    label: "an unavailable range",
    change: (gpu) => {
      gpu.clocks.graphics.minOffsetMHz = null;
      gpu.clocks.graphics.maxOffsetMHz = null;
    },
    reason:
      "Graphics clock offset: the current allowed range is unavailable. Refresh the GPU readings and try again.",
  },
  {
    label: "a range problem alongside another blocker",
    change: (gpu) => {
      gpu.power.minLimitWatts = 500;
      gpu.clocks.graphics.support = "unsupported";
      gpu.clocks.graphics.reason = null;
    },
    reason:
      "Graphics clock offset is unsupported. Choose a compatible profile. Power limit: requested 450 is outside the current allowed range 500 to 600. Discard this change and load the profile again to review a correction.",
  },
])(
  "shows fresh backend validation for $label without mutating or silently correcting",
  async ({ change, reason }) => {
    const user = userEvent.setup();
    const fake = createFakeDesktop({
      profiles: [
        quiet({
          powerLimitWatts: { kind: "value", value: 450 },
          graphicsOffsetMHz: { kind: "value", value: 150 },
        }),
      ],
    });
    change(fake.gpu);
    renderPerformancePage();
    await chooseProfile(user, "Quiet");
    await screen.findByText(`Power limit: Not attempted — ${reason}`);
    expect(
      screen.getByText(`Graphics clock offset: Not attempted — ${reason}`),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("region", { name: "Review profile corrections" }),
    ).not.toBeInTheDocument();
    expect(fake.gpu.power.currentLimitWatts).toBe(475);
    expect(fake.gpu.clocks.graphics.currentOffsetMHz).toBe(120);
    expect(screen.getByRole("spinbutton", { name: "Power limit" })).toHaveValue(
      450,
    );
    expect((await saveAsNewAvailability(user)).enabled).toBe(false);
    expect(profileSelector()).toHaveTextContent("No current profile");
    expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
    expect(fake.profiles[0].settings.powerLimitWatts).toEqual({
      kind: "value",
      value: 450,
    });
    expect(fake.liveWrites()).toBe(0);
  },
);

it("applies only on the stable owner after identical-model GPUs reorder, and retains absent or unidentifiable owners as unavailable", async () => {
  const user = userEvent.setup();
  const owner = createPerformanceGpu();
  const other = createPerformanceGpu({ id: "GPU-456", stableId: "GPU-456" });
  const response = createPerformanceResponse({ gpus: [owner, other] });
  createFakeDesktop({ response, profiles: [quiet()] });
  renderPerformancePage();
  await findProfileControls();
  response.gpus = [other, owner];
  await waitFor(() =>
    expect(
      screen.getAllByRole("group", { name: /profiles \(/ })[0],
    ).toHaveAccessibleName(/GPU-456/),
  );
  await chooseProfile(user, "Quiet");
  await screen.findByText("Power limit: Applied");
  expect(owner.power.currentLimitWatts).toBe(450);
  expect(other.power.currentLimitWatts).toBe(475);
  response.gpus = [other];
  const unavailable = await screen.findByRole("region", {
    name: "Unavailable GPU profiles",
  });
  expect(
    within(unavailable).getByRole("button", { name: "Apply profile" }),
  ).toBeDisabled();
  expect(
    within(unavailable).getByText(/Owning GPU absent.*GPU-123/),
  ).toBeInTheDocument();
  owner.stableId = null;
  owner.id = "nvidia-0";
  response.gpus = [other, owner];
  const unidentified = await findProfileControls("nvidia-0");
  expect(
    within(unavailable).getByRole("button", { name: "Apply profile" }),
  ).toBeDisabled();
  expect(within(unidentified).queryByRole("button")).not.toBeInTheDocument();
});

it("applies only included instructions even when omitted controls have unknown support or unavailable ranges", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    response: createPerformanceResponse({
      gpus: [
        createPerformanceGpu({
          clocks: {
            graphics: {
              support: "unknown",
              minOffsetMHz: null,
              maxOffsetMHz: null,
            },
            memory: { support: "unknown" },
          },
        }),
      ],
    }),
    profiles: [quiet()],
  });
  renderPerformancePage();
  await chooseProfile(user, "Quiet");
  await screen.findByText("Power limit: Applied");
  expect(fake.applyProfileById).toHaveBeenCalledWith({ profileId: "quiet" });
  expect(fake.gpu.power.currentLimitWatts).toBe(450);
  expect(fake.gpu.clocks.graphics.currentOffsetMHz).toBe(120);
  expect(
    screen.queryByText("Changes not saved to profile"),
  ).not.toBeInTheDocument();
  expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
});

it("keeps profile instructions and refreshes actual values when transport acknowledgement is lost", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({ profiles: [quiet()] });
  fake.applyProfileById.mockImplementation(async () => {
    fake.gpu.power.currentLimitWatts = 450;
    throw new Error("Connection lost");
  });
  renderPerformancePage();
  await chooseProfile(user, "Quiet");
  await screen.findByText("Power limit: Outcome unknown — Connection lost");
  expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
  expect(
    screen.queryByText("Changes not saved to profile"),
  ).not.toBeInTheDocument();
  expect((await saveAsNewAvailability(user)).enabled).toBe(false);
  expect(
    screen.getByRole("button", { name: "Retry power limit change" }),
  ).toBeEnabled();
  await user.click(
    screen.getByRole("button", { name: "Discard power limit change" }),
  );
  expect(screen.getByRole("spinbutton", { name: "Power limit" })).toHaveValue(
    450,
  );
});

it.each(["Apply", "Apply all"])(
  "revalidates unresolved profile instructions when retried using %s",
  async (button) => {
    const user = userEvent.setup();
    const fake = createFakeDesktop({ profiles: [quiet()] });
    const blocked: ProfileApplicationResponse = {
      power: { status: "unattempted", error: "Power range changed" },
      graphics: null,
      memory: null,
    };
    fake.applyProfileById.mockResolvedValue(blocked);
    fake.applyProfile.mockResolvedValue(blocked);
    if (button === "Apply all") renderWithQueryClient(<GpuTuningPage />);
    else renderPerformancePage();
    await chooseProfile(user, "Quiet");
    await screen.findByText("Power limit: Not attempted — Power range changed");
    const action =
      button === "Apply"
        ? within(screen.getByRole("region", { name: "Power" })).getByRole(
            "button",
            { name: "Apply" },
          )
        : screen.getByRole("button", { name: "Apply all" });
    await user.click(action);
    await waitFor(() =>
      expect(fake.applyProfile).toHaveBeenCalledExactlyOnceWith({
        gpuId: "GPU-123",
        settings: { powerLimitWatts: { kind: "value", value: 450 } },
      }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Retry power limit change" }),
      ).toBeEnabled(),
    );
    expect(fake.setPowerLimit).not.toHaveBeenCalled();
    expect(fake.gpu.power.currentLimitWatts).toBe(475);
    expect(
      screen.getByText("Power limit: Not attempted — Power range changed"),
    ).toBeInTheDocument();
  },
);

it("retains unresolved profile instructions when the owner disappears and allows explicit discard", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({ profiles: [quiet()] });
  fake.failures.power = "GPU lost";
  renderPerformancePage();
  await chooseProfile(user, "Quiet");
  await screen.findByText("Power limit: Failed — GPU lost");
  fake.response.gpus = [];
  await screen.findByText(
    "Edits for GPU-123 are retained while this GPU is unavailable.",
  );
  await user.click(
    screen.getByRole("button", {
      name: "Discard power limit change for GPU-123",
    }),
  );
  expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Apply profile" })).toBeDisabled();
  expect(fake.profiles[0].settings.powerLimitWatts).toEqual({
    kind: "value",
    value: 450,
  });
});
