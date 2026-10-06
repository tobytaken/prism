import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, expect, it, vi } from "vitest";

import { AppearanceProvider } from "@/features/settings/lib/appearance-provider";
import { createFakeDesktop } from "@/test/fake-desktop";
import {
  createPerformanceGpu,
  createPerformanceResponse,
} from "@/test/gpu-fixtures";
import { renderWithQueryClient } from "@/test/query-client";
import { SettingsPage } from "./settings-page";

beforeEach(() => vi.restoreAllMocks());

it("defaults startup behavior to Off and lists each GPU with no startup profile when enabled", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    response: createPerformanceResponse({
      gpus: [
        createPerformanceGpu(),
        createPerformanceGpu({
          id: "GPU-456",
          stableId: "GPU-456",
          name: "Second GPU",
        }),
      ],
    }),
  });
  renderWithQueryClient(
    <MemoryRouter>
      <AppearanceProvider>
        <SettingsPage />
      </AppearanceProvider>
    </MemoryRouter>,
  );
  await user.click(screen.getByRole("tab", { name: "Startup" }));
  const off = await screen.findByRole("radio", { name: "Off" });
  expect(off).toBeChecked();
  await user.click(screen.getByRole("radio", { name: "Startup profiles" }));
  await waitFor(() => expect(fake.startupBehavior).toBe("startupProfiles"));
  expect(
    await screen.findByText(/Second GPU.*nothing applied at startup/),
  ).toBeInTheDocument();
  expect(screen.getAllByText(/nothing applied at startup/)).toHaveLength(2);
  await user.click(off);
  await waitFor(() => expect(fake.startupBehavior).toBe("off"));
  expect(
    screen.queryByText(/Second GPU.*nothing applied at startup/),
  ).not.toBeInTheDocument();
});

it("warns before restoring last applied values and marks GPUs whose values are possibly stale", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({ possiblyStale: ["GPU-123"] });
  renderWithQueryClient(
    <MemoryRouter>
      <AppearanceProvider>
        <SettingsPage />
      </AppearanceProvider>
    </MemoryRouter>,
  );
  await user.click(screen.getByRole("tab", { name: "Startup" }));
  const restore = await screen.findByRole("radio", {
    name: /Restore last applied/,
  });
  expect(restore).toHaveAccessibleDescription(
    /an unstable value could be restored on every boot/i,
  );
  expect(screen.queryByText(/possibly stale/i)).not.toBeInTheDocument();
  await user.click(restore);
  await waitFor(() => expect(fake.startupBehavior).toBe("restoreLastApplied"));
  expect(restore).toBeChecked();
  const stale = await screen.findByRole("status", { name: "Possibly stale" });
  expect(stale).toHaveTextContent(
    /NVIDIA GeForce RTX 5090 \(GPU-123\).*not recorded for startup/,
  );
  expect(
    screen.queryByText(/nothing applied at startup/),
  ).not.toBeInTheDocument();

  fake.possiblyStale.splice(0);
  await waitFor(() =>
    expect(
      screen.queryByRole("status", { name: "Possibly stale" }),
    ).not.toBeInTheDocument(),
  );
});
