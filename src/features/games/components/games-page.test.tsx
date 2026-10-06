import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router-dom";
import { beforeEach, expect, it, vi } from "vitest";

import { appRoutes } from "@/app/router";
import type { GpuTuningProfile } from "@/features/gpus/types";
import { createFakeDesktop } from "@/test/fake-desktop";
import {
  connectedDaemonStatus,
  createPerformanceGpu,
  createPerformanceResponse,
} from "@/test/gpu-fixtures";
import { renderWithQueryClient } from "@/test/query-client";

const quiet: GpuTuningProfile = {
  id: "quiet",
  name: "Quiet",
  gpuId: "GPU-123",
  settings: { powerLimitWatts: { kind: "value", value: 450 } },
};
const loud: GpuTuningProfile = {
  ...quiet,
  id: "loud",
  name: "Loud",
  gpuId: "GPU-456",
};
const twoGpus = () =>
  createPerformanceResponse({
    gpus: [
      createPerformanceGpu(),
      createPerformanceGpu({
        id: "GPU-456",
        stableId: "GPU-456",
        name: "Second GPU",
      }),
    ],
  });
const dota = { appId: 570, name: "Dota 2" };
const portal = { appId: 620, name: "Portal 2" };

function renderApp(path = "/games") {
  const router = createMemoryRouter(appRoutes, { initialEntries: [path] });
  renderWithQueryClient(<RouterProvider router={router} />);
  return router;
}

function card(name: string) {
  return screen.getByRole("article", { name });
}

function picker(game: string, gpu: string) {
  return within(card(game)).getByRole("combobox", {
    name: new RegExp(`Profile on ${gpu}`),
  });
}

beforeEach(() => {
  vi.restoreAllMocks();
  vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
    connectedDaemonStatus,
  );
});

it("replaces Profiles with Games in the sidebar and routes to the Games page", async () => {
  const user = userEvent.setup();
  createFakeDesktop();
  const router = renderApp("/");
  const navigation = screen.getByRole("navigation");
  expect(
    within(navigation).queryByRole("link", { name: "Profiles" }),
  ).not.toBeInTheDocument();
  await user.click(within(navigation).getByRole("link", { name: "Games" }));
  expect(router.state.location.pathname).toBe("/games");
  expect(
    await screen.findByRole("heading", { level: 1, name: "Games" }),
  ).toBeInTheDocument();
  expect(
    await screen.findByText(/No games yet\. Add a Steam game/),
  ).toBeInTheDocument();
});

it("adds an installed Steam game and links a profile per GPU, defaulting to Leave unchanged", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    response: twoGpus(),
    profiles: structuredClone([quiet, loud]),
    steamApps: [portal, dota],
  });
  renderApp();
  await user.click(
    await screen.findByRole("button", { name: "Add Steam game…" }),
  );
  const dialog = await screen.findByRole("dialog", { name: "Add Steam game" });
  const apps = await within(dialog).findAllByRole("listitem");
  expect(apps.map((app) => app.textContent)).toEqual([
    expect.stringMatching(/^Dota 2.*Steam app 570/),
    expect.stringMatching(/^Portal 2.*Steam app 620/),
  ]);
  await user.click(within(dialog).getByRole("button", { name: "Add Dota 2" }));
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );

  const added = await screen.findByRole("article", { name: "Dota 2" });
  expect(added).toHaveTextContent("Steam app 570");
  expect(within(added).queryByText("Not installed")).not.toBeInTheDocument();
  const first = picker("Dota 2", "NVIDIA GeForce RTX 5090");
  const second = picker("Dota 2", "Second GPU");
  expect(first).toHaveDisplayValue("Leave unchanged");
  expect(second).toHaveDisplayValue("Leave unchanged");
  // Each GPU offers only its own profiles.
  expect(
    within(first)
      .getAllByRole("option")
      .map((option) => option.textContent),
  ).toEqual(["Leave unchanged", "Quiet"]);
  await user.selectOptions(first, "Quiet");
  await waitFor(() =>
    expect(fake.games).toEqual([
      {
        game: { kind: "steam", appId: 570 },
        name: "Dota 2",
        profiles: { "GPU-123": "quiet" },
      },
    ]),
  );
  expect(picker("Dota 2", "NVIDIA GeForce RTX 5090")).toHaveDisplayValue(
    "Quiet",
  );
  expect(second).toHaveDisplayValue("Leave unchanged");

  // Many games can link to one profile.
  await user.click(screen.getByRole("button", { name: "Add Steam game…" }));
  const again = await screen.findByRole("dialog", { name: "Add Steam game" });
  await within(again).findByRole("button", { name: "Add Portal 2" });
  expect(
    within(again).queryByRole("button", { name: "Add Dota 2" }),
  ).not.toBeInTheDocument();
  await user.click(within(again).getByRole("button", { name: "Add Portal 2" }));
  await screen.findByRole("article", { name: "Portal 2" });
  await user.selectOptions(
    picker("Portal 2", "NVIDIA GeForce RTX 5090"),
    "Quiet",
  );
  await waitFor(() =>
    expect(fake.games[1].profiles).toEqual({ "GPU-123": "quiet" }),
  );

  await user.selectOptions(
    picker("Dota 2", "NVIDIA GeForce RTX 5090"),
    "Leave unchanged",
  );
  await waitFor(() => expect(fake.games[0].profiles).toEqual({}));
});

it("keeps an uninstalled game's links and marks it Not installed, and removes and filters games", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    profiles: structuredClone([quiet]),
    steamApps: [portal],
    games: [
      {
        game: { kind: "steam", appId: 570 },
        name: "Dota 2",
        profiles: { "GPU-123": "quiet" },
      },
      { game: { kind: "steam", appId: 620 }, name: "Portal 2", profiles: {} },
    ],
  });
  renderApp();
  const uninstalled = await screen.findByRole("article", { name: "Dota 2" });
  expect(within(uninstalled).getByText("Not installed")).toBeInTheDocument();
  expect(
    within(card("Portal 2")).queryByText("Not installed"),
  ).not.toBeInTheDocument();
  await waitFor(() =>
    expect(picker("Dota 2", "NVIDIA GeForce RTX 5090")).toHaveDisplayValue(
      "Quiet",
    ),
  );

  const filter = screen.getByRole("searchbox", { name: "Filter games" });
  await user.type(filter, "port");
  expect(screen.queryByRole("article", { name: "Dota 2" })).toBeNull();
  expect(card("Portal 2")).toBeInTheDocument();
  await user.clear(filter);
  await user.type(filter, "570");
  expect(card("Dota 2")).toBeInTheDocument();
  expect(screen.queryByRole("article", { name: "Portal 2" })).toBeNull();
  await user.clear(filter);
  await user.type(filter, "zelda");
  expect(screen.getByText('No games match "zelda".')).toBeInTheDocument();
  await user.clear(filter);

  await user.click(
    within(card("Dota 2")).getByRole("button", { name: "Remove Dota 2" }),
  );
  await waitFor(() =>
    expect(screen.queryByRole("article", { name: "Dota 2" })).toBeNull(),
  );
  expect(fake.games.map(({ name }) => name)).toEqual(["Portal 2"]);
});

it("stores the automatic switching opt-in, off by default", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop();
  renderApp();
  const optIn = await screen.findByRole("checkbox", {
    name: "Switch profiles automatically while my games run",
  });
  await waitFor(() => expect(optIn).toBeEnabled());
  expect(optIn).not.toBeChecked();
  await user.click(optIn);
  await waitFor(() => expect(optIn).toBeChecked());
  expect(fake.automaticSwitching).toBe(true);
  await user.click(optIn);
  await waitFor(() => expect(optIn).not.toBeChecked());
  expect(fake.automaticSwitching).toBe(false);
});

it("explains an empty Steam library and an add that the daemon rejects", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({ steamApps: [dota] });
  fake.addSteamGame.mockRejectedValueOnce(
    new Error("Steam app 570 isn't installed in your Steam libraries."),
  );
  renderApp();
  await user.click(
    await screen.findByRole("button", { name: "Add Steam game…" }),
  );
  const dialog = await screen.findByRole("dialog", { name: "Add Steam game" });
  await user.click(
    await within(dialog).findByRole("button", { name: "Add Dota 2" }),
  );
  expect(await within(dialog).findByRole("alert")).toHaveTextContent(
    "Steam app 570 isn't installed in your Steam libraries.",
  );
  fake.steamApps.splice(0);
  await user.click(within(dialog).getByRole("button", { name: "Close" }));
  await user.click(screen.getByRole("button", { name: "Add Steam game…" }));
  expect(
    await screen.findByText(
      "No installed Steam games were found in your Steam libraries.",
    ),
  ).toBeInTheDocument();
});

it("shows a rejected change until a later change succeeds", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    profiles: structuredClone([quiet]),
    steamApps: [dota, portal],
    games: [
      { game: { kind: "steam", appId: 570 }, name: "Dota 2", profiles: {} },
      { game: { kind: "steam", appId: 620 }, name: "Portal 2", profiles: {} },
    ],
  });
  fake.setGameProfile.mockRejectedValueOnce(
    new Error("The GPU tuning profile no longer exists."),
  );
  renderApp();
  await screen.findByRole("article", { name: "Dota 2" });
  await waitFor(() =>
    expect(picker("Dota 2", "NVIDIA GeForce RTX 5090")).toBeEnabled(),
  );
  await user.selectOptions(
    picker("Dota 2", "NVIDIA GeForce RTX 5090"),
    "Quiet",
  );
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "The GPU tuning profile no longer exists.",
  );
  await user.click(screen.getByRole("button", { name: "Remove Portal 2" }));
  await waitFor(() =>
    expect(screen.queryByRole("article", { name: "Portal 2" })).toBeNull(),
  );
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});
