import { beforeEach, describe, expect, it, vi } from "vitest";
import { useQuery } from "@tanstack/react-query";

import { getGpuInventory } from "@/features/gpus/api";
import { useGpuInventory } from "./use-gpu-inventory";

vi.mock("@tanstack/react-query", () => ({
  useQuery: vi.fn(),
}));

vi.mock("@/features/gpus/api", () => ({
  getGpuInventory: vi.fn(),
}));

const mockedUseQuery = vi.mocked(useQuery);
const mockedGetGpuInventory = vi.mocked(getGpuInventory);

describe("useGpuInventory", () => {
  beforeEach(() => {
    mockedUseQuery.mockReset();
    mockedGetGpuInventory.mockReset();
  });

  it("configures the GPU inventory query", async () => {
    useGpuInventory();

    expect(mockedUseQuery).toHaveBeenCalledWith({
      queryKey: ["gpu-inventory"],
      queryFn: getGpuInventory,
      refetchInterval: 1_000,
      refetchIntervalInBackground: true,
    });

    await getQueryFn()();
    expect(mockedGetGpuInventory).toHaveBeenCalledTimes(1);
  });
});

function getQueryFn() {
  const options = mockedUseQuery.mock.calls[0]?.[0] as unknown as {
    queryFn: () => Promise<unknown>;
  };
  return options.queryFn;
}
