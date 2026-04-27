import { afterEach, beforeEach, describe, expect, it } from "vitest";

import {
  applyAccentTheme,
  getStoredAccentPreference,
  resolveDetectedAccent,
  resolveEffectiveAccent,
  setStoredAccentPreference,
} from "./accent-theme";

describe("accent theme", () => {
  const storage = new Map<string, string>();

  beforeEach(() => {
    storage.clear();
    Object.defineProperty(window, "localStorage", {
      configurable: true,
      value: {
        getItem: (key: string) => storage.get(key) ?? null,
        setItem: (key: string, value: string) => {
          storage.set(key, value);
        },
        removeItem: (key: string) => {
          storage.delete(key);
        },
        clear: () => {
          storage.clear();
        },
      },
    });
    applyAccentTheme(null);
  });

  afterEach(() => {
    storage.clear();
    applyAccentTheme(null);
  });

  it("falls back to auto for invalid stored preferences", () => {
    window.localStorage.setItem("prism.accent-preference", "orange");

    expect(getStoredAccentPreference()).toBe("auto");
  });

  it("returns valid stored preferences", () => {
    setStoredAccentPreference("intel");

    expect(getStoredAccentPreference()).toBe("intel");
  });

  it("detects the first supported GPU vendor", () => {
    expect(
      resolveDetectedAccent([
        { vendor: "unknown" },
        { vendor: "NVIDIA" },
        { vendor: "amd" },
      ]),
    ).toBe("nvidia");
  });

  it("returns null when no known GPU vendor is present", () => {
    expect(resolveDetectedAccent([{ vendor: "unknown" }, { vendor: null }])).toBeNull();
  });

  it("resolves auto preferences to the detected accent", () => {
    expect(resolveEffectiveAccent("auto", "amd")).toBe("amd");
  });

  it("resolves manual preferences over detected accents", () => {
    expect(resolveEffectiveAccent("intel", "nvidia")).toBe("intel");
  });

  it("removes accent CSS variables for a null accent", () => {
    applyAccentTheme("nvidia");
    applyAccentTheme(null);

    expect(document.documentElement.style.getPropertyValue("--primary")).toBe("");
    expect(document.documentElement.style.getPropertyValue("--ring")).toBe("");
    expect(document.documentElement.style.getPropertyValue("--sidebar-primary")).toBe("");
    expect(document.documentElement.style.getPropertyValue("--chart-1")).toBe("");
  });

  it("sets accent CSS variables", () => {
    applyAccentTheme("nvidia");

    expect(document.documentElement.style.getPropertyValue("--primary")).toBe("#76B900");
    expect(document.documentElement.style.getPropertyValue("--ring")).toContain("#76B900");
    expect(document.documentElement.style.getPropertyValue("--sidebar-primary")).toBe("#76B900");
    expect(document.documentElement.style.getPropertyValue("--chart-1")).toBe("#76B900");
  });
});
