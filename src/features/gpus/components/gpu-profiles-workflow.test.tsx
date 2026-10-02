import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  connectedDaemonStatus,
  createPerformanceGpu,
  createPerformanceResponse,
} from "@/test/gpu-fixtures";
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
    const response = createPerformanceResponse();
    const profiles: GpuTuningProfile[] = [];
    vi.spyOn(window.prism.gpu, "getPerformance").mockImplementation(async () =>
      structuredClone(response),
    );
    vi.spyOn(window.prism.gpu, "listProfiles").mockImplementation(async () =>
      structuredClone(profiles),
    );
    vi.spyOn(window.prism.gpu, "saveProfile").mockImplementation(
      async ({ name, gpuId }) => {
        const profile: GpuTuningProfile = {
          id: "profile-quiet",
          name,
          gpuId,
          settings: {
            powerLimitWatts: { kind: "value", value: 475 },
            graphicsOffsetMHz: { kind: "value", value: 120 },
          },
        };
        profiles.push(profile);
        return profile;
      },
    );
    const setPower = vi.spyOn(window.prism.gpu, "setPowerLimit");
    const setClocks = vi.spyOn(window.prism.gpu, "setClockOffsets");
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
      ).toHaveValue("profile-quiet"),
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
    await user.selectOptions(select, "profile-quiet");
    expect(select).toHaveValue("profile-quiet");
    expect(setPower).not.toHaveBeenCalled();
    expect(setClocks).not.toHaveBeenCalled();
    expect(profiles[0].settings).not.toHaveProperty("memoryOffsetMHz");
    expect(
      within(screen.getByRole("region", { name: /profiles/ })).getByText(
        "120 MHz",
      ),
    ).toBeInTheDocument();
  });

  it("blocks unresolved profile-setting edits and saves refreshed actual values after explicit discard", async () => {
    const user = userEvent.setup();
    const response = createPerformanceResponse();
    const profiles: GpuTuningProfile[] = [];
    vi.spyOn(window.prism.gpu, "getPerformance").mockImplementation(async () =>
      structuredClone(response),
    );
    vi.spyOn(window.prism.gpu, "listProfiles").mockImplementation(async () =>
      structuredClone(profiles),
    );
    vi.spyOn(window.prism.gpu, "setPowerLimit").mockImplementation(async () => {
      response.gpus[0].power.currentLimitWatts = 450;
      return { success: false, error: "Power denied" };
    });
    vi.spyOn(window.prism.gpu, "saveProfile").mockImplementation(
      async ({ name, gpuId }) => {
        const profile: GpuTuningProfile = {
          id: "actual",
          name,
          gpuId,
          settings: {
            powerLimitWatts: {
              kind: "value",
              value: response.gpus[0].power.currentLimitWatts!,
            },
            graphicsOffsetMHz: { kind: "value", value: 120 },
          },
        };
        profiles.push(profile);
        return profile;
      },
    );
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
    await waitFor(() => expect(profiles).toHaveLength(1));
    expect(profiles[0].settings.powerLimitWatts).toEqual({
      kind: "value",
      value: 450,
    });
  });

  it("compares applied profile settings separately from drafts and lets failed fan edits coexist with saving", async () => {
    const user = userEvent.setup();
    const response = createPerformanceResponse();
    const profile: GpuTuningProfile = {
      id: "quiet",
      name: "Quiet",
      gpuId: "GPU-123",
      settings: {
        powerLimitWatts: { kind: "value", value: 475 },
        graphicsOffsetMHz: { kind: "value", value: 120 },
      },
    };
    vi.spyOn(window.prism.gpu, "getPerformance").mockImplementation(async () =>
      structuredClone(response),
    );
    const profiles = [profile];
    vi.spyOn(window.prism.gpu, "listProfiles").mockImplementation(async () =>
      structuredClone(profiles),
    );
    vi.spyOn(window.prism.gpu, "setFanSpeed").mockResolvedValue({
      success: false,
      error: "Fan denied",
    });
    vi.spyOn(window.prism.gpu, "setPowerLimit").mockImplementation(
      async (_id, value) => {
        response.gpus[0].power.currentLimitWatts = value;
        return { success: true, error: null };
      },
    );
    const save = vi
      .spyOn(window.prism.gpu, "saveProfile")
      .mockImplementation(async () => {
        const saved = { ...profile, id: "new", name: "New" };
        profiles.push(saved);
        return saved;
      });
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
    vi.spyOn(window.prism.gpu, "getPerformance").mockImplementation(async () =>
      structuredClone(response),
    );
    vi.spyOn(window.prism.gpu, "listProfiles").mockResolvedValue([profile]);
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
      label: "enumeration fallback",
      gpu: createPerformanceGpu({ id: "nvidia-0", stableId: null }),
      reason: /stable identity could not be established/,
    },
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
    vi.spyOn(window.prism.gpu, "getPerformance").mockResolvedValue(
      createPerformanceResponse({ gpus: [gpu] }),
    );
    vi.spyOn(window.prism.gpu, "listProfiles").mockResolvedValue([]);
    const save = vi.spyOn(window.prism.gpu, "saveProfile");
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
    vi.spyOn(window.prism.gpu, "getPerformance").mockImplementation(async () =>
      structuredClone(response),
    );
    vi.spyOn(window.prism.gpu, "listProfiles").mockResolvedValue([profile]);
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
});
