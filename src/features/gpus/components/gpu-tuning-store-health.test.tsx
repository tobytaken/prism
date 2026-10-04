import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";

import unhealthyStoreStatus from "@/test/contracts/daemon-status-unhealthy-store.json";
import {
  connectedDaemonStatus,
  createPerformanceResponse,
} from "@/test/gpu-fixtures";
import { renderWithQueryClient } from "@/test/query-client";
import type { GpuDaemonStatus } from "@/features/gpus/types";
import { GpuPerformancePage } from "./gpu-performance-page";

const newerVersionStatus = unhealthyStoreStatus as GpuDaemonStatus;
const unreadableStatus: GpuDaemonStatus = {
  ...connectedDaemonStatus,
  tuningStore: {
    status: "unhealthy",
    problem: "unreadable",
    path: "/var/lib/prism/tuning-state.json",
    detail: "EOF while parsing an object at line 1 column 31",
  },
};

function renderPerformancePage() {
  return renderWithQueryClient(
    <MemoryRouter>
      <GpuPerformancePage />
    </MemoryRouter>,
  );
}

describe("Tuning store health on Performance", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.spyOn(window.prism.gpu, "getPerformance").mockResolvedValue(
      createPerformanceResponse(),
    );
  });

  it("explains an unreadable store and keeps manual tuning available", async () => {
    const user = userEvent.setup();
    vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
      unreadableStatus,
    );
    const setPower = vi
      .spyOn(window.prism.gpu, "setPowerLimit")
      .mockResolvedValue({ success: true, error: null });
    renderPerformancePage();

    const problem = await screen.findByRole("alert", {
      name: "Tuning store can't be used",
    });
    expect(
      within(problem).getByText(
        /Prism couldn't read its tuning store\. It left the file untouched/,
      ),
    ).toBeInTheDocument();
    expect(
      within(problem).getByText(/Manual tuning still works/),
    ).toBeInTheDocument();
    expect(
      within(problem).getByText("/var/lib/prism/tuning-state.json"),
    ).toBeInTheDocument();
    expect(
      within(problem).getByText(
        "EOF while parsing an object at line 1 column 31",
      ),
    ).toBeInTheDocument();

    const power = screen.getByRole("spinbutton", { name: "Power limit" });
    await user.clear(power);
    await user.type(power, "450");
    await user.click(
      within(power.closest("section")!).getByRole("button", { name: "Apply" }),
    );
    await waitFor(() => expect(setPower).toHaveBeenCalledWith("GPU-123", 450));
  });

  it("explains a store written by a newer version of Prism", async () => {
    vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
      newerVersionStatus,
    );
    renderPerformancePage();

    const problem = await screen.findByRole("alert", {
      name: "Tuning store can't be used",
    });
    expect(
      within(problem).getByText(
        /Prism's tuning store was written by a newer version of Prism\. It left the file untouched/,
      ),
    ).toBeInTheDocument();
  });

  it("shows no problem while the store is healthy", async () => {
    vi.spyOn(window.prism.gpu, "getDaemonStatus").mockResolvedValue(
      connectedDaemonStatus,
    );
    renderPerformancePage();

    await screen.findByRole("spinbutton", { name: "Power limit" });
    await waitFor(() =>
      expect(window.prism.gpu.getDaemonStatus).toHaveBeenCalled(),
    );
    expect(
      screen.queryByRole("alert", { name: "Tuning store can't be used" }),
    ).not.toBeInTheDocument();
  });
});
