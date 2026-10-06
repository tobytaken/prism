import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, expect, it, vi } from "vitest";

import type { GpuTuningProfile } from "@/features/gpus/types";
import { createFakeDesktop } from "@/test/fake-desktop";
import {
  connectedDaemonStatus,
  createPerformanceGpu,
  createPerformanceResponse,
} from "@/test/gpu-fixtures";
import {
  openProfileMenu,
  profileEntry,
  profileSelector,
} from "@/test/profile-selector";
import { renderWithQueryClient } from "@/test/query-client";
import { GpuPerformancePage } from "./gpu-performance-page";

const quiet: GpuTuningProfile = {
  id: "quiet",
  name: "Quiet",
  gpuId: "GPU-123",
  settings: { powerLimitWatts: { kind: "value", value: 450 } },
};

function renderPerformancePage() {
  return renderWithQueryClient(
    <MemoryRouter>
      <GpuPerformancePage />
    </MemoryRouter>,
  );
}

async function manageProfiles(
  user: ReturnType<typeof userEvent.setup>,
  gpuId = "GPU-123",
) {
  const menu = await openProfileMenu(user, gpuId);
  await user.click(
    within(menu).getByRole("menuitem", { name: "Manage profiles…" }),
  );
  return screen.findByRole("dialog", { name: "Manage profiles" });
}

beforeEach(() => {
  vi.restoreAllMocks();
  vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
    connectedDaemonStatus,
  );
});

it.each(["pending", "failed"])(
  "waits for the current profile to be known before allowing deletion when its state is %s",
  async (read) => {
    const user = userEvent.setup();
    const fake = createFakeDesktop({
      profiles: [structuredClone(quiet)],
      currentProfiles: { "GPU-123": "quiet" },
    });
    if (read === "pending")
      fake.getTuningStates.mockImplementation(
        () => new Promise<never>(() => {}),
      );
    else
      fake.getTuningStates.mockRejectedValue(
        new Error("Failed to read the current profile."),
      );
    renderPerformancePage();
    const dialog = await manageProfiles(user);
    expect(
      within(dialog).getByRole("button", { name: "Delete Quiet" }),
    ).toBeDisabled();
    expect(within(dialog).getByRole("status")).toHaveTextContent(
      "Deletion is unavailable until this GPU's current profile can be read. Try again once the Prism daemon is available.",
    );
    expect(
      within(dialog).getByRole("button", { name: "Rename Quiet" }),
    ).toBeEnabled();
    expect(fake.profiles).toEqual([quiet]);
  },
);

it("renames the current profile with a trimmed name and keeps it current under its new name", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    profiles: [structuredClone(quiet)],
    currentProfiles: { "GPU-123": "quiet" },
  });
  renderPerformancePage();
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));
  const dialog = await manageProfiles(user);
  await user.click(
    within(dialog).getByRole("button", { name: "Rename Quiet" }),
  );
  const input = within(dialog).getByRole("textbox", { name: "Profile name" });
  await user.clear(input);
  await user.type(input, "  Silent  ");
  await user.click(within(dialog).getByRole("button", { name: "Save name" }));

  await within(dialog).findByRole("button", { name: "Rename Silent" });
  expect(fake.profiles).toEqual([{ ...quiet, name: "Silent" }]);
  expect(fake.currentProfiles).toEqual({ "GPU-123": "quiet" });
  expect(fake.gpu.power.currentLimitWatts).toBe(475);
  await user.click(within(dialog).getByRole("button", { name: "Done" }));
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Silent"));
  await openProfileMenu(user);
  expect(profileEntry("Silent")).toBeChecked();
});

it("confirms the current-profile role before deleting, keeps tuning values, and shows No current profile", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    profiles: [structuredClone(quiet)],
    currentProfiles: { "GPU-123": "quiet" },
  });
  renderPerformancePage();
  await waitFor(() => expect(profileSelector()).toHaveTextContent("Quiet"));
  const dialog = await manageProfiles(user);
  await user.click(
    within(dialog).getByRole("button", { name: "Delete Quiet" }),
  );
  const confirmation = await screen.findByRole("alertdialog", {
    name: 'Delete "Quiet"?',
  });
  expect(confirmation).toHaveTextContent(
    "This is the current profile. After deletion, this GPU will have no current profile.",
  );
  expect(confirmation).toHaveTextContent(
    "Current tuning values will stay unchanged.",
  );
  expect(fake.profiles).toEqual([quiet]);
  await user.click(
    within(confirmation).getByRole("button", { name: "Cancel" }),
  );
  expect(fake.profiles).toEqual([quiet]);

  await user.click(
    within(dialog).getByRole("button", { name: "Delete Quiet" }),
  );
  await user.click(
    within(await screen.findByRole("alertdialog")).getByRole("button", {
      name: "Delete profile",
    }),
  );
  await waitFor(() =>
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument(),
  );
  expect(fake.profiles).toEqual([]);
  expect(fake.currentProfiles).toEqual({});
  expect(fake.gpu.power.currentLimitWatts).toBe(475);
  expect(fake.gpu.clocks.graphics.currentOffsetMHz).toBe(120);
  await user.click(within(dialog).getByRole("button", { name: "Done" }));
  await waitFor(() =>
    expect(profileSelector()).toHaveTextContent("No current profile"),
  );
  const menu = await openProfileMenu(user);
  expect(within(menu).queryByRole("menuitemradio")).not.toBeInTheDocument();
});

it("lists only the caller's profiles for that GPU and explains a case-insensitive rename clash", async () => {
  const user = userEvent.setup();
  const loud = { ...quiet, id: "loud", name: "Loud" };
  const other = { ...quiet, id: "other", name: "Other GPU", gpuId: "GPU-456" };
  const theirs = { ...quiet, id: "theirs", name: "Another user's" };
  const fake = createFakeDesktop({
    response: createPerformanceResponse({
      gpus: [
        createPerformanceGpu(),
        createPerformanceGpu({ id: "GPU-456", stableId: "GPU-456" }),
      ],
    }),
    profiles: structuredClone([quiet, loud, other]),
    otherUsersProfiles: [theirs],
  });
  renderPerformancePage();
  const dialog = await manageProfiles(user);
  await within(dialog).findByRole("button", { name: "Rename Quiet" });
  expect(within(dialog).getAllByRole("listitem")).toHaveLength(2);
  expect(within(dialog).queryByText("Other GPU")).not.toBeInTheDocument();
  expect(within(dialog).queryByText("Another user's")).not.toBeInTheDocument();
  await user.click(
    within(dialog).getByRole("button", { name: "Rename Quiet" }),
  );
  const input = within(dialog).getByRole("textbox", { name: "Profile name" });
  await user.clear(input);
  expect(
    within(dialog).getByRole("button", { name: "Save name" }),
  ).toBeDisabled();
  await user.type(input, "  lOuD  ");
  await user.click(within(dialog).getByRole("button", { name: "Save name" }));
  expect(await within(dialog).findByRole("alert")).toHaveTextContent(
    'A profile named "lOuD" already exists for this GPU. Choose another name.',
  );
  expect(fake.profiles).toEqual([quiet, loud, other]);
  await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
  await user.click(within(dialog).getByRole("button", { name: "Done" }));

  const otherDialog = await manageProfiles(user, "GPU-456");
  await user.click(
    within(otherDialog).getByRole("button", { name: "Rename Other GPU" }),
  );
  const otherInput = within(otherDialog).getByRole("textbox", {
    name: "Profile name",
  });
  await user.clear(otherInput);
  await user.type(otherInput, "Loud");
  await user.click(
    within(otherDialog).getByRole("button", { name: "Save name" }),
  );
  await within(otherDialog).findByRole("button", { name: "Rename Loud" });
  expect(fake.profiles.find(({ id }) => id === "other")?.name).toBe("Loud");
});

it("keeps a non-current deletion separate from another user's current profile and shows deletion failures", async () => {
  const user = userEvent.setup();
  const theirs = { ...quiet, id: "theirs", name: "Another user's" };
  const fake = createFakeDesktop({
    profiles: [structuredClone(quiet)],
    otherUsersProfiles: [theirs],
    currentProfiles: { "GPU-123": "theirs" },
  });
  renderPerformancePage();
  await waitFor(() =>
    expect(profileSelector()).toHaveTextContent("Another user's"),
  );
  const dialog = await manageProfiles(user);
  await user.click(
    within(dialog).getByRole("button", { name: "Delete Quiet" }),
  );
  const confirmation = await screen.findByRole("alertdialog");
  expect(confirmation).not.toHaveTextContent("This is the current profile.");
  fake.deleteProfile.mockResolvedValueOnce({
    success: false,
    error: "Failed to write Prism's tuning store.",
  });
  await user.click(
    within(confirmation).getByRole("button", { name: "Delete profile" }),
  );
  expect(await within(confirmation).findByRole("alert")).toHaveTextContent(
    "Failed to write Prism's tuning store.",
  );
  expect(fake.profiles).toEqual([quiet]);
  await user.click(
    within(confirmation).getByRole("button", { name: "Delete profile" }),
  );
  await waitFor(() =>
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument(),
  );
  await user.click(within(dialog).getByRole("button", { name: "Done" }));
  expect(profileSelector()).toHaveTextContent("Another user's");
  expect(fake.currentProfiles).toEqual({ "GPU-123": "theirs" });
});

it("designates a startup profile, stars it, warns on save and confirms its startup role on delete", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    profiles: [structuredClone(quiet)],
    currentProfiles: { "GPU-123": "quiet" },
    startupBehavior: "startupProfiles",
  });
  renderPerformancePage();
  const dialog = await manageProfiles(user);
  const picker = await within(dialog).findByRole("combobox", {
    name: "Startup profile",
  });
  expect(picker).toHaveValue("");
  expect(dialog).toHaveTextContent("Startup behavior: Startup profiles");
  expect(
    within(dialog).getByRole("link", { name: "Settings" }),
  ).toHaveAttribute("href", "/settings");
  await user.selectOptions(picker, "quiet");
  await waitFor(() =>
    expect(fake.startupProfiles).toEqual({ "GPU-123": "quiet" }),
  );
  await user.click(within(dialog).getByRole("button", { name: "Done" }));
  const menu = await openProfileMenu(user);
  await waitFor(() =>
    expect(profileEntry("Quiet")).toHaveTextContent("Startup profile"),
  );
  expect(menu).toHaveTextContent("Also changes the values applied at startup.");
  await user.keyboard("{Escape}");
  const again = await manageProfiles(user);
  await user.click(within(again).getByRole("button", { name: "Delete Quiet" }));
  const confirmation = await screen.findByRole("alertdialog");
  expect(confirmation).toHaveTextContent(
    "This is the startup profile. After deletion, this GPU will have no startup profile.",
  );
  await user.click(
    within(confirmation).getByRole("button", { name: "Delete profile" }),
  );
  await waitFor(() => expect(fake.startupProfiles).toEqual({}));
});

it("allows an out-of-range startup designation with a warning, and shows another user's designation by name", async () => {
  const user = userEvent.setup();
  const outside = {
    ...quiet,
    id: "outside",
    name: "Too high",
    settings: { powerLimitWatts: { kind: "value" as const, value: 700 } },
  };
  const theirs = { ...quiet, id: "theirs", name: "Silent" };
  const fake = createFakeDesktop({
    profiles: [outside],
    otherUsersProfiles: [theirs],
    startupProfiles: { "GPU-123": "theirs" },
  });
  renderPerformancePage();
  const dialog = await manageProfiles(user);
  const picker = await within(dialog).findByRole("combobox", {
    name: "Startup profile",
  });
  expect(picker).toHaveValue("theirs");
  expect(dialog).toHaveTextContent("Silent (another user's profile)");
  await user.selectOptions(picker, "outside");
  await waitFor(() => expect(picker).toHaveValue("outside"));
  expect(
    await within(dialog).findByText(/Boot will skip this profile/),
  ).toHaveTextContent("out of range");
  expect(fake.startupProfiles).toEqual({ "GPU-123": "outside" });
  expect(fake.applyProfileById).not.toHaveBeenCalled();
});

it("can designate and delete an absent GPU's startup profile with a startup-role confirmation", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    response: createPerformanceResponse({ gpus: [] }),
    profiles: [structuredClone(quiet)],
  });
  renderPerformancePage();
  await user.click(
    await screen.findByRole("button", {
      name: "Manage profiles for Quiet (GPU-123)",
    }),
  );
  const dialog = await screen.findByRole("dialog", { name: "Manage profiles" });
  await user.selectOptions(
    within(dialog).getByRole("combobox", { name: "Startup profile" }),
    "quiet",
  );
  expect(
    await within(dialog).findByText(
      /Boot will skip this profile while its GPU is absent/,
    ),
  ).toBeInTheDocument();
  expect(fake.startupProfiles).toEqual({ "GPU-123": "quiet" });
  await user.click(
    within(dialog).getByRole("button", { name: "Delete Quiet" }),
  );
  const confirmation = await screen.findByRole("alertdialog", {
    name: 'Delete "Quiet"?',
  });
  expect(confirmation).toHaveTextContent("This is the startup profile.");
  expect(confirmation).toHaveTextContent(
    "Any current-profile designation for this profile will be cleared.",
  );
  await user.click(
    within(confirmation).getByRole("button", { name: "Delete profile" }),
  );
  await waitFor(() => expect(fake.profiles).toEqual([]));
  expect(fake.startupProfiles).toEqual({});
});

it("shows Restore last applied as the host-wide behavior in Manage profiles", async () => {
  const user = userEvent.setup();
  createFakeDesktop({ startupBehavior: "restoreLastApplied" });
  renderPerformancePage();
  const dialog = await manageProfiles(user);
  expect(dialog).toHaveTextContent("Startup behavior: Restore last applied");
});

it("lists each profile's linked games in the selector menu and links Manage profiles to the Games page", async () => {
  const user = userEvent.setup();
  const loud = { ...quiet, id: "loud", name: "Loud" };
  createFakeDesktop({
    profiles: structuredClone([quiet, loud]),
    steamApps: [{ appId: 570, name: "Dota 2" }],
    games: [
      {
        game: { kind: "steam", appId: 570 },
        name: "Dota 2",
        profiles: { "GPU-123": "quiet" },
      },
      {
        game: { kind: "steam", appId: 620 },
        name: "Portal 2",
        profiles: { "GPU-123": "quiet" },
      },
    ],
  });
  renderPerformancePage();
  await openProfileMenu(user);
  await waitFor(() =>
    expect(profileEntry("Quiet")).toHaveTextContent("Games: Dota 2, Portal 2"),
  );
  expect(profileEntry("Loud")).not.toHaveTextContent("Games:");
  await user.keyboard("{Escape}");
  const dialog = await manageProfiles(user);
  expect(within(dialog).getByRole("link", { name: "Games" })).toHaveAttribute(
    "href",
    "/games",
  );
});

it("mentions linked games when confirming a deletion and removes the profile's links", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({
    response: createPerformanceResponse({
      gpus: [
        createPerformanceGpu(),
        createPerformanceGpu({ id: "GPU-456", stableId: "GPU-456" }),
      ],
    }),
    profiles: structuredClone([
      quiet,
      { ...quiet, id: "loud", name: "Loud", gpuId: "GPU-456" },
    ]),
    steamApps: [{ appId: 570, name: "Dota 2" }],
    games: [
      {
        game: { kind: "steam", appId: 570 },
        name: "Dota 2",
        profiles: { "GPU-123": "quiet", "GPU-456": "loud" },
      },
    ],
  });
  renderPerformancePage();
  const dialog = await manageProfiles(user);
  await user.click(
    await within(dialog).findByRole("button", { name: "Delete Quiet" }),
  );
  const confirmation = await screen.findByRole("alertdialog", {
    name: 'Delete "Quiet"?',
  });
  expect(confirmation).toHaveTextContent(
    "It's linked to Dota 2. Those links will be removed, and the game will leave this GPU unchanged.",
  );
  await user.click(
    within(confirmation).getByRole("button", { name: "Delete profile" }),
  );
  await waitFor(() =>
    expect(fake.games[0].profiles).toEqual({ "GPU-456": "loud" }),
  );
});

it("waits for game associations to be known before allowing deletion", async () => {
  const user = userEvent.setup();
  const fake = createFakeDesktop({ profiles: [structuredClone(quiet)] });
  fake.getGames.mockRejectedValue(new Error("The daemon is unavailable."));
  renderPerformancePage();
  const dialog = await manageProfiles(user);
  await waitFor(() =>
    expect(within(dialog).getByRole("status")).toHaveTextContent(
      "Deletion is unavailable until game associations can be read. Try again once the Prism daemon is available.",
    ),
  );
  expect(
    within(dialog).getByRole("button", { name: "Delete Quiet" }),
  ).toBeDisabled();
});
