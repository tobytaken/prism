import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { SettingsPage } from "./settings-page";

const mocks = vi.hoisted(() => ({
  appearance: {
    accentPreference: "auto",
    effectiveAccent: "nvidia",
    detectedAccent: "nvidia",
    isDetectingAccent: false,
    detectionErrorMessage: null as string | null,
    setAccentPreference: vi.fn(),
  },
  props: undefined as unknown,
}));

vi.mock("@/features/settings/lib/appearance-provider", () => ({
  useAppearance: () => mocks.appearance,
}));

vi.mock("@/features/settings/components/settings-screen", () => ({
  SettingsScreen: vi.fn((props) => {
    mocks.props = props;
    return <div data-testid="settings-screen" />;
  }),
}));

describe("SettingsPage", () => {
  beforeEach(() => {
    mocks.appearance.accentPreference = "auto";
    mocks.appearance.effectiveAccent = "nvidia";
    mocks.appearance.detectedAccent = "nvidia";
    mocks.appearance.isDetectingAccent = false;
    mocks.appearance.detectionErrorMessage = null;
    mocks.appearance.setAccentPreference.mockReset();
    mocks.props = undefined;
  });

  it("passes appearance context into the settings screen", () => {
    render(<SettingsPage />);

    expect(screen.getByTestId("settings-screen")).toBeInTheDocument();
    expect(mocks.props).toMatchObject({
      accentPreference: "auto",
      effectiveAccent: "nvidia",
      detectedAccent: "nvidia",
      isDetectingAccent: false,
      detectionErrorMessage: null,
      onAccentPreferenceChange: mocks.appearance.setAccentPreference,
    });
  });
});
