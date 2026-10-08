import { queryOptions } from "@tanstack/react-query";
import { previewGpuFanCurve } from "@/features/gpus/api";
import type {
  FanCurvePoint,
  GpuPerformanceDevice,
} from "@/features/gpus/types";

/** Editing feedback and arrangement Apply use the same daemon validation and cache entry. */
export function fanCurvePreviewOptions(
  gpu: GpuPerformanceDevice,
  points: FanCurvePoint[] | null,
  fanIndex?: number,
) {
  return queryOptions({
    queryKey: [
      "fan-curve-preview",
      gpu.id,
      gpu.stableId,
      fanIndex,
      points,
      gpu.cooling.fans
        .filter((fan) => fanIndex == null || fan.index === fanIndex)
        .map((fan) => [fan.index, fan.minSpeedPercent, fan.maxSpeedPercent]),
      gpu.cooling.maxOperatingTemperatureC,
      gpu.cooling.curveSupport,
    ],
    queryFn: () =>
      previewGpuFanCurve({
        gpuId: gpu.id,
        ...(fanIndex != null && { fanIndex }),
        points: points!,
      }),
    enabled: points != null,
  });
}
