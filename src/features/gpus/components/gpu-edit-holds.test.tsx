import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router-dom";
import { beforeEach, expect, it, vi } from "vitest";
import { appRoutes } from "@/app/router";
import { createFakeDesktop } from "@/test/fake-desktop";
import {
  connectedDaemonStatus,
  createPerformanceResponse,
} from "@/test/gpu-fixtures";
import { renderWithQueryClient } from "@/test/query-client";

beforeEach(() => {
  vi.restoreAllMocks();
  vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
    connectedDaemonStatus,
  );
});

it("reports fan-only edits once per changed GPU set and releases on discard or apply", async () => {
  const user = userEvent.setup();
  const response = createPerformanceResponse();
  const gpu = response.gpus[0];
  createFakeDesktop({ response });
  const holds = vi.spyOn(window.prism.games, "setEditHolds");
  const router = createMemoryRouter(appRoutes, {
    initialEntries: ["/performance"],
  });
  renderWithQueryClient(<RouterProvider router={router} />);
  const fan = await screen.findByRole("spinbutton", {
    name: "Fan 1 manual target",
  });
  await user.clear(fan);
  await user.type(fan, "65");
  await waitFor(() => expect(holds).toHaveBeenLastCalledWith([gpu.stableId]));
  expect(holds.mock.calls.filter(([ids]) => ids.length > 0)).toHaveLength(1);
  await user.click(
    screen.getByRole("button", { name: "Discard fan 1 change" }),
  );
  await waitFor(() => expect(holds).toHaveBeenLastCalledWith([]));
  await user.clear(fan);
  await user.type(fan, "70");
  await user.click(screen.getByRole("button", { name: "Apply all" }));
  await waitFor(() => expect(holds).toHaveBeenLastCalledWith([]));
});

it("explains the wait in both places and confirms navigation with the GPU and waiting profile", async () => {
  const user = userEvent.setup();
  const response = createPerformanceResponse();
  const gpu = response.gpus[0];
  const fake = createFakeDesktop({ response });
  const router = createMemoryRouter(appRoutes, {
    initialEntries: ["/performance"],
  });
  renderWithQueryClient(<RouterProvider router={router} />);
  const power = await screen.findByRole("spinbutton", {
    name: "Power limit",
  });
  await user.clear(power);
  await user.type(power, "500");
  act(() =>
    fake.pushSwitchingStatus({
      condition: { status: "ready" },
      gpus: [
        {
          gpuId: gpu.stableId!,
          state: {
            status: "waitingForEdits",
            profile: { id: "gaming", name: "Gaming" },
          },
        },
      ],
    }),
  );
  const copy =
    'Automatic profile "Gaming" is waiting for unapplied tuning changes.';
  expect(await screen.findAllByText(copy, { exact: false })).toHaveLength(2);
  const panel = screen.getByRole("region", { name: "Game tuning" });
  expect(
    within(panel).getByRole("button", { name: "Apply my changes" }),
  ).toBeEnabled();
  expect(
    within(panel).getByRole("button", { name: "Discard my changes" }),
  ).toBeEnabled();
  await user.click(screen.getByRole("link", { name: "Overview" }));
  const dialog = await screen.findByRole("dialog", {
    name: "Discard unapplied tuning changes?",
  });
  expect(dialog).toHaveTextContent(gpu.name);
  expect(dialog).toHaveTextContent(copy);
  await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect(power).toHaveValue(500);
  expect(router.state.location.pathname).toBe("/performance");
  await user.click(screen.getByRole("link", { name: "Overview" }));
  await user.click(
    within(await screen.findByRole("dialog")).getByRole("button", {
      name: "Continue",
    }),
  );
  await waitFor(() => expect(router.state.location.pathname).toBe("/"));
});

it("keeps failed edits held after Apply my changes and releases them with Discard my changes", async () => {
  const user = userEvent.setup();
  const response = createPerformanceResponse();
  const gpu = response.gpus[0];
  const fake = createFakeDesktop({
    response,
    switchingStatus: {
      condition: { status: "ready" },
      gpus: [
        {
          gpuId: gpu.stableId!,
          state: {
            status: "waitingForEdits",
            profile: { id: "gaming", name: "Gaming" },
          },
        },
      ],
    },
  });
  const holds = vi.spyOn(window.prism.games, "setEditHolds");
  const router = createMemoryRouter(appRoutes, {
    initialEntries: ["/performance"],
  });
  renderWithQueryClient(<RouterProvider router={router} />);
  const power = await screen.findByRole("spinbutton", {
    name: "Power limit",
  });
  const fan = screen.getByRole("spinbutton", { name: "Fan 1 manual target" });
  await user.clear(power);
  await user.type(power, "500");
  await user.clear(fan);
  await user.type(fan, "65");
  fake.fanFailures[0] = "Fan refused the change";
  const panel = screen.getByRole("region", { name: "Game tuning" });
  await user.click(
    within(panel).getByRole("button", { name: "Apply my changes" }),
  );
  await screen.findByText(/Fan 1: Failed/);
  expect(fake.gpu.power.currentLimitWatts).toBe(500);
  expect(holds).toHaveBeenLastCalledWith([gpu.stableId]);
  await user.click(
    within(panel).getByRole("button", { name: "Discard my changes" }),
  );
  await waitFor(() => expect(holds).toHaveBeenLastCalledWith([]));
  expect(screen.queryByText("Unapplied changes")).not.toBeInTheDocument();
  expect(fake.gpu.power.currentLimitWatts).toBe(500);
});

it("includes fan-only edits in navigation confirmation and releases holds when the screen closes", async () => {
  const user = userEvent.setup();
  createFakeDesktop();
  const holds = vi.spyOn(window.prism.games, "setEditHolds");
  const router = createMemoryRouter(appRoutes, {
    initialEntries: ["/performance"],
  });
  const view = renderWithQueryClient(<RouterProvider router={router} />);
  const fan = await screen.findByRole("spinbutton", {
    name: "Fan 1 manual target",
  });
  await user.clear(fan);
  await user.type(fan, "65");
  await user.click(
    within(screen.getByRole("navigation")).getByRole("link", {
      name: "Games",
    }),
  );
  await screen.findByRole("dialog", {
    name: "Discard unapplied tuning changes?",
  });
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  view.unmount();
  await waitFor(() => expect(holds).toHaveBeenLastCalledWith([]));
});
