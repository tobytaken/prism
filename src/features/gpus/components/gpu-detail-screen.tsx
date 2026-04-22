import { AlertTriangle, ArrowLeft, Cpu } from "lucide-react";
import { Link } from "react-router-dom";
import { toast } from "sonner";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { VendorBadge } from "@/features/gpus/components/vendor-badge";
import { Table, TableBody, TableCell, TableRow } from "@/components/ui/table";
import { formatNumber, formatText, formatVendor } from "@/features/gpus/lib/formatters";
import type { GpuDevice } from "@/features/gpus/types";

type GpuDetailScreenProps = {
  gpu?: GpuDevice;
  isLoading: boolean;
  errorMessage: string | null;
};

export function GpuDetailScreen({
  gpu,
  isLoading,
  errorMessage,
}: GpuDetailScreenProps) {
  if (isLoading) {
    return <GpuDetailSkeleton />;
  }

  if (errorMessage) {
    return (
      <div className="space-y-6">
        <BackButton />
        <Alert variant="destructive">
          <AlertTriangle />
          <AlertTitle>Failed to load hardware inventory</AlertTitle>
          <AlertDescription>{errorMessage}</AlertDescription>
        </Alert>
      </div>
    );
  }

  if (!gpu) {
    return (
      <div className="space-y-6">
        <BackButton />
        <div className="rounded-xl border border-dashed border-border bg-card/50 px-6 py-16 text-center">
          <Cpu className="mx-auto size-8 text-muted-foreground" />
          <p className="mt-4 text-sm font-medium">GPU not found</p>
          <p className="mt-1 text-sm text-muted-foreground">
            The selected device is no longer present in the current inventory.
          </p>
        </div>
      </div>
    );
  }

  const rows: Array<[label: string, value: string]> = [
    ["Vendor", formatVendor(gpu.vendor)],
    ["Architecture", formatText(gpu.architecture)],
    ["PCI Bus", formatText(gpu.busId)],
    ["Driver Version", formatText(gpu.driverVersion)],
    ["Firmware Version", formatText(gpu.firmwareVersion)],
    ["VRAM", formatNumber(gpu.memory.totalMiB, "MiB")],
    ["Compute API", formatText(gpu.compute.api?.toUpperCase() ?? null)],
    ["Compute API Version", formatText(gpu.compute.apiVersion)],
    ["Compute Capability", formatText(gpu.compute.capability)],
    ["Core Count", formatNumber(gpu.compute.coreCount)],
    ["PCIe Maximum", combinePcie(gpu)],
    ["Enforced Power Limit", formatNumber(gpu.power.enforcedLimitWatts, "W")],
    ["Default Power Limit", formatNumber(gpu.power.defaultLimitWatts, "W")],
    ["Current Graphics Clock", formatNumber(gpu.clocks.graphicsCurrentMHz, "MHz")],
    ["Maximum Graphics Clock", formatNumber(gpu.clocks.graphicsMaxMHz, "MHz")],
    ["Maximum SM Clock", formatNumber(gpu.clocks.smMaxMHz, "MHz")],
    ["Maximum Memory Clock", formatNumber(gpu.clocks.memoryMaxMHz, "MHz")],
  ];

  return (
    <div className="space-y-6">
      <BackButton />

      <section className="space-y-4">
        <div className="space-y-3">
          <div className="flex flex-wrap items-center gap-2">
            <VendorBadge vendor={gpu.vendor} />
            {gpu.architecture && <Badge variant="outline">{gpu.architecture}</Badge>}
            {gpu.busId && (
              <span className="font-mono text-xs text-muted-foreground">{gpu.busId}</span>
            )}
          </div>
          <div className="space-y-1">
            <h1 className="text-2xl font-semibold tracking-tight">{gpu.name}</h1>
            <p className="text-sm text-muted-foreground">
              Hardware inventory and supported platform details.
            </p>
          </div>
        </div>

        <div className="overflow-hidden rounded-xl border border-border/80 bg-card/70">
          <Table>
            <TableBody>
              {rows.map(([label, value]) => (
                <TableRow
                  key={label}
                  className="cursor-pointer"
                  onClick={() => {
                    navigator.clipboard.writeText(value);
                    toast.success("Copied to clipboard", {
                      description: value,
                    });
                  }}
                >
                  <TableCell className="w-56 text-muted-foreground">{label}</TableCell>
                  <TableCell className="font-medium tabular-nums">{value}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      </section>
    </div>
  );
}

function BackButton() {
  return (
    <Button asChild variant="ghost" size="sm" className="w-fit">
      <Link to="/">
        <ArrowLeft />
        Back to Overview
      </Link>
    </Button>
  );
}

function GpuDetailSkeleton() {
  return (
    <div
      data-testid="gpu-detail-skeleton"
      className="overflow-hidden rounded-xl border border-border/80 bg-card/70"
    >
      <div className="space-y-3 p-6">
        <Skeleton className="h-5 w-32 rounded-md" />
        <Skeleton className="h-7 w-64 rounded-md" />
      </div>
      <div className="space-y-2 px-6 pb-6">
        {Array.from({ length: 8 }).map((_, index) => (
          <Skeleton key={index} className="h-12 rounded-md" />
        ))}
      </div>
    </div>
  );
}

function combinePcie(gpu: GpuDevice) {
  if (gpu.pcie.genMax == null || gpu.pcie.widthMax == null) {
    return "Unavailable";
  }

  return `Gen ${gpu.pcie.genMax} x${gpu.pcie.widthMax}`;
}
