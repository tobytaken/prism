import { beforeEach, describe, expect, it, vi } from "vitest";

import fanCurvePreview from "@/test/contracts/fan-curve-preview.json";
import coolingApplication from "@/test/contracts/cooling-application.json";
import type {
  ApplyCoolingRequest,
  FanCurveSupport,
} from "@/features/gpus/types";
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

describe("fake desktop cooling applications", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("request the daemon's curve speeds from the shared contract", async () => {
    const fake = createFakeDesktop({
      response: createPerformanceResponse({
        gpus: [
          createPerformanceGpu({
            cooling: {
              curveSupport: fanCurvePreview.limits[0]
                .curveSupport as FanCurveSupport,
            },
          }),
        ],
      }),
    });
    for (const [
      temperatureC,
      speedPercent,
    ] of coolingApplication.requestedAtTemperature) {
      fake.gpu.telemetry.temperatureC = temperatureC;
      await window.prism.gpu.applyCooling(
        coolingApplication.request as ApplyCoolingRequest,
      );
      expect(
        fake.gpu.cooling.control?.requested[0].speedPercent,
        `${temperatureC}°C`,
      ).toBe(speedPercent);
    }
  });
});
