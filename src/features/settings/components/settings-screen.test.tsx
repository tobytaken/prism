import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { SettingsScreen } from "./settings-screen";

describe("SettingsScreen", () => {
  it("renders the appearance tab with brand accent options", () => {
    render(
      <SettingsScreen
        accentPreference="auto"
        effectiveAccent="nvidia"
        detectedAccent="nvidia"
        isDetectingAccent={false}
        detectionErrorMessage={null}
        onAccentPreferenceChange={() => undefined}
      />,
    );

    expect(screen.getByRole("tab", { name: "Appearance" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("radio", { name: /^AMD$/i })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /^NVIDIA$/i })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /^Intel$/i })).toBeInTheDocument();
    
    // Auto radio contains the summary text, so we use a regex that starts with Auto
    const autoOption = screen.getByRole("radio", { name: /^Auto/i });

    expect(autoOption).toBeInTheDocument();
    expect(within(autoOption).getByText(/Currently resolves to NVIDIA Green/i)).toBeInTheDocument();
  });

  it("changes the selected accent and can switch tabs", async () => {
    const user = userEvent.setup();
    const onAccentPreferenceChange = vi.fn();

    render(
      <SettingsScreen
        accentPreference="auto"
        effectiveAccent="nvidia"
        detectedAccent="nvidia"
        isDetectingAccent={false}
        detectionErrorMessage={null}
        onAccentPreferenceChange={onAccentPreferenceChange}
      />,
    );

    await user.click(screen.getByRole("radio", { name: /^Intel$/i }));
    expect(onAccentPreferenceChange).toHaveBeenCalledWith("intel");

    await user.click(screen.getByRole("tab", { name: "About" }));
    expect(screen.getByText(/Platform Scope/i)).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Appearance" }));
    expect(screen.getByText("Accent Color")).toBeInTheDocument();
  });
});
