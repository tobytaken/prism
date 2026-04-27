import { describe, expect, it } from "vitest";

import {
  buildSparklinePath,
  calculateMetricStats,
  calculateSparklineBounds,
  calculateSparklinePoints,
} from "./performance-chart";

const insets = {
  top: 4,
  right: 4,
  bottom: 4,
  left: 4,
};

describe("performance chart helpers", () => {
  it("calculates bounds while ignoring null samples", () => {
    expect(calculateSparklineBounds([null, 10, 24, null, 18])).toEqual({
      min: 10,
      max: 24,
    });
  });

  it("returns null bounds when every sample is null", () => {
    expect(calculateSparklineBounds([null, null])).toBeNull();
  });

  it("calculates metric stats while ignoring null samples", () => {
    expect(calculateMetricStats([10, null, 20, 30])).toEqual({
      min: 10,
      max: 30,
      average: 20,
    });
  });

  it("handles a single sparkline sample", () => {
    expect(calculateSparklinePoints([42], 100, 50, insets)).toEqual([
      { x: 4, y: 46 },
    ]);
  });

  it("builds a baseline fallback path for no data", () => {
    expect(buildSparklinePath([null], 100, 50, insets)).toBe("M 4 46");
  });
});
