import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { createInventoryGpu, createInventoryResponse } from "@/test/gpu-fixtures";
import { GpuDetailPage } from "./gpu-detail-page";

const mocks = vi.hoisted(() => ({
  gpuId: "GPU-456",
  inventoryQuery: {
    data: undefined as unknown,
    isPending: false,
    error: null as Error | null,
  },
  props: undefined as unknown,
}));

vi.mock("react-router-dom", async () => {
  const actual = await vi.importActual<typeof import("react-router-dom")>("react-router-dom");

  return {
    ...actual,
    useParams: () => ({ gpuId: mocks.gpuId }),
  };
});

vi.mock("@/features/gpus/hooks/use-gpu-inventory", () => ({
  useGpuInventory: () => mocks.inventoryQuery,
}));

vi.mock("./gpu-detail-screen", () => ({
  GpuDetailScreen: vi.fn((props) => {
    mocks.props = props;
    return <div data-testid="gpu-detail-screen" />;
  }),
}));

describe("GpuDetailPage", () => {
  beforeEach(() => {
    mocks.gpuId = "GPU-456";
    mocks.inventoryQuery.data = undefined;
    mocks.inventoryQuery.isPending = false;
    mocks.inventoryQuery.error = null;
    mocks.props = undefined;
  });

  it("selects the route GPU from inventory", () => {
    const selectedGpu = createInventoryGpu({ id: "GPU-456", name: "Selected GPU" });
    mocks.inventoryQuery.data = createInventoryResponse({
      gpus: [createInventoryGpu({ id: "GPU-123" }), selectedGpu],
    });

    render(<GpuDetailPage />);

    expect(screen.getByTestId("gpu-detail-screen")).toBeInTheDocument();
    expect(mocks.props).toMatchObject({
      gpu: selectedGpu,
      isLoading: false,
      errorMessage: null,
    });
  });
});
