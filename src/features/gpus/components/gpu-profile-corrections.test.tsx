import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import {
  connectedDaemonStatus,
  createPerformanceResponse,
} from "@/test/gpu-fixtures";
import { createFakeDesktop as createDesktop } from "@/test/fake-desktop";
import {
  chooseProfile,
  openProfileMenu,
  profileAvailability,
  profileEntry,
  profileSelector,
  saveAsNewAvailability,
} from "@/test/profile-selector";
import { renderWithQueryClient } from "@/test/query-client";
import { MemoryRouter } from "react-router-dom";
import { GpuPerformancePage } from "./gpu-performance-page";

// The daemon-backed desktop API: fresh validation, profile storage, and GPU controls.
function createFakeDesktop() {
  const response = createPerformanceResponse();
  const gpu = response.gpus[0];
  Object.assign(gpu.power, {
    currentLimitWatts: 250,
    minLimitWatts: 100,
    maxLimitWatts: 280,
  });
  gpu.clocks.memory = {
    ...gpu.clocks.graphics,
    currentOffsetMHz: 0,
    minOffsetMHz: -1000,
    maxOffsetMHz: 1000,
  };
  return createDesktop({
    response,
    profiles: [
      {
        id: "quiet",
        name: "Quiet",
        gpuId: "GPU-123",
        settings: {
          powerLimitWatts: { kind: "value", value: 300 },
          graphicsOffsetMHz: { kind: "value", value: -250 },
          memoryOffsetMHz: { kind: "value", value: 500 },
        },
      },
    ],
  });
}

function renderPerformancePage() {
  return renderWithQueryClient(
    <MemoryRouter>
      <GpuPerformancePage />
    </MemoryRouter>,
  );
}

async function openReview(user: ReturnType<typeof userEvent.setup>) {
  await chooseProfile(user, "Quiet");
  return screen.findByRole("region", { name: "Review profile corrections" });
}

beforeEach(() => {
  vi.restoreAllMocks();
  vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
    connectedDaemonStatus,
  );
});

it("collects every out-of-range setting into one review, and Cancel changes neither the profile nor the GPU", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop();
  renderPerformancePage();
  const review = await openReview(user);
  expect(
    within(review).getByText(
      "Power limit: saved 300 W → current maximum 280 W",
    ),
  ).toBeInTheDocument();
  expect(
    within(review).getByText(
      "Graphics clock offset: saved -250 MHz → current minimum -200 MHz",
    ),
  ).toBeInTheDocument();
  expect(within(review).queryByText(/Memory clock offset/)).toBeNull();
  expect(within(review).getAllByRole("listitem")).toHaveLength(2);
  expect(
    within(review)
      .getAllByRole("button")
      .map((button) => button.textContent),
  ).toEqual(["Adjust profile and apply", "Cancel"]);
  expect(screen.queryByText(/Not attempted/)).not.toBeInTheDocument();
  expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
  expect((await profileAvailability(user, "Quiet")).enabled).toBe(false);

  await user.click(within(review).getByRole("button", { name: "Cancel" }));
  expect(
    screen.queryByRole("region", { name: "Review profile corrections" }),
  ).not.toBeInTheDocument();
  expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
  expect((await profileAvailability(user, "Quiet")).enabled).toBe(true);
  expect(profileSelector()).toHaveTextContent("No current profile");
  expect(fake.correctAndApplyProfile).not.toHaveBeenCalled();
  expect(fake.applyProfileById).toHaveBeenCalledExactlyOnceWith({
    profileId: "quiet",
  });
  expect(fake.profiles[0].settings).toEqual({
    powerLimitWatts: { kind: "value", value: 300 },
    graphicsOffsetMHz: { kind: "value", value: -250 },
    memoryOffsetMHz: { kind: "value", value: 500 },
  });
  expect(fake.gpu.power.currentLimitWatts).toBe(250);
  expect(fake.gpu.clocks.graphics.currentOffsetMHz).toBe(120);
  expect(fake.gpu.clocks.memory.currentOffsetMHz).toBe(0);
  expect(fake.liveWrites()).toBe(0);
});

it.each([["Performance", renderPerformancePage]])(
  "Adjust profile and apply on the %s page saves exactly the approved values before applying them",
  async (_page, render) => {
    const user = userEvent.setup();
    const fake = createFakeDesktop();
    render();
    const review = await openReview(user);
    await user.click(
      within(review).getByRole("button", { name: "Adjust profile and apply" }),
    );
    await screen.findByText("Power limit: Applied");
    expect(fake.correctAndApplyProfile).toHaveBeenCalledExactlyOnceWith({
      profileId: "quiet",
      corrections: [
        {
          setting: "powerLimitWatts",
          savedValue: 300,
          proposedValue: 280,
          bound: "maximum",
        },
        {
          setting: "graphicsOffsetMHz",
          savedValue: -250,
          proposedValue: -200,
          bound: "minimum",
        },
      ],
    });
    const corrected = {
      powerLimitWatts: { kind: "value", value: 280 },
      graphicsOffsetMHz: { kind: "value", value: -200 },
      memoryOffsetMHz: { kind: "value", value: 500 },
    };
    expect(fake.profiles[0].settings).toEqual(corrected);
    expect(fake.applyProfile).not.toHaveBeenCalled();
    expect(fake.gpu.power.currentLimitWatts).toBe(280);
    expect(fake.gpu.clocks.graphics.currentOffsetMHz).toBe(-200);
    expect(fake.gpu.clocks.memory.currentOffsetMHz).toBe(500);
    expect(screen.getByText("Graphics clock offset: Applied")).toBeVisible();
    expect(
      screen.queryByRole("region", { name: "Review profile corrections" }),
    ).not.toBeInTheDocument();
    await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));
    await waitFor(() =>
      expect(
        screen.queryByText("Changes not saved to profile"),
      ).not.toBeInTheDocument(),
    );
    await openProfileMenu(user);
    expect(profileEntry("Quiet")).toHaveTextContent(
      "Power limit 280 W · Graphics offset -200 MHz · Memory offset 500 MHz",
    );
    expect(profileEntry("Quiet")).toBeChecked();
    await user.keyboard("{Escape}");
    expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
    expect(fake.liveWrites()).toBe(0);
  },
);

it("reports a persistence failure clearly and makes no GPU changes", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop();
  fake.correctAndApplyProfile.mockRejectedValue(new Error("Disk is read-only"));
  renderPerformancePage();
  const review = await openReview(user);
  await user.click(
    within(review).getByRole("button", { name: "Adjust profile and apply" }),
  );
  expect(await within(review).findByRole("alert")).toHaveTextContent(
    "The corrected profile could not be saved, so no GPU settings were changed. Disk is read-only",
  );
  expect(fake.applyProfileById).toHaveBeenCalledTimes(1);
  expect(fake.profiles[0].settings.powerLimitWatts).toEqual({
    kind: "value",
    value: 300,
  });
  expect(fake.gpu.power.currentLimitWatts).toBe(250);
  expect(fake.liveWrites()).toBe(0);
  expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
  await user.click(within(review).getByRole("button", { name: "Cancel" }));
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

it("keeps the corrected profile after partial application failure and reload, with actual outcomes and Save gated", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop();
  fake.failures.graphics = "Graphics write denied";
  const { unmount } = renderPerformancePage();
  const review = await openReview(user);
  await user.click(
    within(review).getByRole("button", { name: "Adjust profile and apply" }),
  );
  await screen.findByText(
    "Graphics clock offset: Failed — Graphics write denied",
  );
  expect(screen.getByText("Power limit: Applied")).toBeInTheDocument();
  expect(
    screen.getByText(/Memory clock offset: Not attempted/),
  ).toBeInTheDocument();
  expect(
    screen.getByText(
      'Saved the approved corrections to "Quiet". The saved profile does not confirm GPU settings; check each setting\'s application outcome.',
    ),
  ).toBeInTheDocument();
  expect(fake.gpu.power.currentLimitWatts).toBe(280);
  expect(fake.gpu.clocks.graphics.currentOffsetMHz).toBe(120);
  expect(fake.gpu.clocks.memory.currentOffsetMHz).toBe(0);
  // Applied only in part, the corrected profile doesn't become current.
  expect(profileSelector()).toHaveTextContent("No current profile");
  expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
  expect((await saveAsNewAvailability(user)).enabled).toBe(false);
  expect(
    screen.getByRole("button", { name: "Retry graphics clock offset change" }),
  ).toBeEnabled();
  expect(
    screen.getByRole("button", { name: "Discard memory clock offset change" }),
  ).toBeEnabled();
  expect(fake.profiles[0].settings.graphicsOffsetMHz).toEqual({
    kind: "value",
    value: -200,
  });

  delete fake.failures.graphics;
  await user.click(
    screen.getByRole("button", { name: "Retry graphics clock offset change" }),
  );
  await screen.findByText("Graphics clock offset: Applied");
  expect(fake.gpu.clocks.graphics.currentOffsetMHz).toBe(-200);

  unmount();
  renderPerformancePage();
  const menu = await openProfileMenu(user);
  await within(menu).findByRole("menuitemradio", { name: /^Quiet\b/ });
  expect(profileEntry("Quiet")).toHaveTextContent(
    "Power limit 280 W · Graphics offset -200 MHz",
  );
  expect(fake.correctAndApplyProfile).toHaveBeenCalledTimes(1);
});

it("requires another explicit review when capabilities change while the review is open", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop();
  renderPerformancePage();
  const review = await openReview(user);
  fake.gpu.power.maxLimitWatts = 260;
  fake.gpu.clocks.memory.maxOffsetMHz = 400;
  await user.click(
    within(review).getByRole("button", { name: "Adjust profile and apply" }),
  );
  await screen.findByText("Power limit: saved 280 W → current maximum 260 W");
  const second = screen.getByRole("region", {
    name: "Review profile corrections",
  });
  expect(
    within(second).getByText(
      "Memory clock offset: saved 500 MHz → current maximum 400 MHz",
    ),
  ).toBeInTheDocument();
  expect(within(second).getAllByRole("listitem")).toHaveLength(2);
  expect(fake.gpu.power.currentLimitWatts).toBe(250);
  expect(fake.gpu.clocks.graphics.currentOffsetMHz).toBe(120);
  expect(fake.liveWrites()).toBe(0);
  expect(screen.queryByText(/Not attempted/)).not.toBeInTheDocument();

  await user.click(within(second).getByRole("button", { name: "Cancel" }));
  expect(fake.profiles[0].settings).toEqual({
    powerLimitWatts: { kind: "value", value: 280 },
    graphicsOffsetMHz: { kind: "value", value: -200 },
    memoryOffsetMHz: { kind: "value", value: 500 },
  });
  expect(fake.gpu.power.currentLimitWatts).toBe(250);

  const third = await openReview(user);
  await user.click(
    within(third).getByRole("button", { name: "Adjust profile and apply" }),
  );
  await screen.findByText("Memory clock offset: Applied");
  expect(fake.profiles[0].settings).toEqual({
    powerLimitWatts: { kind: "value", value: 260 },
    graphicsOffsetMHz: { kind: "value", value: -200 },
    memoryOffsetMHz: { kind: "value", value: 400 },
  });
  expect(fake.gpu.power.currentLimitWatts).toBe(260);
  expect(fake.gpu.clocks.memory.currentOffsetMHz).toBe(400);
});

it("blocks application without GPU changes when a setting becomes unsupported during review", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop();
  renderPerformancePage();
  const review = await openReview(user);
  fake.gpu.clocks.memory.support = "unsupported";
  await user.click(
    within(review).getByRole("button", { name: "Adjust profile and apply" }),
  );
  await screen.findByText(
    /Memory clock offset: Not attempted — .*Memory clock offset is unsupported/,
  );
  expect(
    screen.queryByRole("region", { name: "Review profile corrections" }),
  ).not.toBeInTheDocument();
  expect(fake.gpu.power.currentLimitWatts).toBe(250);
  expect(fake.gpu.clocks.graphics.currentOffsetMHz).toBe(120);
  expect(fake.gpu.clocks.memory.currentOffsetMHz).toBe(0);
  expect(fake.liveWrites()).toBe(0);
  expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Discard power limit change" }),
  ).toBeEnabled();
});

it("explains why adjusting waits while tuning edits are outstanding during review", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop();
  renderPerformancePage();
  const review = await openReview(user);
  const power = screen.getByRole("spinbutton", { name: "Power limit" });
  await user.clear(power);
  await user.type(power, "260");
  const adjust = within(review).getByRole("button", {
    name: "Adjust profile and apply",
  });
  expect(adjust).toBeDisabled();
  expect(
    within(review).getByText(
      "Apply or discard outstanding tuning changes before adjusting the profile.",
    ),
  ).toBeInTheDocument();
  await user.click(
    screen.getByRole("button", { name: "Discard power limit change" }),
  );
  expect(adjust).toBeEnabled();
  expect(fake.correctAndApplyProfile).not.toHaveBeenCalled();
});
