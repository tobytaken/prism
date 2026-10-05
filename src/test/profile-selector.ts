import { screen, within } from "@testing-library/react";
import type userEvent from "@testing-library/user-event";

type User = ReturnType<typeof userEvent.setup>;

/** The profile controls in the header of the GPU with this UUID (or fallback id). */
export function profileControls(gpuId = "GPU-123") {
  return screen.getByRole("group", {
    name: new RegExp(`profiles \\(${gpuId}\\)`),
  });
}

export async function findProfileControls(gpuId = "GPU-123") {
  return screen.findByRole("group", {
    name: new RegExp(`profiles \\(${gpuId}\\)`),
  });
}

/** The selector button, named after the GPU's current profile. */
export function profileSelector(gpuId = "GPU-123") {
  return within(profileControls(gpuId)).getByRole("button", {
    name: /^Profile:/,
  });
}

export async function openProfileMenu(user: User, gpuId = "GPU-123") {
  await findProfileControls(gpuId);
  await user.click(profileSelector(gpuId));
  return screen.findByRole("menu");
}

/** A profile's entry in the open menu, named after the profile and its values. */
export function profileEntry(name: string) {
  return within(screen.getByRole("menu")).getByRole("menuitemradio", {
    name: new RegExp(`^${name}\\b`),
  });
}

/** Chooses the profile in the GPU's selector, which applies it. */
export async function chooseProfile(
  user: User,
  name: string,
  gpuId = "GPU-123",
) {
  const menu = await openProfileMenu(user, gpuId);
  await within(menu).findByRole("menuitemradio", {
    name: new RegExp(`^${name}\\b`),
  });
  await user.click(profileEntry(name));
}

export async function closeProfileMenu(user: User) {
  await user.keyboard("{Escape}");
}

/** Confirms a profile switch while leaving the previous profile's saved values unchanged. */
export async function switchWithoutSaving(user: User) {
  const dialog = await screen.findByRole("dialog", { name: "Switch profile?" });
  await user.click(
    within(dialog).getByRole("button", { name: "Switch without saving" }),
  );
}

/** Opens Save as new… and saves under the name, leaving the dialog open if saving fails. */
export async function saveAsNew(user: User, name: string, gpuId = "GPU-123") {
  const menu = await openProfileMenu(user, gpuId);
  await user.click(
    within(menu).getByRole("menuitem", { name: "Save as new…" }),
  );
  const dialog = await screen.findByRole("dialog", {
    name: "Save as new profile",
  });
  await user.type(
    within(dialog).getByRole("textbox", { name: "Profile name" }),
    name,
  );
  await user.click(within(dialog).getByRole("button", { name: "Save" }));
  return dialog;
}

export async function saveToCurrent(user: User, gpuId = "GPU-123") {
  const menu = await openProfileMenu(user, gpuId);
  await user.click(within(menu).getByRole("menuitem", { name: /^Save to "/ }));
}

/** Whether a menu item in the open menu can be chosen. */
export function isEnabled(item: HTMLElement) {
  return item.getAttribute("aria-disabled") !== "true";
}

/** Whether Save as new… can be chosen, and the menu's text explaining what's blocked. */
export async function saveAsNewAvailability(user: User, gpuId = "GPU-123") {
  const menu = await openProfileMenu(user, gpuId);
  const enabled = isEnabled(
    within(menu).getByRole("menuitem", { name: "Save as new…" }),
  );
  const text = menu.textContent ?? "";
  await closeProfileMenu(user);
  return { enabled, text };
}

/** Whether the profile's entry can be chosen, and the menu's text explaining what's blocked. */
export async function profileAvailability(
  user: User,
  name: string,
  gpuId = "GPU-123",
) {
  await openProfileMenu(user, gpuId);
  const enabled = isEnabled(profileEntry(name));
  const text = screen.getByRole("menu").textContent ?? "";
  await closeProfileMenu(user);
  return { enabled, text };
}
