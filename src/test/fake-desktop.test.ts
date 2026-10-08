import { beforeEach, describe, expect, it, vi } from "vitest";

import fanCurvePreview from "@/test/contracts/fan-curve-preview.json";
import type { FanCurveSupport } from "@/features/gpus/types";
import { createFakeDesktop } from "./fake-desktop";
import {
  createPerformanceGpu,
  createPerformanceResponse,
} from "./gpu-fixtures";

// The Performance page tests trust the fake's fan curve previews, so it must judge curves exactly
// as the daemon does; the daemon is held to the same contract.
describe("fake desktop fan curve previews", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it.each(fanCurvePreview.limits)(
    "follow the daemon's contract for $name",
    async ({ curveSupport, cases }) => {
      createFakeDesktop({
        response: createPerformanceResponse({
          gpus: [
            createPerformanceGpu({
              cooling: { curveSupport: curveSupport as FanCurveSupport },
            }),
          ],
        }),
      });

      for (const { name, points, problems } of cases)
        expect(
          await window.prism.gpu.previewFanCurve({
            gpuId: "GPU-123",
            points: points.map(([temperatureC, speedPercent]) => ({
              temperatureC,
              speedPercent,
            })),
          }),
          name,
        ).toEqual({ problems });
    },
  );
});
