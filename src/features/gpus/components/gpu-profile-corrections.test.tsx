import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import {
  connectedDaemonStatus,
  createPerformanceResponse,
} from "@/test/gpu-fixtures";
import { renderWithQueryClient } from "@/test/query-client";
import type {
  GpuProfileSettings,
  GpuTuningProfile,
  ProfileApplicationResponse,
  ProfileRangeCorrection,
  TuningOutcome,
} from "@/features/gpus/types";
import { MemoryRouter } from "react-router-dom";
import { GpuPerformancePage } from "./gpu-performance-page";
import { GpuTuningPage } from "./gpu-tuning-page";

type Setting = "power" | "graphics" | "memory";

// A stateful stand-in for the desktop bridge: fresh validation, profile storage, and GPU controls.
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
  const profiles: GpuTuningProfile[] = [
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
  ];
  const failures: Partial<Record<Setting, string>> = {};
  const savedWhenApplied: GpuProfileSettings[] = [];
  const controls = {
    power: {
      key: "powerLimitWatts",
      label: "Power limit",
      info: () => ({
        support: gpu.power.support,
        min: gpu.power.minLimitWatts,
        max: gpu.power.maxLimitWatts,
      }),
      write: (value: number) => (gpu.power.currentLimitWatts = value),
    },
    graphics: {
      key: "graphicsOffsetMHz",
      label: "Graphics clock offset",
      info: () => ({
        support: gpu.clocks.graphics.support,
        min: gpu.clocks.graphics.minOffsetMHz,
        max: gpu.clocks.graphics.maxOffsetMHz,
      }),
      write: (value: number) => (gpu.clocks.graphics.currentOffsetMHz = value),
    },
    memory: {
      key: "memoryOffsetMHz",
      label: "Memory clock offset",
      info: () => ({
        support: gpu.clocks.memory.support,
        min: gpu.clocks.memory.minOffsetMHz,
        max: gpu.clocks.memory.maxOffsetMHz,
      }),
      write: (value: number) => (gpu.clocks.memory.currentOffsetMHz = value),
    },
  } as const;
  vi.spyOn(window.prism.gpu, "getPerformance").mockImplementation(async () =>
    structuredClone(response),
  );
  vi.spyOn(window.prism.gpu, "listProfiles").mockImplementation(async () =>
    structuredClone(profiles),
  );
  const correctProfile = vi
    .spyOn(window.prism.gpu, "correctProfile")
    .mockImplementation(async ({ profileId, corrections }) => {
      const profile = profiles.find((profile) => profile.id === profileId)!;
      for (const { setting, savedValue, proposedValue } of corrections) {
        expect(profile.settings[setting]).toEqual({
          kind: "value",
          value: savedValue,
        });
        profile.settings[setting] = { kind: "value", value: proposedValue };
      }
      return structuredClone(profile);
    });
  const applyProfile = vi
    .spyOn(window.prism.gpu, "applyProfile")
    .mockImplementation(async ({ settings }) => {
      savedWhenApplied.push(structuredClone(profiles[0].settings));
      const included = (Object.keys(controls) as Setting[]).filter(
        (setting) => settings[controls[setting].key] != null,
      );
      const errors: string[] = [];
      const corrections: ProfileRangeCorrection[] = [];
      for (const setting of included) {
        const { key, label, info } = controls[setting];
        const value = settings[key]!;
        const { support, min, max } = info();
        if (support !== "supported") errors.push(`${label} is ${support}.`);
        else if (value > max! || value < min!) {
          errors.push(`${label}: requested ${value} is outside the range.`);
          corrections.push({
            setting: key,
            savedValue: value,
            proposedValue: value > max! ? max! : min!,
            bound: value > max! ? "maximum" : "minimum",
          });
        }
      }
      const result: ProfileApplicationResponse = {
        power: null,
        graphics: null,
        memory: null,
      };
      if (errors.length > 0) {
        for (const setting of included)
          result[setting] = { status: "unattempted", error: errors.join(" ") };
        if (corrections.length === errors.length)
          result.corrections = corrections;
        return result;
      }
      let failed = false;
      for (const setting of included) {
        let outcome: TuningOutcome;
        if (failed)
          outcome = { status: "unattempted", error: "An earlier failure" };
        else if (failures[setting]) {
          failed = true;
          outcome = { status: "failed", error: failures[setting]! };
        } else {
          controls[setting].write(settings[controls[setting].key]!);
          outcome = { status: "succeeded", error: null };
        }
        result[setting] = outcome;
      }
      return result;
    });
  const livePower = vi.spyOn(window.prism.gpu, "setPowerLimit");
  const liveClocks = vi.spyOn(window.prism.gpu, "setClockOffsets");
  return {
    gpu,
    profiles,
    failures,
    savedWhenApplied,
    correctProfile,
    applyProfile,
    liveWrites: () =>
      livePower.mock.calls.length + liveClocks.mock.calls.length,
  };
}

function renderPerformancePage() {
  return renderWithQueryClient(
    <MemoryRouter>
      <GpuPerformancePage />
    </MemoryRouter>,
  );
}

function profilePanel() {
  return screen.getByRole("region", { name: /profiles \(GPU-123\)/ });
}

async function openReview(user: ReturnType<typeof userEvent.setup>) {
  await screen.findByRole("option", { name: "Quiet" });
  await user.selectOptions(screen.getByRole("combobox"), "quiet");
  await user.click(screen.getByRole("button", { name: "Load profile" }));
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
  expect(screen.getByRole("button", { name: "Load profile" })).toBeDisabled();

  await user.click(within(review).getByRole("button", { name: "Cancel" }));
  expect(
    screen.queryByRole("region", { name: "Review profile corrections" }),
  ).not.toBeInTheDocument();
  expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Load profile" })).toBeEnabled();
  expect(fake.correctProfile).not.toHaveBeenCalled();
  expect(fake.applyProfile).toHaveBeenCalledTimes(1);
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

it.each([
  ["Performance", renderPerformancePage],
  ["Tuning", () => renderWithQueryClient(<GpuTuningPage />)],
])(
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
    expect(fake.correctProfile).toHaveBeenCalledExactlyOnceWith({
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
    expect(fake.applyProfile).toHaveBeenLastCalledWith({
      gpuId: "GPU-123",
      settings: {
        powerLimitWatts: 280,
        graphicsOffsetMHz: -200,
        memoryOffsetMHz: 500,
      },
    });
    expect(fake.savedWhenApplied.at(-1)).toEqual(corrected);
    expect(fake.gpu.power.currentLimitWatts).toBe(280);
    expect(fake.gpu.clocks.graphics.currentOffsetMHz).toBe(-200);
    expect(fake.gpu.clocks.memory.currentOffsetMHz).toBe(500);
    expect(screen.getByText("Graphics clock offset: Applied")).toBeVisible();
    expect(
      screen.queryByRole("region", { name: "Review profile corrections" }),
    ).not.toBeInTheDocument();
    expect(await within(profilePanel()).findByText("280 W")).toBeVisible();
    await waitFor(() =>
      expect(
        screen.queryByText("Changes not saved to profile"),
      ).not.toBeInTheDocument(),
    );
    expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
    expect(fake.liveWrites()).toBe(0);
  },
);

it("reports a persistence failure clearly and makes no GPU changes", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop();
  fake.correctProfile.mockRejectedValue(new Error("Disk is read-only"));
  renderPerformancePage();
  const review = await openReview(user);
  await user.click(
    within(review).getByRole("button", { name: "Adjust profile and apply" }),
  );
  expect(await within(review).findByRole("alert")).toHaveTextContent(
    "The corrected profile could not be saved, so no GPU settings were changed. Disk is read-only",
  );
  expect(fake.applyProfile).toHaveBeenCalledTimes(1);
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
  await waitFor(() =>
    expect(screen.getByText("Changes not saved to profile")).toBeVisible(),
  );
  expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
  await user.type(screen.getByRole("textbox", { name: "Profile name" }), "New");
  expect(screen.getByRole("button", { name: "Save profile" })).toBeDisabled();
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
  await screen.findByRole("option", { name: "Quiet" });
  await user.selectOptions(screen.getByRole("combobox"), "quiet");
  expect(within(profilePanel()).getByText("280 W")).toBeVisible();
  expect(within(profilePanel()).getByText("-200 MHz")).toBeVisible();
  expect(fake.correctProfile).toHaveBeenCalledTimes(1);
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
