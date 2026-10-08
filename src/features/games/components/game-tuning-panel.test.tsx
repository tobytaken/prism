import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router-dom";
import { beforeEach, expect, it, vi } from "vitest";
import { appRoutes } from "@/app/router";
import { createFakeDesktop } from "@/test/fake-desktop";
import { connectedDaemonStatus } from "@/test/gpu-fixtures";
import { findProfileControls, profileSelector } from "@/test/profile-selector";
import { renderWithQueryClient } from "@/test/query-client";
import type { AutomaticSwitchingStatus } from "@/features/games/types";

beforeEach(() => {
  vi.restoreAllMocks();
  vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
    connectedDaemonStatus,
  );
});

it("pauses and resumes a GPU only from Game tuning and shows the matching row and header status", async () => {
  const user = userEvent.setup();
  createFakeDesktop({
    automaticSwitching: true,
    switchingStatus: { condition: { status: "ready" }, gpus: [] },
  });
  const router = createMemoryRouter(appRoutes, {
    initialEntries: ["/performance"],
  });
  renderWithQueryClient(<RouterProvider router={router} />);
  const panel = await screen.findByRole("region", { name: "Game tuning" });
  await findProfileControls();
  const header = profileSelector().closest("summary")!;
  const pause = await within(panel).findByRole("button", { name: "Pause" });
  expect(screen.getAllByRole("button", { name: "Pause" })).toHaveLength(1);
  await user.click(pause);
  expect(
    await within(panel).findByRole("button", { name: "Resume" }),
  ).toBeEnabled();
  expect(panel).toHaveTextContent(
    "Paused. Current tuning is kept; applied fan curves keep running.",
  );
  expect(within(header).getByTestId("switching-chip")).toHaveTextContent(
    "Switching paused",
  );
  expect(screen.getAllByRole("button", { name: "Resume" })).toHaveLength(1);
  await user.click(within(panel).getByRole("button", { name: "Resume" }));
  expect(
    await within(panel).findByRole("button", { name: "Pause" }),
  ).toBeEnabled();
  expect(panel).toHaveTextContent("Waiting for a linked game to run.");
  expect(within(header).queryByTestId("switching-chip")).toBeNull();
  await user.click(within(panel).getByRole("link", { name: "Games" }));
  expect(await screen.findByRole("heading", { name: "Games" })).toBeVisible();
  expect(screen.queryByRole("button", { name: /^(Pause|Resume)$/ })).toBeNull();
});

it("lists running games from pushed events, keeps the opt-in, and links to Games with live Running badges", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    steamApps: [{ appId: 570, name: "Dota 2" }],
    games: [
      { game: { kind: "steam", appId: 570 }, name: "Dota 2", profiles: {} },
    ],
  });
  const router = createMemoryRouter(appRoutes, {
    initialEntries: ["/performance"],
  });
  renderWithQueryClient(<RouterProvider router={router} />);
  const panel = await screen.findByRole("region", { name: "Game tuning" });
  expect(await within(panel).findByText("No running games.")).toBeVisible();
  await user.click(
    within(panel).getByRole("checkbox", {
      name: "Switch profiles automatically while my games run",
    }),
  );
  await waitFor(() => expect(fake.automaticSwitching).toBe(true));
  expect(panel).toHaveTextContent(
    "restores the values from before the game when it ends",
  );
  act(() =>
    fake.pushRunningGames({
      status: "available",
      games: [
        { game: { kind: "steam", appId: 570 }, pid: 123, startTimeTicks: 420 },
        { game: { kind: "steam", appId: 620 }, pid: 124, startTimeTicks: 430 },
      ],
    }),
  );
  expect(await within(panel).findByText("Dota 2")).toBeVisible();
  expect(within(panel).getByText("Steam app 620")).toBeVisible();
  await user.click(within(panel).getByRole("link", { name: "Games" }));
  const card = await screen.findByRole("article", { name: "Dota 2" });
  expect(await within(card).findByText("Running")).toBeVisible();
  act(() =>
    fake.pushRunningGames({
      status: "unavailable",
      reason: "Process details could not be read.",
    }),
  );
  expect(await screen.findByText(/Recognition unavailable/)).toBeVisible();
  expect(within(card).queryByText("Running")).not.toBeInTheDocument();
  act(() => fake.pushRunningGames({ status: "available", games: [] }));
  await waitFor(() =>
    expect(
      screen.queryByText(/Recognition unavailable/),
    ).not.toBeInTheDocument(),
  );
});

it("offers Resume for a manual override, reports a rejected request, and disables actions while disconnected", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    switchingStatus: {
      condition: { status: "ready" },
      gpus: [
        {
          gpuId: "GPU-123",
          state: { status: "manualOverride", changedOutsidePrism: false },
        },
      ],
    },
  });
  fake.setGpuSwitchingPaused.mockRejectedValueOnce(
    new Error("The pause could not be saved."),
  );
  const router = createMemoryRouter(appRoutes, {
    initialEntries: ["/performance"],
  });
  renderWithQueryClient(<RouterProvider router={router} />);
  const panel = await screen.findByRole("region", { name: "Game tuning" });
  await user.click(
    await within(panel).findByRole("button", { name: "Resume" }),
  );
  expect(await within(panel).findByRole("alert")).toHaveTextContent(
    "The pause could not be saved.",
  );
  expect(within(panel).getByRole("button", { name: "Resume" })).toBeEnabled();
  expect(panel).toHaveTextContent("Manual override");
  // A temporary override can become a persistent pause without first applying game tuning.
  await user.click(within(panel).getByRole("button", { name: "Pause" }));
  await waitFor(() =>
    expect(panel).toHaveTextContent("Paused. Current tuning is kept"),
  );
  expect(within(panel).queryByRole("button", { name: "Pause" })).toBeNull();
  await user.click(within(panel).getByRole("button", { name: "Resume" }));
  expect(
    await within(panel).findByRole("button", { name: "Pause" }),
  ).toBeEnabled();
  expect(within(panel).queryByRole("alert")).toBeNull();
  act(() =>
    fake.pushDaemonStatus({
      ...connectedDaemonStatus,
      reachable: false,
      managedByDaemon: false,
    }),
  );
  await waitFor(() =>
    expect(within(panel).getByRole("button", { name: "Pause" })).toBeDisabled(),
  );
});

it("shows recognition unavailable on scan failure or disconnect and refreshes after reconnect", async () => {
  const fake = createFakeDesktop({
    runningGames: {
      status: "unavailable",
      reason: "Process details could not be read.",
    },
  });
  const router = createMemoryRouter(appRoutes, {
    initialEntries: ["/performance"],
  });
  renderWithQueryClient(<RouterProvider router={router} />);
  const panel = await screen.findByRole("region", { name: "Game tuning" });
  expect(
    await within(panel).findByText(/Recognition unavailable/),
  ).toHaveTextContent("Process details could not be read");
  expect(
    within(panel).queryByText("No running games."),
  ).not.toBeInTheDocument();
  act(() =>
    fake.pushRunningGames({
      status: "available",
      games: [
        { game: { kind: "steam", appId: 570 }, pid: 1, startTimeTicks: 100 },
      ],
    }),
  );
  expect(await within(panel).findByText("Steam app 570")).toBeVisible();
  act(() =>
    fake.pushDaemonStatus({
      ...connectedDaemonStatus,
      reachable: false,
      managedByDaemon: false,
    }),
  );
  expect(
    await within(panel).findByText(/Recognition unavailable/),
  ).toHaveTextContent("disconnected");
  expect(within(panel).queryByText("Steam app 570")).not.toBeInTheDocument();
  act(() => fake.pushDaemonStatus(connectedDaemonStatus));
  expect(await within(panel).findByText("Steam app 570")).toBeVisible();
});

it("keeps a newer pushed snapshot when an older snapshot request finishes later", async () => {
  const fake = createFakeDesktop();
  let resolve!: (state: { status: "available"; games: [] }) => void;
  vi.spyOn(window.prism.games, "getRunning").mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  const router = createMemoryRouter(appRoutes, {
    initialEntries: ["/performance"],
  });
  renderWithQueryClient(<RouterProvider router={router} />);
  const panel = await screen.findByRole("region", { name: "Game tuning" });
  act(() =>
    fake.pushRunningGames({
      status: "available",
      games: [
        { game: { kind: "steam", appId: 570 }, pid: 1, startTimeTicks: 100 },
      ],
    }),
  );
  expect(await within(panel).findByText("Steam app 570")).toBeVisible();
  await act(async () => resolve({ status: "available", games: [] }));
  expect(within(panel).getByText("Steam app 570")).toBeVisible();
});

const dotaOnGaming = {
  status: "active",
  game: { kind: "steam", appId: 570 },
  gameName: "Dota 2",
  profile: { id: "gaming", name: "Gaming" },
} as const;

it("shows each GPU's automatic switching status live, a header chip when it isn't ready, and tags the game profile as auto", async () => {
  const fake = createFakeDesktop({
    profiles: [
      {
        id: "gaming",
        name: "Gaming",
        gpuId: "GPU-123",
        settings: { powerLimitWatts: { kind: "value", value: 350 } },
      },
    ],
    currentProfiles: { "GPU-123": "gaming" },
    switchingStatus: {
      condition: { status: "ready" },
      gpus: [{ gpuId: "GPU-123", state: dotaOnGaming }],
    },
  });
  const router = createMemoryRouter(appRoutes, {
    initialEntries: ["/performance"],
  });
  renderWithQueryClient(<RouterProvider router={router} />);
  const panel = await screen.findByRole("region", { name: "Game tuning" });
  const rows = await within(panel).findByRole("list", {
    name: "Automatic switching status",
  });
  const row = within(rows).getByRole("listitem");
  expect(row).toHaveTextContent("NVIDIA GeForce RTX 5090");
  await waitFor(() =>
    expect(row).toHaveTextContent("Gaming applied for Dota 2"),
  );
  await findProfileControls();
  await waitFor(() =>
    expect(profileSelector()).toHaveTextContent("auto · Dota 2"),
  );
  const header = profileSelector().closest("summary")!;
  expect(within(header).queryByTestId("switching-chip")).toBeNull();

  const push = (status: AutomaticSwitchingStatus) =>
    act(() => fake.pushSwitchingStatus(status));
  push({
    condition: {
      status: "recognitionUnavailable",
      reason: "A complete host process scan could not be read.",
    },
    gpus: [{ gpuId: "GPU-123", state: dotaOnGaming }],
  });
  await waitFor(() =>
    expect(row).toHaveTextContent(
      "Holding Gaming for Dota 2: recognition unavailable.",
    ),
  );
  expect(within(header).getByTestId("switching-chip")).toHaveTextContent(
    "Recognition unavailable",
  );

  push({
    condition: { status: "ready" },
    gpus: [
      {
        gpuId: "GPU-123",
        state: { status: "failed", message: "Power limit failed." },
      },
    ],
  });
  await waitFor(() =>
    expect(row).toHaveTextContent("Failed: Power limit failed."),
  );
  expect(within(header).getByTestId("switching-chip")).toHaveTextContent(
    "Failed · retry needed",
  );
  expect(profileSelector()).not.toHaveTextContent("auto ·");

  push({
    condition: {
      status: "blocked",
      reason:
        "Automatic switching is suspended because Prism's tuning store can't be used.",
    },
    gpus: [],
  });
  await waitFor(() =>
    expect(row).toHaveTextContent("Blocked: Automatic switching is suspended"),
  );
  expect(within(header).getByTestId("switching-chip")).toHaveTextContent(
    "Switching blocked",
  );

  push({
    condition: { status: "blocked", reason: "The boot state cannot be read." },
    gpus: [
      {
        gpuId: "GPU-123",
        state: {
          status: "recoveryRequired",
          message: "The pre-game tuning values cannot be read.",
        },
      },
    ],
  });
  await waitFor(() =>
    expect(row).toHaveTextContent(
      "Recovery required: The pre-game tuning values cannot be read.",
    ),
  );
  expect(within(header).getByTestId("switching-chip")).toHaveTextContent(
    "Recovery required",
  );

  push({ condition: { status: "ready" }, gpus: [] });
  await waitFor(() =>
    expect(row).toHaveTextContent("Waiting for a linked game to run."),
  );
  expect(within(header).queryByTestId("switching-chip")).toBeNull();

  push({
    condition: { status: "off" },
    gpus: [{ gpuId: "GPU-123", state: { status: "anotherUser" } }],
  });
  await waitFor(() =>
    expect(row).toHaveTextContent("Another user's game is running."),
  );

  act(() =>
    fake.pushDaemonStatus({
      ...connectedDaemonStatus,
      reachable: false,
      managedByDaemon: false,
    }),
  );
  await waitFor(() => expect(row).toHaveTextContent("Status unknown"));
  act(() => fake.pushDaemonStatus(connectedDaemonStatus));
  await waitFor(() =>
    expect(row).toHaveTextContent("Another user's game is running."),
  );
});

it("shows manual overrides and changes outside Prism in the GPU row and header chip", async () => {
  const fake = createFakeDesktop();
  const router = createMemoryRouter(appRoutes, {
    initialEntries: ["/performance"],
  });
  renderWithQueryClient(<RouterProvider router={router} />);
  const panel = await screen.findByRole("region", { name: "Game tuning" });
  await findProfileControls();
  const header = profileSelector().closest("summary")!;
  for (const changedOutsidePrism of [false, true]) {
    act(() =>
      fake.pushSwitchingStatus({
        condition: { status: "ready" },
        gpus: [
          {
            gpuId: "GPU-123",
            state: { status: "manualOverride", changedOutsidePrism },
          },
        ],
      }),
    );
    const label = changedOutsidePrism
      ? "Manual override · changed outside Prism"
      : "Manual override";
    await waitFor(() => expect(panel).toHaveTextContent(label));
    expect(within(header).getByTestId("switching-chip")).toHaveTextContent(
      label,
    );
    expect(profileSelector()).not.toHaveTextContent("auto ·");
  }
});

it("shows automatic setting and rollback results and retries explicitly from the failure panel", async () => {
  const user = userEvent.setup();
  const failed: AutomaticSwitchingStatus = {
    condition: { status: "ready" },
    gpus: [
      {
        gpuId: "GPU-123",
        state: { status: "failed", message: "Memory rejected." },
        application: {
          power: { status: "succeeded", error: null },
          graphics: null,
          memory: { status: "failed", error: "Memory rejected." },
          rollback: {
            status: "restored",
            power: { status: "succeeded", error: null },
            graphics: null,
            memory: null,
            cooling: null,
          },
        },
      },
    ],
  };
  const fake = createFakeDesktop({ switchingStatus: failed });
  const retry = vi
    .spyOn(window.prism.games, "retrySwitching")
    .mockImplementation(async () => {
      const active: AutomaticSwitchingStatus = {
        condition: { status: "ready" },
        gpus: [{ gpuId: "GPU-123", state: dotaOnGaming }],
      };
      fake.pushSwitchingStatus(active);
      return active;
    });
  renderWithQueryClient(
    <RouterProvider
      router={createMemoryRouter(appRoutes, {
        initialEntries: ["/performance"],
      })}
    />,
  );
  const panel = await screen.findByRole("region", { name: "Game tuning" });
  await waitFor(() =>
    expect(panel).toHaveTextContent("Memory clock offset: Failed"),
  );
  expect(panel).toHaveTextContent("Rollback: Restored");
  await findProfileControls();
  expect(
    within(profileSelector().closest("summary")!).getByTestId("switching-chip"),
  ).toHaveTextContent("Failed · retry needed");
  act(() =>
    fake.pushSwitchingStatus({
      condition: { status: "ready" },
      gpus: [
        {
          ...failed.gpus[0],
          state: {
            status: "recoveryRequired",
            message: "Cooling recovery could not be verified.",
          },
          application: {
            power: null,
            graphics: null,
            memory: null,
            cooling: {
              revision: 2,
              outcome: { status: "failed", error: null },
              fans: [
                {
                  fanIndex: 0,
                  outcome: { status: "failed", error: "Fan rejected." },
                },
              ],
            },
            rollback: {
              status: "restoreFailed",
              power: null,
              graphics: null,
              memory: null,
              cooling: null,
              coolingRestoration: [
                { fanIndex: 0, status: "failed", detail: "Auto rejected." },
                {
                  fanIndex: 1,
                  status: "unverified",
                  detail: "Fan mode is unavailable.",
                },
              ],
            },
          },
        },
      ],
    }),
  );
  await waitFor(() =>
    expect(panel).toHaveTextContent("Rollback: Restore failed"),
  );
  expect(panel).toHaveTextContent(
    "Fan 1 automatic cooling: Failed — Auto rejected.",
  );
  expect(panel).toHaveTextContent(
    "Fan 2 automatic cooling: Unverified — Fan mode is unavailable.",
  );
  expect(
    within(profileSelector().closest("summary")!).getByTestId("switching-chip"),
  ).toHaveTextContent("Failed · retry needed");
  expect(retry).not.toHaveBeenCalled();
  await user.click(within(panel).getByRole("button", { name: /Retry/ }));
  expect(retry).toHaveBeenCalledWith({ gpuId: "GPU-123" });
  await waitFor(() =>
    expect(panel).toHaveTextContent("Gaming applied for Dota 2"),
  );
  expect(
    within(panel).queryByRole("button", { name: /Retry/ }),
  ).not.toBeInTheDocument();
});
