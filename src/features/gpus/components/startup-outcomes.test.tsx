import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, expect, it, vi } from "vitest";

import { createFakeDesktop } from "@/test/fake-desktop";
import { connectedDaemonStatus } from "@/test/gpu-fixtures";
import { renderWithQueryClient } from "@/test/query-client";
import { chooseProfile } from "@/test/profile-selector";
import { GpuPerformancePage } from "./gpu-performance-page";

beforeEach(() => {
  vi.restoreAllMocks();
  vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
    connectedDaemonStatus,
  );
});

it("shows honest per-setting startup results, allows manual range review, and dismisses the outcome", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    profiles: [
      {
        id: "quiet",
        name: "Quiet",
        gpuId: "GPU-123",
        settings: { powerLimitWatts: { kind: "value", value: 700 } },
      },
    ],
    startupOutcomes: [
      {
        gpuId: "GPU-123",
        profile: { id: "quiet", name: "Quiet" },
        status: "failed",
        reason: "Driver rejected the offset.",
        application: {
          power: { status: "succeeded", error: null },
          graphics: { status: "failed", error: "Driver rejected the offset." },
          memory: {
            status: "unattempted",
            error: "An earlier setting failed.",
          },
        },
      },
    ],
  });
  renderWithQueryClient(
    <MemoryRouter>
      <GpuPerformancePage />
    </MemoryRouter>,
  );
  const rows = await screen.findByRole("region", { name: "Startup outcomes" });
  expect(rows).toHaveTextContent("Quiet — Failed");
  expect(rows).toHaveTextContent("Power limit: Applied");
  expect(rows).toHaveTextContent("Graphics clock offset: Failed");
  expect(rows).toHaveTextContent("Memory clock offset: Not attempted");
  await chooseProfile(user, "Quiet");
  expect(
    await screen.findByRole("region", { name: "Review profile corrections" }),
  ).toHaveTextContent("saved 700 W");
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(fake.profiles[0].settings.powerLimitWatts).toEqual({
    kind: "value",
    value: 700,
  });
  await user.click(within(rows).getByRole("button", { name: /Dismiss/ }));
  await waitFor(() =>
    expect(
      screen.queryByRole("region", { name: "Startup outcomes" }),
    ).not.toBeInTheDocument(),
  );
  expect(fake.startupOutcomes).toEqual([]);
});
