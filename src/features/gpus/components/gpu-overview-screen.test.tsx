import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";

import { createInventoryResponse } from "@/test/gpu-fixtures";
import { GpuOverviewScreen } from "./gpu-overview-screen";

describe("GpuOverviewScreen", () => {
  it("renders a loading skeleton", () => {
    render(
      <MemoryRouter>
        <GpuOverviewScreen isLoading errorMessage={null} />
      </MemoryRouter>,
    );

    expect(screen.getByTestId("gpu-overview-skeleton")).toBeInTheDocument();
  });

  it("renders the detected GPUs as a navigable list", () => {
    render(
      <MemoryRouter>
        <GpuOverviewScreen
          inventory={createInventoryResponse()}
          isLoading={false}
          errorMessage={null}
        />
      </MemoryRouter>,
    );

    expect(screen.getByRole("heading", { name: "NVIDIA GeForce RTX 5090" })).toBeInTheDocument();
    expect(
      screen.getByText("NVIDIA", { selector: '[data-vendor="nvidia"]' }),
    ).toHaveAttribute("data-vendor", "nvidia");
    expect(screen.getByText("VRAM")).toBeInTheDocument();
    expect(screen.getAllByText("32,768 MiB").length).toBeGreaterThan(0);
    expect(screen.getByText("Power Limit")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /nvidia geforce rtx 5090/i })).toHaveAttribute(
      "href",
      "/gpus/GPU-123",
    );
    expect(screen.queryByRole("button", { name: "Apply" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Reset to Default" })).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Reapply Saved Settings" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText("Privileged GPU controls are connected"),
    ).not.toBeInTheDocument();
  });

  it("renders an empty state with warnings", () => {
    render(
      <MemoryRouter>
        <GpuOverviewScreen
          inventory={{
            gpus: [],
            warnings: ["NVIDIA inventory is unavailable on this system."],
          }}
          isLoading={false}
          errorMessage={null}
        />
      </MemoryRouter>,
    );

    expect(screen.getByText("No supported GPUs detected")).toBeInTheDocument();
    expect(screen.getByText("Warnings")).toBeInTheDocument();
    expect(
      screen.getByText("NVIDIA inventory is unavailable on this system."),
    ).toBeInTheDocument();
  });
});
