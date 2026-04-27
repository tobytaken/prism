import { beforeEach, describe, expect, it, vi } from "vitest";
import { useQuery } from "@tanstack/react-query";

import { getGpuPerformance } from "@/features/gpus/api";
import { useGpuPerformance } from "./use-gpu-performance";

vi.mock("@tanstack/react-query", () => ({
  useQuery: vi.fn(),
}));

vi.mock("@/features/gpus/api", () => ({
  getGpuPerformance: vi.fn(),
}));

const mockedUseQuery = vi.mocked(useQuery);
const mockedGetGpuPerformance = vi.mocked(getGpuPerformance);

describe("useGpuPerformance", () => {
  beforeEach(() => {
    mockedUseQuery.mockReset();
    mockedGetGpuPerformance.mockReset();
  });

  it("configures the GPU performance query", async () => {
    useGpuPerformance();

    expect(mockedUseQuery).toHaveBeenCalledWith({
      queryKey: ["gpu-performance"],
      queryFn: getGpuPerformance,
      refetchInterval: 500,
      refetchIntervalInBackground: true,
    });

    await getQueryFn()();
    expect(mockedGetGpuPerformance).toHaveBeenCalledTimes(1);
  });
});

function getQueryFn() {
  const options = mockedUseQuery.mock.calls[0]?.[0] as unknown as {
    queryFn: () => Promise<unknown>;
  };
  return options.queryFn;
}
