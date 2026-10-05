import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
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
import type { GpuTuningProfile } from "@/features/gpus/types";
import { GpuTuningPage } from "./gpu-tuning-page";
import { GpuPerformancePage } from "./gpu-performance-page";
import { MemoryRouter } from "react-router-dom";

function renderPerformancePage() {
  return renderWithQueryClient(
    <MemoryRouter>
      <GpuPerformancePage />
    </MemoryRouter>,
  );
}

const quiet: GpuTuningProfile = {
  id: "quiet",
  name: "Quiet",
  gpuId: "GPU-123",
  settings: {
    powerLimitWatts: { kind: "value", value: 475 },
    graphicsOffsetMHz: { kind: "value", value: 120 },
  },
};

describe("GPU tuning profiles", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
      connectedDaemonStatus,
    );
  });

  it("saves current configured values without Apply as the current profile, which survives a page reload", async () => {
    const user = userEvent.setup();
    const fake = createFakeDesktop();
    const first = renderPerformancePage();
    await saveAsNew(user, "Quiet");
    await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));
    expect(
      screen.queryByText("Changes not saved to profile"),
    ).not.toBeInTheDocument();
    await openProfileMenu(user);
    expect(profileEntry("Quiet")).toHaveTextContent(
      "Power limit 475 W · Graphics offset 120 MHz",
    );
    expect(profileEntry("Quiet")).toBeChecked();
    first.unmount();

    renderPerformancePage();
    await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));
    expect(fake.liveWrites()).toBe(0);
    expect(fake.saveProfile).toHaveBeenCalledExactlyOnceWith({
      gpuId: "GPU-123",
      name: "Quiet",
      revision: expect.any(String),
    });
    expect(fake.profiles[0].settings).toEqual({
      powerLimitWatts: { kind: "value", value: 475 },
      graphicsOffsetMHz: { kind: "value", value: 120 },
    });
  });

  it("blocks unresolved profile-setting edits and saves refreshed actual values after explicit discard", async () => {
    const user = userEvent.setup();
    const fake = createFakeDesktop();
    fake.setPowerLimit.mockImplementation(async () => {
      fake.gpu.power.currentLimitWatts = 450;
      return { success: false, error: "Power denied" };
    });
    renderPerformancePage();
    await findProfileControls();
    expect((await saveAsNewAvailability(user)).enabled).toBe(true);
    const power = screen.getByRole("spinbutton", { name: "Power limit" });
    await user.clear(power);
    await user.type(power, "500");
    const blocked = await saveAsNewAvailability(user);
    expect(blocked.enabled).toBe(false);
    expect(blocked.text).toContain(
      "Apply or discard unapplied power-limit or clock-offset changes before saving a profile.",
    );
    const section = power.closest("section")!;
    await user.click(within(section).getByRole("button", { name: "Apply" }));
    await screen.findByText("Power limit: Failed — Power denied");
    expect((await saveAsNewAvailability(user)).enabled).toBe(false);
    expect(power).toHaveValue(500);
    await user.click(
      screen.getByRole("button", { name: "Discard power limit change" }),
    );
    expect(power).toHaveValue(450);
    expect((await saveAsNewAvailability(user)).enabled).toBe(true);
    await saveAsNew(user, "Actual");
    await waitFor(() => expect(fake.profiles).toHaveLength(1));
    expect(fake.profiles[0].settings.powerLimitWatts).toEqual({
      kind: "value",
      value: 450,
    });
  });

  it("compares applied values with the current profile separately from drafts and lets failed fan edits coexist with saving", async () => {
    const user = userEvent.setup();
    const fake = createFakeDesktop({
      profiles: [quiet],
      currentProfiles: { "GPU-123": "quiet" },
    });
    fake.setFanSpeed.mockResolvedValue({
      success: false,
      error: "Fan denied",
    });
    renderPerformancePage();
    await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));
    const fan = screen.getByRole("spinbutton", { name: "Fan 1 manual target" });
    await user.clear(fan);
    await user.type(fan, "65");
    expect((await saveAsNewAvailability(user)).enabled).toBe(true);
    await user.click(screen.getByRole("button", { name: "Set Manual" }));
    await screen.findByText("Fan 1: Failed — Fan denied");
    expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
    expect(
      screen.queryByText("Changes not saved to profile"),
    ).not.toBeInTheDocument();
    expect((await saveAsNewAvailability(user)).enabled).toBe(true);
    await saveAsNew(user, "New");
    await waitFor(() =>
      expect(fake.saveProfile).toHaveBeenCalledWith({
        gpuId: "GPU-123",
        name: "New",
        revision: expect.any(String),
      }),
    );
    await waitFor(() => expect(profileSelector()).toHaveTextContent("New"));
    const power = screen.getByRole("spinbutton", { name: "Power limit" });
    await user.clear(power);
    await user.type(power, "500");
    expect(
      screen.queryByText("Changes not saved to profile"),
    ).not.toBeInTheDocument();
    await user.click(
      within(power.closest("section")!).getByRole("button", { name: "Apply" }),
    );
    await screen.findByText("Changes not saved to profile");
    expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
    expect(fan).toHaveValue(65);
    expect((await saveAsNewAvailability(user)).enabled).toBe(true);
  });

  it.each(["Performance", "Apply all"])(
    "allows saving known profile settings while %s awaits a fan outcome",
    async (surface) => {
      const user = userEvent.setup();
      const fake = createFakeDesktop();
      const save = fake.saveProfile;
      let finishFan!: (result: {
        success: boolean;
        error: string | null;
      }) => void;
      const fanResult = new Promise<{ success: boolean; error: string | null }>(
        (resolve) => {
          finishFan = resolve;
        },
      );
      fake.setFanSpeed.mockReturnValue(fanResult);
      if (surface === "Performance") renderPerformancePage();
      else renderWithQueryClient(<GpuTuningPage />);
      await findProfileControls();
      const fan = screen.getByRole("spinbutton", {
        name: "Fan 1 manual target",
      });
      await user.clear(fan);
      await user.type(fan, "65");
      const action = screen.getByRole("button", {
        name: surface === "Performance" ? "Set Manual" : "Apply all",
      });
      await user.click(action);
      try {
        expect(action).toBeDisabled();
        if (surface === "Apply all") {
          expect(
            screen.getByRole("spinbutton", { name: "Power limit" }),
          ).toBeDisabled();
          expect(
            screen.getByRole("spinbutton", { name: "Graphics clock offset" }),
          ).toBeDisabled();
        }
        expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
        await saveAsNew(user, "Quiet");
        await waitFor(() =>
          expect(save).toHaveBeenCalledWith({
            gpuId: "GPU-123",
            name: "Quiet",
            revision: expect.any(String),
          }),
        );
        expect(fan).toHaveValue(65);
      } finally {
        finishFan({ success: false, error: "Fan denied" });
      }
      await screen.findByText("Fan 1: Failed — Fan denied");
    },
  );

  it("keeps the current profile with its GPU when identical-model GPUs reorder, and retains absent GPU profiles as unavailable", async () => {
    const user = userEvent.setup();
    const first = createPerformanceGpu();
    const second = createPerformanceGpu({ id: "GPU-456", stableId: "GPU-456" });
    const response = createPerformanceResponse({ gpus: [first, second] });
    createFakeDesktop({
      response,
      profiles: [
        {
          ...quiet,
          settings: { powerLimitWatts: quiet.settings.powerLimitWatts! },
        },
      ],
    });
    renderPerformancePage();
    const menu = await openProfileMenu(user, "GPU-456");
    expect(within(menu).queryByRole("menuitemradio")).not.toBeInTheDocument();
    await user.keyboard("{Escape}");
    await chooseProfile(user, "Quiet");
    await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));
    response.gpus = [second, first];
    await waitFor(() =>
      expect(
        screen.getAllByRole("group", { name: /profiles \(/ })[0],
      ).toHaveAccessibleName(/GPU-456/),
    );
    expect(profileSelector()).toHaveTextContent("Quiet");
    expect(profileSelector("GPU-456")).toHaveTextContent("No current profile");
    response.gpus = [second];
    const unavailable = await screen.findByRole("region", {
      name: "Unavailable GPU profiles",
    });
    expect(within(unavailable).getByText("Quiet")).toBeInTheDocument();
    expect(
      within(unavailable).getByText(/Owning GPU absent.*GPU-123/),
    ).toBeInTheDocument();
    response.gpus = [first, second];
    await waitFor(() =>
      expect(
        screen.queryByRole("region", { name: "Unavailable GPU profiles" }),
      ).not.toBeInTheDocument(),
    );
    await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));
  });

  it.each([
    {
      label: "unknown power support",
      gpu: createPerformanceGpu({
        power: { support: "unknown", reason: "Power probe failed" },
      }),
      reason: "Power limit: support is unknown. Power probe failed",
    },
    {
      label: "unknown memory support",
      gpu: createPerformanceGpu({
        clocks: {
          memory: { support: "unknown", reason: "Memory probe failed" },
        },
      }),
      reason: "Memory clock offset: support is unknown. Memory probe failed",
    },
    {
      label: "unavailable configured power",
      gpu: createPerformanceGpu({
        power: {
          currentLimitWatts: null,
          currentValueError: "Power read failed",
        },
      }),
      reason:
        "Power limit: current configured value is unavailable. Power read failed",
    },
    {
      label: "unavailable graphics offset",
      gpu: createPerformanceGpu({
        clocks: {
          graphics: {
            currentOffsetMHz: null,
            currentValueError: "Graphics read failed",
          },
        },
      }),
      reason:
        "Graphics clock offset: current configured value is unavailable. Graphics read failed",
    },
    {
      label: "unavailable supported memory offset",
      gpu: createPerformanceGpu({
        clocks: {
          memory: {
            support: "supported",
            currentOffsetMHz: null,
            currentValueError: "Memory read failed",
          },
        },
      }),
      reason:
        "Memory clock offset: current configured value is unavailable. Memory read failed",
    },
    {
      label: "no supported profile settings",
      gpu: createPerformanceGpu({
        power: { support: "unsupported" },
        clocks: { graphics: { support: "unsupported" } },
      }),
      reason: "no supported profile settings",
    },
  ])("explains why saving is blocked for $label", async ({ gpu, reason }) => {
    const user = userEvent.setup();
    const { saveProfile: save } = createFakeDesktop({
      response: createPerformanceResponse({ gpus: [gpu] }),
    });
    renderPerformancePage();
    const availability = await saveAsNewAvailability(user);
    expect(availability.enabled).toBe(false);
    expect(availability.text).toContain(reason);
    expect(save).not.toHaveBeenCalled();
  });

  it("blocks saving and marks comparison unavailable when polling loses a reading of the current profile", async () => {
    const user = userEvent.setup();
    const response = createPerformanceResponse();
    const profile: GpuTuningProfile = {
      ...quiet,
      settings: { powerLimitWatts: { kind: "value", value: 475 } },
    };
    createFakeDesktop({
      response,
      profiles: [profile],
      currentProfiles: { "GPU-123": "quiet" },
    });
    renderPerformancePage();
    await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));
    response.gpus[0].power.currentLimitWatts = null;
    response.gpus[0].power.currentValueError = "Power read lost";
    await waitFor(async () =>
      expect((await saveAsNewAvailability(user)).text).toContain(
        "Profile comparison is unavailable until current settings can be read.",
      ),
    );
    expect((await saveAsNewAvailability(user)).enabled).toBe(false);
    expect(
      screen.queryByText("Changes not saved to profile"),
    ).not.toBeInTheDocument();
    response.gpus[0].power.currentLimitWatts = 450;
    response.gpus[0].power.currentValueError = null;
    await screen.findByText("Changes not saved to profile");
    expect((await saveAsNewAvailability(user)).enabled).toBe(true);
  });

  it.each([
    ["Performance", renderPerformancePage],
    ["Tuning", () => renderWithQueryClient(<GpuTuningPage />)],
  ])(
    "shows profile features as unavailable, with the reason, on the %s page for a GPU without a stable identity",
    async (_page, render) => {
      const fake = createFakeDesktop({
        response: createPerformanceResponse({
          gpus: [createPerformanceGpu({ id: "nvidia-0", stableId: null })],
        }),
        profiles: [{ ...quiet, id: "other" }],
      });
      render();
      const controls = await findProfileControls("nvidia-0");
      expect(within(controls).getByRole("status")).toHaveTextContent(
        "Profiles unavailable: this GPU reports no stable identity (UUID), so a profile can't be tied to it.",
      );
      expect(within(controls).queryByRole("button")).not.toBeInTheDocument();
      expect(fake.saveProfile).not.toHaveBeenCalled();
      expect(fake.applyProfileById).not.toHaveBeenCalled();
    },
  );
});
