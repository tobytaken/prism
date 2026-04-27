import { beforeEach, describe, expect, it, vi } from "vitest";
import { useQuery } from "@tanstack/react-query";

import { getGpuDaemonStatus } from "@/features/gpus/api";
import { useGpuDaemonStatus } from "./use-gpu-daemon-status";

vi.mock("@tanstack/react-query", () => ({
  useQuery: vi.fn(),
}));

vi.mock("@/features/gpus/api", () => ({
  getGpuDaemonStatus: vi.fn(),
}));

const mockedUseQuery = vi.mocked(useQuery);
const mockedGetGpuDaemonStatus = vi.mocked(getGpuDaemonStatus);

describe("useGpuDaemonStatus", () => {
  beforeEach(() => {
    mockedUseQuery.mockReset();
    mockedGetGpuDaemonStatus.mockReset();
  });

  it("configures the GPU daemon status query", async () => {
    useGpuDaemonStatus();

    expect(mockedUseQuery).toHaveBeenCalledWith({
      queryKey: ["gpu-daemon-status"],
      queryFn: getGpuDaemonStatus,
      refetchInterval: 2_000,
      refetchIntervalInBackground: true,
    });

    await getQueryFn()();
    expect(mockedGetGpuDaemonStatus).toHaveBeenCalledTimes(1);
  });
});

function getQueryFn() {
  const options = mockedUseQuery.mock.calls[0]?.[0] as unknown as {
    queryFn: () => Promise<unknown>;
  };
  return options.queryFn;
}
