import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { createInventoryResponse } from "@/test/gpu-fixtures";
import { GpuOverviewPage } from "./gpu-overview-page";

const mocks = vi.hoisted(() => ({
  inventoryQuery: {
    data: undefined as unknown,
    isPending: false,
    error: null as Error | null,
  },
  props: undefined as unknown,
}));

vi.mock("@/features/gpus/hooks/use-gpu-inventory", () => ({
  useGpuInventory: () => mocks.inventoryQuery,
}));

vi.mock("./gpu-overview-screen", () => ({
  GpuOverviewScreen: vi.fn((props) => {
    mocks.props = props;
    return <div data-testid="gpu-overview-screen" />;
  }),
}));

describe("GpuOverviewPage", () => {
  beforeEach(() => {
    mocks.inventoryQuery.data = undefined;
    mocks.inventoryQuery.isPending = false;
    mocks.inventoryQuery.error = null;
    mocks.props = undefined;
  });

  it("passes inventory query state into the overview screen", () => {
    const inventory = createInventoryResponse();
    mocks.inventoryQuery.data = inventory;
    mocks.inventoryQuery.isPending = true;
    mocks.inventoryQuery.error = new Error("inventory failed");

    render(<GpuOverviewPage />);

    expect(screen.getByTestId("gpu-overview-screen")).toBeInTheDocument();
    expect(mocks.props).toMatchObject({
      inventory,
      isLoading: true,
      errorMessage: "inventory failed",
    });
  });
});
