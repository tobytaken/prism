import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";

import { createInventoryGpu } from "@/test/gpu-fixtures";
import { GpuDetailScreen } from "./gpu-detail-screen";

function renderScreen(ui: React.ReactNode) {
  return render(<MemoryRouter>{ui}</MemoryRouter>);
}

describe("GpuDetailScreen", () => {
  it("renders a loading skeleton", () => {
    renderScreen(<GpuDetailScreen isLoading errorMessage={null} />);

    expect(screen.getByTestId("gpu-detail-skeleton")).toBeInTheDocument();
  });

  it("renders the selected GPU details and specs", () => {
    renderScreen(
      <GpuDetailScreen gpu={createInventoryGpu()} isLoading={false} errorMessage={null} />,
    );

    expect(screen.getByRole("heading", { name: "NVIDIA GeForce RTX 5090" })).toBeInTheDocument();
    expect(
      screen.getByText("NVIDIA", { selector: '[data-vendor="nvidia"]' }),
    ).toHaveAttribute("data-vendor", "nvidia");
    expect(screen.getByText("VRAM")).toBeInTheDocument();
    expect(screen.getAllByText("32,768 MiB").length).toBeGreaterThan(0);
    expect(screen.getByText("Core Count")).toBeInTheDocument();
    expect(screen.getByText("Current Graphics Clock")).toBeInTheDocument();
    expect(screen.getAllByText("1,365 MHz").length).toBeGreaterThan(0);
    expect(screen.getByText("Gen 5 x16")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /back to overview/i })).toBeInTheDocument();
  });

  it("renders a missing gpu state", () => {
    renderScreen(<GpuDetailScreen isLoading={false} errorMessage={null} />);

    expect(screen.getByText("GPU not found")).toBeInTheDocument();
  });
});
