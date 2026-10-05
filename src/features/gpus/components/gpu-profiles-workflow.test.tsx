import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  connectedDaemonStatus,
  createPerformanceGpu,
  createPerformanceResponse,
} from "@/test/gpu-fixtures";
import { createFakeDesktop } from "@/test/fake-desktop";
import { renderWithQueryClient } from "@/test/query-client";
import type { GpuTuningProfile } from "@/features/gpus/types";
import { GpuTuningPage } from "./gpu-tuning-page";
import { GpuPerformancePage } from "./gpu-performance-page";
import { MemoryRouter } from "react-router-dom";

describe("GPU tuning profiles", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
      connectedDaemonStatus,
    );
  });

  it("saves current configured values without Apply and selects retained profiles after page reload", async () => {
    const user = userEvent.setup();
    const fake = createFakeDesktop();
    const first = renderWithQueryClient(
      <MemoryRouter>
        <GpuPerformancePage />
      </MemoryRouter>,
    );
    await user.type(
      await screen.findByRole("textbox", { name: "Profile name" }),
      "Quiet",
    );
    await user.click(screen.getByRole("button", { name: "Save profile" }));
    await waitFor(() =>
      expect(
        screen.getByRole("combobox", { name: "Saved profile" }),
      ).toHaveValue("profile-1"),
    );
    expect(
      within(screen.getByRole("region", { name: /profiles/ })).getByText(
        "475 W",
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("Changes not saved to profile"),
    ).not.toBeInTheDocument();
    first.unmount();
    renderWithQueryClient(
      <MemoryRouter>
        <GpuPerformancePage />
      </MemoryRouter>,
    );
    const select = await screen.findByRole("combobox", {
      name: "Saved profile",
    });
    await screen.findByRole("option", { name: "Quiet" });
    await user.selectOptions(select, "profile-1");
    expect(select).toHaveValue("profile-1");
    expect(fake.liveWrites()).toBe(0);
    expect(fake.saveProfile).toHaveBeenCalledExactlyOnceWith({
      gpuId: "GPU-123",
      name: "Quiet",
    });
    expect(fake.profiles[0].settings).toEqual({
      powerLimitWatts: { kind: "value", value: 475 },
      graphicsOffsetMHz: { kind: "value", value: 120 },
    });
    expect(
      within(screen.getByRole("region", { name: /profiles/ })).getByText(
        "120 MHz",
      ),
    ).toBeInTheDocument();
  });

  it("blocks unresolved profile-setting edits and saves refreshed actual values after explicit discard", async () => {
    const user = userEvent.setup();
    const fake = createFakeDesktop();
    fake.setPowerLimit.mockImplementation(async () => {
      fake.gpu.power.currentLimitWatts = 450;
      return { success: false, error: "Power denied" };
    });
    renderWithQueryClient(<GpuTuningPage />);
    await user.type(
      await screen.findByRole("textbox", { name: "Profile name" }),
      "Actual",
    );
    const save = screen.getByRole("button", { name: "Save profile" });
    expect(save).toBeEnabled();
    const power = screen.getByRole("spinbutton", { name: "Power limit" });
    await user.clear(power);
    await user.type(power, "500");
    expect(save).toBeDisabled();
    expect(
      screen.getByText(
        /Apply or discard unapplied power-limit or clock-offset changes/,
      ),
    ).toBeInTheDocument();
    const section = power.closest("section")!;
    await user.click(within(section).getByRole("button", { name: "Apply" }));
    await screen.findByText("Power limit: Failed — Power denied");
    expect(save).toBeDisabled();
    expect(power).toHaveValue(500);
    await user.click(
      screen.getByRole("button", { name: "Discard power limit change" }),
    );
    expect(power).toHaveValue(450);
    expect(save).toBeEnabled();
    await user.click(save);
    await waitFor(() => expect(fake.profiles).toHaveLength(1));
    expect(fake.profiles[0].settings.powerLimitWatts).toEqual({
      kind: "value",
      value: 450,
    });
  });

  it("compares applied profile settings separately from drafts and lets failed fan edits coexist with saving", async () => {
    const user = userEvent.setup();
    const fake = createFakeDesktop({
      profiles: [
        {
          id: "quiet",
          name: "Quiet",
          gpuId: "GPU-123",
          settings: {
            powerLimitWatts: { kind: "value", value: 475 },
            graphicsOffsetMHz: { kind: "value", value: 120 },
          },
        },
      ],
    });
    fake.setFanSpeed.mockResolvedValue({
      success: false,
      error: "Fan denied",
    });
    const save = fake.saveProfile;
    renderWithQueryClient(<GpuTuningPage />);
    await screen.findByRole("option", { name: "Quiet" });
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Saved profile" }),
      "quiet",
    );
    await user.type(
      screen.getByRole("textbox", { name: "Profile name" }),
      "New",
    );
    const fan = screen.getByRole("spinbutton", { name: "Fan 1 manual target" });
    await user.clear(fan);
    await user.type(fan, "65");
    expect(screen.getByRole("button", { name: "Save profile" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "Set Manual" }));
    await screen.findByText("Fan 1: Failed — Fan denied");
    expect(screen.getByText("Unapplied changes")).toBeInTheDocument();
    expect(
      screen.queryByText("Changes not saved to profile"),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save profile" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "Save profile" }));
    await waitFor(() =>
      expect(save).toHaveBeenCalledWith({ gpuId: "GPU-123", name: "New" }),
    );
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
    expect(screen.getByRole("button", { name: "Save profile" })).toBeEnabled();
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
      renderWithQueryClient(
        surface === "Performance" ? (
          <MemoryRouter>
            <GpuPerformancePage />
          </MemoryRouter>
        ) : (
          <GpuTuningPage />
        ),
      );
      await user.type(
        await screen.findByRole("textbox", { name: "Profile name" }),
        "Quiet",
      );
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
        expect(
          screen.getByRole("button", { name: "Save profile" }),
        ).toBeEnabled();
        await user.click(screen.getByRole("button", { name: "Save profile" }));
        await waitFor(() =>
          expect(save).toHaveBeenCalledWith({
            gpuId: "GPU-123",
            name: "Quiet",
          }),
        );
        expect(fan).toHaveValue(65);
      } finally {
        finishFan({ success: false, error: "Fan denied" });
      }
      await screen.findByText("Fan 1: Failed — Fan denied");
    },
  );

  it("keeps ownership separate for identical-model GPUs and retains absent GPU profiles as unavailable", async () => {
    const user = userEvent.setup();
    const first = createPerformanceGpu();
    const second = createPerformanceGpu({ id: "GPU-456", stableId: "GPU-456" });
    const response = createPerformanceResponse({ gpus: [first, second] });
    const profile: GpuTuningProfile = {
      id: "quiet",
      name: "Quiet",
      gpuId: "GPU-123",
      settings: { powerLimitWatts: { kind: "value", value: 475 } },
    };
    createFakeDesktop({ response, profiles: [profile] });
    renderWithQueryClient(<GpuTuningPage />);
    const firstPanel = await screen.findByRole("region", {
      name: /profiles \(GPU-123\)/,
    });
    const secondPanel = screen.getByRole("region", {
      name: /profiles \(GPU-456\)/,
    });
    await within(firstPanel).findByRole("option", { name: "Quiet" });
    expect(
      within(secondPanel).queryByRole("option", { name: "Quiet" }),
    ).not.toBeInTheDocument();
    await user.selectOptions(within(firstPanel).getByRole("combobox"), "quiet");
    response.gpus = [second, first];
    await waitFor(() =>
      expect(screen.getAllByRole("region")[0]).toHaveAccessibleName(/GPU-456/),
    );
    expect(
      within(
        screen.getByRole("region", { name: /profiles \(GPU-123\)/ }),
      ).getByRole("combobox"),
    ).toHaveValue("quiet");
    response.gpus = [second];
    const unavailable = await screen.findByRole("region", {
      name: "Unavailable GPU profiles",
    });
    expect(within(unavailable).getByText("Quiet")).toBeInTheDocument();
    expect(
      within(unavailable).getByText(/Owning GPU absent.*GPU-123/),
    ).toBeInTheDocument();
    expect(
      within(
        screen.getByRole("region", { name: /profiles \(GPU-456\)/ }),
      ).queryByRole("option", { name: "Quiet" }),
    ).not.toBeInTheDocument();
    response.gpus = [first, second];
    await waitFor(() =>
      expect(
        screen.queryByRole("region", { name: "Unavailable GPU profiles" }),
      ).not.toBeInTheDocument(),
    );
    expect(
      within(
        screen.getByRole("region", { name: /profiles \(GPU-123\)/ }),
      ).getByRole("combobox"),
    ).toHaveValue("quiet");
  });

  it.each([
    {
      label: "unknown power support",
      gpu: createPerformanceGpu({
        power: { support: "unknown", reason: "Power probe failed" },
      }),
      reason: /Power limit: support is unknown.*Power probe failed/,
    },
    {
      label: "unknown memory support",
      gpu: createPerformanceGpu({
        clocks: {
          memory: { support: "unknown", reason: "Memory probe failed" },
        },
      }),
      reason: /Memory clock offset: support is unknown.*Memory probe failed/,
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
        /Power limit: current configured value is unavailable.*Power read failed/,
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
        /Graphics clock offset: current configured value is unavailable.*Graphics read failed/,
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
        /Memory clock offset: current configured value is unavailable.*Memory read failed/,
    },
    {
      label: "no supported profile settings",
      gpu: createPerformanceGpu({
        power: { support: "unsupported" },
        clocks: { graphics: { support: "unsupported" } },
      }),
      reason: /no supported profile settings/,
    },
  ])("explains why saving is blocked for $label", async ({ gpu, reason }) => {
    const user = userEvent.setup();
    const { saveProfile: save } = createFakeDesktop({
      response: createPerformanceResponse({ gpus: [gpu] }),
    });
    renderWithQueryClient(<GpuTuningPage />);
    const panel = await screen.findByRole("region", { name: /profiles/ });
    await user.type(
      within(panel).getByRole("textbox", { name: "Profile name" }),
      "Quiet",
    );
    expect(
      within(panel).getByRole("button", { name: "Save profile" }),
    ).toBeDisabled();
    expect(within(panel).getByText(reason)).toBeInTheDocument();
    expect(save).not.toHaveBeenCalled();
  });

  it("blocks saving and marks comparison unavailable when polling loses a selected profile's reading", async () => {
    const user = userEvent.setup();
    const response = createPerformanceResponse();
    const profile: GpuTuningProfile = {
      id: "quiet",
      name: "Quiet",
      gpuId: "GPU-123",
      settings: { powerLimitWatts: { kind: "value", value: 475 } },
    };
    createFakeDesktop({ response, profiles: [profile] });
    renderWithQueryClient(<GpuTuningPage />);
    await screen.findByRole("option", { name: "Quiet" });
    await user.selectOptions(screen.getByRole("combobox"), "quiet");
    await user.type(
      screen.getByRole("textbox", { name: "Profile name" }),
      "New",
    );
    response.gpus[0].power.currentLimitWatts = null;
    response.gpus[0].power.currentValueError = "Power read lost";
    await screen.findByText(
      "Profile comparison is unavailable until current settings can be read.",
    );
    expect(screen.getByRole("button", { name: "Save profile" })).toBeDisabled();
    expect(
      screen.queryByText("Changes not saved to profile"),
    ).not.toBeInTheDocument();
    response.gpus[0].power.currentLimitWatts = 450;
    response.gpus[0].power.currentValueError = null;
    await screen.findByText("Changes not saved to profile");
    expect(screen.getByRole("button", { name: "Save profile" })).toBeEnabled();
  });

  it.each([
    [
      "Performance",
      () => (
        <MemoryRouter>
          <GpuPerformancePage />
        </MemoryRouter>
      ),
    ],
    ["Tuning", () => <GpuTuningPage />],
  ])(
    "shows profile features as unavailable, with the reason, on the %s page for a GPU without a stable identity",
    async (_page, page) => {
      const fake = createFakeDesktop({
        response: createPerformanceResponse({
          gpus: [createPerformanceGpu({ id: "nvidia-0", stableId: null })],
        }),
        profiles: [
          {
            id: "quiet",
            name: "Quiet",
            gpuId: "GPU-123",
            settings: { powerLimitWatts: { kind: "value", value: 450 } },
          },
        ],
      });
      renderWithQueryClient(page());
      const panel = await screen.findByRole("region", {
        name: /profiles \(nvidia-0\)/,
      });
      expect(
        within(panel).getByText("GPU tuning profiles unavailable"),
      ).toBeInTheDocument();
      expect(within(panel).getByRole("status")).toHaveTextContent(
        "This GPU reports no stable identity (UUID), so a profile can't be tied to it.",
      );
      expect(within(panel).queryByRole("combobox")).not.toBeInTheDocument();
      expect(within(panel).queryByRole("button")).not.toBeInTheDocument();
      expect(fake.saveProfile).not.toHaveBeenCalled();
      expect(fake.applyProfileById).not.toHaveBeenCalled();
    },
  );
});
