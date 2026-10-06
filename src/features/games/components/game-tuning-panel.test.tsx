import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router-dom";
import { beforeEach, expect, it, vi } from "vitest";
import { appRoutes } from "@/app/router";
import { createFakeDesktop } from "@/test/fake-desktop";
import { connectedDaemonStatus } from "@/test/gpu-fixtures";
import { renderWithQueryClient } from "@/test/query-client";

beforeEach(() => {
  vi.restoreAllMocks();
  vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
    connectedDaemonStatus,
  );
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
    "Prism doesn't switch profiles for games yet",
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
