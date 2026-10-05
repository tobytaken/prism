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

it("leaves the GPU, the profile and the current profile unchanged when the range review is cancelled", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    profiles: [quiet, loud],
    currentProfiles: { "GPU-123": "quiet" },
  });
  renderPerformancePage();
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));

  await chooseProfile(user, "Loud");
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
