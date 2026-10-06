import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, expect, it, vi } from "vitest";
import { createFakeDesktop } from "@/test/fake-desktop";
import { connectedDaemonStatus } from "@/test/gpu-fixtures";
import { renderWithQueryClient } from "@/test/query-client";
import { openProfileMenu } from "@/test/profile-selector";
import type { MigrationState } from "@/features/gpus/types";
import { GpuPerformancePage } from "./gpu-performance-page";

beforeEach(() => {
  vi.restoreAllMocks();
  vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
    connectedDaemonStatus,
  );
});

it.each([true, false])(
  "shows migrated profiles read-only and offers adoption only to the first user (%s)",
  async (canAdopt) => {
    const user = userEvent.setup();
    const fake = createFakeDesktop();
    const state: MigrationState = {
      canAdopt,
      profiles: [
        {
          id: "previous",
          name: "Previous startup settings",
          gpuId: "GPU-123",
          settings: { powerLimitWatts: { kind: "value", value: 400 } },
        },
      ],
      notice: {
        dropped: [
          {
            key: "0000:02:00.0",
            values: {
              powerLimitWatts: 350,
              graphicsOffsetMHz: 100,
              gpuClockLock: { minMhz: 1500, maxMhz: 1800 },
            },
            reason: "No unique GPU with a UUID matched this key.",
          },
        ],
      },
    };
    vi.spyOn(window.prism.gpu, "getMigrationState").mockImplementation(
      async () => structuredClone(state),
    );
    vi.spyOn(window.prism.gpu, "adoptMigratedProfile").mockImplementation(
      async () => {
        const profile = state.profiles.shift()!;
        fake.profiles.push(profile);
        return profile;
      },
    );
    vi.spyOn(window.prism.gpu, "dismissMigrationNotice").mockImplementation(
      async () => {
        state.notice = null;
        return { success: true, error: null };
      },
    );
    renderWithQueryClient(
      <MemoryRouter>
        <GpuPerformancePage />
      </MemoryRouter>,
    );
    const notice = await screen.findByRole("region", {
      name: "Previous startup settings",
    });
    expect(notice).toHaveTextContent(
      "Saved clock locks are no longer restored",
    );
    expect(notice).toHaveTextContent("0000:02:00.0");
    expect(notice).toHaveTextContent("350");
    expect(notice).toHaveTextContent("1500");
    await openProfileMenu(user);
    expect(
      screen.getByRole("menuitemradio", { name: /Previous startup settings/ }),
    ).toHaveAttribute("aria-disabled", "true");
    await user.click(
      screen.getByRole("menuitem", { name: "Manage profiles…" }),
    );
    const dialog = await screen.findByRole("dialog", {
      name: "Manage profiles",
    });
    expect(
      within(dialog).getByText("Previous startup settings"),
    ).toBeInTheDocument();
    expect(
      within(dialog).queryByRole("button", { name: /Rename Previous/ }),
    ).not.toBeInTheDocument();
    expect(
      within(dialog).queryByRole("button", { name: /Delete Previous/ }),
    ).not.toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Done" }));
    if (canAdopt) {
      await user.click(
        within(notice).getByRole("button", { name: /Add to my profiles/ }),
      );
      await waitFor(() => expect(fake.profiles).toHaveLength(1));
      await openProfileMenu(user);
      await user.click(
        screen.getByRole("menuitem", { name: "Manage profiles…" }),
      );
      const adopted = await screen.findByRole("dialog", {
        name: "Manage profiles",
      });
      expect(
        within(adopted).getByRole("button", {
          name: "Rename Previous startup settings",
        }),
      ).toBeEnabled();
      await user.click(within(adopted).getByRole("button", { name: "Done" }));
    } else
      expect(
        within(notice).queryByRole("button", { name: /Add to my profiles/ }),
      ).not.toBeInTheDocument();
    await user.click(
      within(notice).getByRole("button", { name: "Dismiss migration notice" }),
    );
    await waitFor(() =>
      expect(
        screen.queryByText(/Saved clock locks are no longer restored/),
      ).not.toBeInTheDocument(),
    );
  },
);
