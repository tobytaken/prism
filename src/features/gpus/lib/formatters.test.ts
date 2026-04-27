import { describe, expect, it } from "vitest";

import { formatNumber, formatText, formatVendor } from "./formatters";

describe("formatters", () => {
  it("formats unavailable numbers", () => {
    expect(formatNumber(null)).toBe("Unavailable");
  });

  it("formats numbers with units", () => {
    expect(formatNumber(32_768, "MiB")).toBe("32,768 MiB");
  });

  it("formats unavailable text", () => {
    expect(formatText(null)).toBe("Unavailable");
  });

  it("formats GPU vendors", () => {
    expect(formatVendor("nvidia")).toBe("NVIDIA");
    expect(formatVendor("amd")).toBe("AMD");
    expect(formatVendor("intel")).toBe("Intel");
  });
});
