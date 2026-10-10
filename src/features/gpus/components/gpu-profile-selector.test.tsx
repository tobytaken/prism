import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, expect, it, vi } from "vitest";

import type { GpuTuningProfile } from "@/features/gpus/types";
import { createFakeDesktop } from "@/test/fake-desktop";
import {
  connectedDaemonStatus,
  createPerformanceGpu,
  createPerformanceResponse,
} from "@/test/gpu-fixtures";
import {
  chooseProfile,
  closeProfileMenu,
  findProfileControls,
  openProfileMenu,
  profileEntry,
  profileSelector,
  saveAsNew,
  saveToCurrent,
  switchWithoutSaving,
} from "@/test/profile-selector";
import { renderWithQueryClient } from "@/test/query-client";
import { GpuPerformancePage } from "./gpu-performance-page";

const STALE =
  "The GPU's values changed since you last looked. Review them and save again.";

const quiet: GpuTuningProfile = {
  id: "quiet",
  name: "Quiet",
  gpuId: "GPU-123",
  settings: {
    powerLimitWatts: { kind: "value", value: 450 },
    graphicsOffsetMHz: { kind: "default" },
  },
};
const loud: GpuTuningProfile = {
  id: "loud",
  name: "Loud",
  gpuId: "GPU-123",
  settings: { powerLimitWatts: { kind: "value", value: 650 } },
};

// Reported 475 W differs from Quiet's saved 450 W; Loud requests 500 W.
function createDesktopWithProfileDifference() {
  return createFakeDesktop({
    profiles: [
      {
        ...quiet,
        settings: { powerLimitWatts: { kind: "value", value: 450 } },
      },
      { ...loud, settings: { powerLimitWatts: { kind: "value", value: 500 } } },
    ],
    currentProfiles: { "GPU-123": "quiet" },
  });
}

function renderPerformancePage() {
  return renderWithQueryClient(
    <MemoryRouter>
      <GpuPerformancePage />
    </MemoryRouter>,
  );
}

beforeEach(() => {
  vi.restoreAllMocks();
  vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
    connectedDaemonStatus,
  );
});

it("lists this GPU's profiles with their values, and choosing one applies it and checks it as the current profile", async () => {
  const user = userEvent.setup();
  const other: GpuTuningProfile = { ...quiet, id: "other", gpuId: "GPU-456" };
  const fake = createFakeDesktop({
    response: createPerformanceResponse({
      gpus: [
        createPerformanceGpu(),
        createPerformanceGpu({ id: "GPU-456", stableId: "GPU-456" }),
      ],
    }),
    profiles: [quiet, other],
  });
  renderPerformancePage();
  await findProfileControls();
  expect(profileSelector()).toHaveTextContent("No current profile");

  const menu = await openProfileMenu(user);
  expect(
    within(menu)
      .getAllByRole("menuitemradio")
      .map((entry) => entry.textContent),
  ).toEqual(["Quiet Power limit 450 W · Graphics offset Use default"]);
  expect(profileEntry("Quiet")).not.toBeChecked();
  await user.click(profileEntry("Quiet"));

  await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));
  expect(fake.applyProfileById).toHaveBeenCalledExactlyOnceWith({
    profileId: "quiet",
  });
  expect(fake.gpu.power.currentLimitWatts).toBe(450);
  await openProfileMenu(user);
  expect(profileEntry("Quiet")).toBeChecked();
  expect(profileSelector("GPU-456")).toHaveTextContent("No current profile");
});

it("leaves the GPU, the profile and the current profile unchanged when the range review is canceled", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    profiles: [quiet, loud],
    currentProfiles: { "GPU-123": "quiet" },
  });
  renderPerformancePage();
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));

  await chooseProfile(user, "Loud");
  await switchWithoutSaving(user);
  const review = await screen.findByRole("region", {
    name: "Review profile corrections",
  });
  expect(
    within(review).getByText(
      "Power limit: saved 650 W → current maximum 600 W",
    ),
  ).toBeInTheDocument();
  await user.click(within(review).getByRole("button", { name: "Cancel" }));

  expect(
    screen.queryByRole("region", { name: "Review profile corrections" }),
  ).not.toBeInTheDocument();
  expect(profileSelector()).toHaveTextContent("Quiet");
  expect(fake.currentProfiles).toEqual({ "GPU-123": "quiet" });
  expect(fake.profiles[1].settings).toEqual(loud.settings);
  expect(fake.gpu.power.currentLimitWatts).toBe(475);
  expect(fake.correctAndApplyProfile).not.toHaveBeenCalled();
  expect(fake.liveWrites()).toBe(0);
  await openProfileMenu(user);
  expect(profileEntry("Quiet")).toBeChecked();
  expect(profileEntry("Loud")).not.toBeChecked();
});

it("keeps the previous current profile when an application is blocked, and clears it after a partial one", async () => {
  const user = userEvent.setup();
  const both: GpuTuningProfile = {
    id: "both",
    name: "Both",
    gpuId: "GPU-123",
    settings: {
      powerLimitWatts: { kind: "value", value: 500 },
      graphicsOffsetMHz: { kind: "value", value: 150 },
    },
  };
  const fake = createFakeDesktop({
    profiles: [quiet, both],
    currentProfiles: { "GPU-123": "quiet" },
  });
  fake.gpu.clocks.graphics.support = "unknown";
  fake.gpu.clocks.graphics.reason = null;
  renderPerformancePage();
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));

  await chooseProfile(user, "Both");
  await switchWithoutSaving(user);
  await screen.findByText(/Power limit: Not attempted/);
  expect(profileSelector()).toHaveTextContent("Quiet");
  await user.click(
    screen.getByRole("button", { name: "Discard power limit change" }),
  );
  await user.click(
    screen.getByRole("button", {
      name: "Discard graphics clock offset change",
    }),
  );

  fake.gpu.clocks.graphics.support = "supported";
  fake.failures.graphics = "Graphics write denied";
  await chooseProfile(user, "Both");
  await switchWithoutSaving(user);
  await screen.findByText(
    "Graphics clock offset: Failed — Graphics write denied",
  );
  await waitFor(() =>
    expect(profileSelector()).toHaveTextContent("No current profile"),
  );
  expect(fake.gpu.power.currentLimitWatts).toBe(500);
});

it("Save to the current profile overwrites it with the applied values, and Save as new creates a new current profile", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    profiles: [quiet],
    currentProfiles: { "GPU-123": "quiet" },
  });
  renderPerformancePage();
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));

  await saveToCurrent(user);
  await waitFor(() =>
    expect(fake.saveProfileToCurrent).toHaveBeenCalledTimes(1),
  );
  expect(fake.saveProfileToCurrent.mock.calls[0][0].gpuId).toBe("GPU-123");
  await waitFor(() =>
    expect(fake.profiles[0].settings).toEqual({
      cooling: { arrangement: "shared", setting: { mode: "automatic" } },
      powerLimitWatts: { kind: "value", value: 475 },
      graphicsOffsetMHz: { kind: "value", value: 120 },
    }),
  );
  expect(fake.profiles).toHaveLength(1);

  const dialog = await saveAsNew(user, "  Warmer ");
  await waitFor(() => expect(dialog).not.toBeInTheDocument());
  expect(fake.saveProfile).toHaveBeenCalledExactlyOnceWith({
    gpuId: "GPU-123",
    name: "Warmer",
    revision: expect.any(String),
  });
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Warmer"));
  expect(fake.profiles.map((profile) => profile.name)).toEqual([
    "Quiet",
    "Warmer",
  ]);
  await openProfileMenu(user);
  expect(profileEntry("Warmer")).toBeChecked();
  expect(
    screen.getByRole("menuitem", { name: 'Save to "Warmer"' }),
  ).toBeEnabled();
});

it("shows the current profile's saved values above the controls and saves reported values from that note", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    profiles: [
      {
        ...quiet,
        settings: {
          powerLimitWatts: { kind: "value", value: 450 },
          graphicsOffsetMHz: { kind: "default" },
        },
      },
    ],
    currentProfiles: { "GPU-123": "quiet" },
  });
  renderPerformancePage();
  const note = await screen.findByRole("region", {
    name: "Current profile saved values",
  });
  expect(profileSelector()).toHaveTextContent("Changes not saved to profile");
  expect(note).toHaveTextContent('Saved in "Quiet":');
  expect(note).toHaveTextContent(
    "Power limit 450 W · Graphics offset Use default",
  );
  expect(screen.getByRole("spinbutton", { name: "Power limit" })).toHaveValue(
    475,
  );

  await user.click(
    within(note).getByRole("button", { name: 'Save to "Quiet"' }),
  );
  await waitFor(() => expect(note).not.toBeInTheDocument());
  expect(fake.profiles[0].settings).toEqual({
    cooling: { arrangement: "shared", setting: { mode: "automatic" } },
    powerLimitWatts: { kind: "value", value: 475 },
    graphicsOffsetMHz: { kind: "value", value: 120 },
  });
  expect(profileSelector()).not.toHaveTextContent(
    "Changes not saved to profile",
  );
  expect(fake.liveWrites()).toBe(0);
});

it("shows the reported power limit beside an unapplied edit and disables profiles with the reason in the menu", async () => {
  const user = userEvent.setup();
  const matching: GpuTuningProfile = {
    ...quiet,
    settings: { powerLimitWatts: { kind: "value", value: 475 } },
  };
  const fake = createFakeDesktop({
    profiles: [matching, loud],
    currentProfiles: { "GPU-123": "quiet" },
  });
  renderPerformancePage();
  const power = await screen.findByRole("spinbutton", { name: "Power limit" });
  await user.clear(power);
  await user.type(power, "500");

  expect(screen.getByText("unapplied · reported 475 W")).toBeVisible();
  expect(power).toHaveValue(500);
  expect(profileSelector()).not.toHaveTextContent(
    "Changes not saved to profile",
  );
  const menu = await openProfileMenu(user);
  expect(menu).toHaveTextContent(
    "Apply or discard outstanding tuning changes before applying a profile.",
  );
  expect(profileEntry("Quiet")).toHaveAttribute("aria-disabled", "true");
  expect(profileEntry("Loud")).toHaveAttribute("aria-disabled", "true");
  await user.click(profileEntry("Loud"));
  expect(fake.currentProfiles).toEqual({ "GPU-123": "quiet" });
  expect(fake.liveWrites()).toBe(0);
  await closeProfileMenu(user);

  await user.click(
    screen.getByRole("button", { name: "Discard power limit change" }),
  );
  expect(
    screen.queryByText("unapplied · reported 475 W"),
  ).not.toBeInTheDocument();
  expect(power).toHaveValue(475);
  await openProfileMenu(user);
  expect(profileEntry("Loud")).not.toHaveAttribute("aria-disabled", "true");
});

it("keeps reported clock offsets and fan speed visible alongside their unapplied targets", async () => {
  const user = userEvent.setup();
  createFakeDesktop({
    response: createPerformanceResponse({
      gpus: [
        createPerformanceGpu({
          clocks: {
            memory: {
              support: "supported",
              currentOffsetMHz: -100,
              minOffsetMHz: -500,
              maxOffsetMHz: 1000,
            },
          },
        }),
      ],
    }),
  });
  renderPerformancePage();
  const graphics = await screen.findByRole("spinbutton", {
    name: "Graphics clock offset",
  });
  await user.clear(graphics);
  await user.type(graphics, "150");
  const memory = screen.getByRole("spinbutton", {
    name: "Memory clock offset",
  });
  await user.clear(memory);
  await user.type(memory, "500");
  const fan = screen.getByRole("spinbutton", { name: "Fan 1 manual target" });
  await user.clear(fan);
  await user.type(fan, "65");

  expect(screen.getByText("unapplied · reported +120 MHz")).toBeVisible();
  expect(screen.getByText("unapplied · reported -100 MHz")).toBeVisible();
  expect(screen.getByText("unapplied · reported 42%")).toBeVisible();
  expect(graphics).toHaveValue(150);
  expect(memory).toHaveValue(500);
  expect(fan).toHaveValue(65);
});

it("offers to save applied changes to the current profile before switching to another profile", async () => {
  const user = userEvent.setup();
  const fake = createDesktopWithProfileDifference();
  renderPerformancePage();
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));

  await chooseProfile(user, "Loud");
  const dialog = await screen.findByRole("dialog", { name: "Switch profile?" });
  expect(
    within(dialog).getByRole("button", { name: 'Save to "Quiet" and switch' }),
  ).toBeEnabled();
  expect(
    within(dialog).getByRole("button", { name: "Switch without saving" }),
  ).toBeEnabled();
  expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeEnabled();
  expect(fake.liveWrites()).toBe(0);
  expect(fake.profiles[0].settings.powerLimitWatts).toEqual({
    kind: "value",
    value: 450,
  });

  await user.click(
    within(dialog).getByRole("button", { name: 'Save to "Quiet" and switch' }),
  );
  await waitFor(() => expect(dialog).not.toBeInTheDocument());
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Loud"));
  expect(fake.profiles[0].settings).toEqual({
    cooling: { arrangement: "shared", setting: { mode: "automatic" } },
    powerLimitWatts: { kind: "value", value: 475 },
    graphicsOffsetMHz: { kind: "value", value: 120 },
  });
  expect(fake.gpu.power.currentLimitWatts).toBe(500);
  expect(fake.currentProfiles).toEqual({ "GPU-123": "loud" });
});

it.each(["Switch without saving", "Cancel"])(
  "%s leaves the current profile's saved values unchanged",
  async (choice) => {
    const user = userEvent.setup();
    const fake = createDesktopWithProfileDifference();
    renderPerformancePage();
    await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));
    const saved = structuredClone(fake.profiles);
    await chooseProfile(user, "Loud");
    const dialog = await screen.findByRole("dialog", {
      name: "Switch profile?",
    });
    await user.click(within(dialog).getByRole("button", { name: choice }));
    await waitFor(() => expect(dialog).not.toBeInTheDocument());
    expect(fake.profiles).toEqual(saved);
    expect(fake.saveProfileToCurrent).not.toHaveBeenCalled();
    if (choice === "Cancel") {
      expect(fake.currentProfiles).toEqual({ "GPU-123": "quiet" });
      expect(fake.gpu.power.currentLimitWatts).toBe(475);
      expect(fake.liveWrites()).toBe(0);
      expect(profileSelector()).toHaveTextContent("Quiet");
      expect(profileSelector()).toHaveTextContent(
        "Changes not saved to profile",
      );
    } else {
      await waitFor(() => expect(profileSelector()).toHaveTextContent("Loud"));
      expect(fake.currentProfiles).toEqual({ "GPU-123": "loud" });
      expect(fake.gpu.power.currentLimitWatts).toBe(500);
    }
  },
);

it("keeps the switch dialog open and applies nothing if saving to the current profile fails", async () => {
  const user = userEvent.setup();
  const fake = createDesktopWithProfileDifference();
  renderPerformancePage();
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));
  await chooseProfile(user, "Loud");
  const dialog = await screen.findByRole("dialog", { name: "Switch profile?" });
  fake.saveProfileToCurrent.mockRejectedValueOnce(new Error(STALE));
  await user.click(
    within(dialog).getByRole("button", { name: 'Save to "Quiet" and switch' }),
  );

  expect(await within(dialog).findByRole("alert")).toHaveTextContent(STALE);
  expect(fake.currentProfiles).toEqual({ "GPU-123": "quiet" });
  expect(fake.profiles[0].settings.powerLimitWatts).toEqual({
    kind: "value",
    value: 450,
  });
  expect(fake.liveWrites()).toBe(0);
  await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect(profileSelector()).toHaveTextContent("Quiet");
});

it("requires choosing again if another session changes the current profile during the switch dialog", async () => {
  const user = userEvent.setup();
  const fake = createDesktopWithProfileDifference();
  renderPerformancePage();
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));
  await chooseProfile(user, "Loud");
  const dialog = await screen.findByRole("dialog", { name: "Switch profile?" });
  fake.currentProfiles["GPU-123"] = "loud";
  fake.pushTuningState("GPU-123");

  expect(await within(dialog).findByRole("status")).toHaveTextContent(
    "The current profile changed. Cancel and choose a profile again.",
  );
  expect(
    within(dialog).getByRole("button", { name: 'Save to "Quiet" and switch' }),
  ).toBeDisabled();
  expect(
    within(dialog).getByRole("button", { name: "Switch without saving" }),
  ).toBeDisabled();
  await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect(profileSelector()).toHaveTextContent("Loud");
  expect(fake.liveWrites()).toBe(0);
  expect(fake.saveProfileToCurrent).not.toHaveBeenCalled();
});

it("lets a stale save be retried using the refreshed reported values shown inside the switch dialog", async () => {
  const user = userEvent.setup();
  const fake = createDesktopWithProfileDifference();
  renderPerformancePage();
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));
  await chooseProfile(user, "Loud");
  const dialog = await screen.findByRole("dialog", { name: "Switch profile?" });
  // Keep the old readings on screen until the rejected save is acknowledged.
  fake.getPerformance.mockResolvedValue(
    await window.prism.gpu.getPerformance(),
  );
  fake.gpu.power.currentLimitWatts = 500;
  await user.click(
    within(dialog).getByRole("button", { name: 'Save to "Quiet" and switch' }),
  );
  expect(await within(dialog).findByRole("alert")).toHaveTextContent(STALE);
  expect(fake.currentProfiles).toEqual({ "GPU-123": "quiet" });
  expect(fake.liveWrites()).toBe(0);

  fake.getPerformance.mockImplementation(async () =>
    structuredClone(fake.response),
  );
  expect(
    await within(dialog).findByText(/Reported values: Power limit 500 W/),
  ).toBeVisible();
  await user.click(
    within(dialog).getByRole("button", { name: 'Save to "Quiet" and switch' }),
  );
  await waitFor(() => expect(dialog).not.toBeInTheDocument());
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Loud"));
  expect(fake.profiles[0].settings.powerLimitWatts).toEqual({
    kind: "value",
    value: 500,
  });
});

it("rejects a save when the GPU's values changed since the app last saw them", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    profiles: [quiet],
    currentProfiles: { "GPU-123": "quiet" },
  });
  renderPerformancePage();
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));
  // The app holds the revision it last saw while another change reaches the GPU.
  fake.getTuningStates.mockResolvedValue(
    await window.prism.gpu.getTuningStates(),
  );
  fake.gpu.power.currentLimitWatts = 500;

  await saveToCurrent(user);
  expect(await screen.findByRole("alert")).toHaveTextContent(STALE);
  expect(fake.profiles[0].settings).toEqual(quiet.settings);

  const dialog = await saveAsNew(user, "New");
  expect(await within(dialog).findByRole("alert")).toHaveTextContent(STALE);
  expect(fake.profiles).toHaveLength(1);
  expect(fake.currentProfiles).toEqual({ "GPU-123": "quiet" });
});

it("rejects a save while the app still shows values from before a pushed change", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    profiles: [quiet],
    currentProfiles: { "GPU-123": "quiet" },
  });
  renderPerformancePage();
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));
  // Automation changes the GPU; its pushed state arrives before the readings catch up.
  fake.getPerformance.mockResolvedValue(
    await window.prism.gpu.getPerformance(),
  );
  fake.gpu.power.currentLimitWatts = 500;
  fake.pushTuningState("GPU-123");

  await saveToCurrent(user);
  expect(await screen.findByRole("alert")).toHaveTextContent(STALE);
  const dialog = await saveAsNew(user, "New");
  expect(await within(dialog).findByRole("alert")).toHaveTextContent(STALE);
  expect(fake.saveProfileToCurrent).not.toHaveBeenCalled();
  expect(fake.saveProfile).not.toHaveBeenCalled();
  expect(fake.profiles).toHaveLength(1);
});

it("shows another user's current profile by name without offering to save over it", async () => {
  const user = userEvent.setup();
  const theirs: GpuTuningProfile = { ...quiet, id: "theirs", name: "Silent" };
  const fake = createFakeDesktop({
    profiles: [quiet],
    otherUsersProfiles: [theirs],
    currentProfiles: { "GPU-123": "theirs" },
  });
  renderPerformancePage();
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Silent"));

  const menu = await openProfileMenu(user);
  expect(
    within(menu).getByText(
      'The current profile "Silent" belongs to another user.',
    ),
  ).toBeInTheDocument();
  expect(profileEntry("Quiet")).not.toBeChecked();
  expect(
    within(menu).queryByRole("menuitem", { name: /^Save to/ }),
  ).not.toBeInTheDocument();
  expect(
    within(menu).getByRole("menuitem", { name: "Save as new…" }),
  ).toBeEnabled();
  expect(
    within(menu).queryByText("Changes not saved to profile"),
  ).not.toBeInTheDocument();
  await closeProfileMenu(user);
  expect(fake.saveProfileToCurrent).not.toHaveBeenCalled();
  expect(profileSelector()).not.toHaveTextContent(
    "Changes not saved to profile",
  );
  expect(
    screen.queryByRole("region", { name: "Current profile saved values" }),
  ).not.toBeInTheDocument();
  await chooseProfile(user, "Quiet");
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));
  expect(
    screen.queryByRole("dialog", { name: "Switch profile?" }),
  ).not.toBeInTheDocument();
});

it("shows a current profile pushed after another session's change", async () => {
  const fake = createFakeDesktop({ profiles: [quiet] });
  renderPerformancePage();
  await waitFor(() =>
    expect(profileSelector()).toHaveTextContent("No current profile"),
  );
  // Polling hasn't seen the change yet, so only the pushed state can show it.
  fake.getTuningStates.mockResolvedValue(
    await window.prism.gpu.getTuningStates(),
  );
  fake.currentProfiles["GPU-123"] = "quiet";

  fake.pushTuningState("GPU-123");
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));
});
