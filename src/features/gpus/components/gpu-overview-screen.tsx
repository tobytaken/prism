import { AlertTriangle, Cpu } from "lucide-react";
import { Link } from "react-router-dom";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { VendorBadge } from "@/features/gpus/components/vendor-badge";
import { formatNumber } from "@/features/gpus/lib/formatters";
import type { GpuDevice, GpuInventoryResponse } from "@/features/gpus/types";

type GpuOverviewScreenProps = {
  inventory?: GpuInventoryResponse;
  isLoading: boolean;
  errorMessage: string | null;
};

export function GpuOverviewScreen({
  inventory,
  isLoading,
  errorMessage,
}: GpuOverviewScreenProps) {
  const gpus = inventory?.gpus ?? [];
  const warnings = inventory?.warnings ?? [];

  return (
    <div className="space-y-6">
      <div className="space-y-1">
        <h1 className="text-2xl font-semibold tracking-tight">Overview</h1>
        <p className="text-sm text-muted-foreground">
          {gpus.length > 0
            ? `${gpus.length} GPU${gpus.length !== 1 ? "s" : ""} detected`
            : "Detected hardware"}
        </p>
      </div>

      {errorMessage && (
        <Alert variant="destructive">
          <AlertTriangle />
          <AlertTitle>Failed to load hardware inventory</AlertTitle>
          <AlertDescription>{errorMessage}</AlertDescription>
        </Alert>
      )}

      {warnings.length > 0 && (
        <Alert>
          <AlertTriangle className="text-primary" />
          <AlertTitle>Warnings</AlertTitle>
          <AlertDescription className="space-y-1">
            {warnings.map((warning) => (
              <p key={warning}>{warning}</p>
            ))}
          </AlertDescription>
        </Alert>
      )}

      {isLoading && <GpuOverviewSkeleton />}

      {!isLoading && gpus.length === 0 && !errorMessage && (
        <div className="rounded-xl border border-dashed border-border bg-card/50 px-6 py-16 text-center">
          <Cpu className="mx-auto size-8 text-muted-foreground" />
          <p className="mt-4 text-sm font-medium">No supported GPUs detected</p>
          <p className="mt-1 text-sm text-muted-foreground">
            Currently supports NVIDIA GPUs via NVML.
          </p>
        </div>
      )}

      {!isLoading && gpus.length > 0 && (
        <div className="space-y-3">
          {gpus.map((gpu) => (
            <GpuListItem key={gpu.id} gpu={gpu} />
          ))}
        </div>
      )}
    </div>
  );
}

function GpuListItem({ gpu }: { gpu: GpuDevice }) {
  return (
    <Link to={`/gpus/${encodeURIComponent(gpu.id)}`} className="block">
      <Card className="border-border/80 bg-card/70 py-0 transition-colors hover:bg-card">
        <CardContent className="py-4">
          <div className="flex flex-col gap-4 md:flex-row md:items-center md:justify-between">
            <div className="space-y-3">
              <div className="flex flex-wrap items-center gap-2">
                <VendorBadge vendor={gpu.vendor} />
                {gpu.architecture && <Badge variant="outline">{gpu.architecture}</Badge>}
                {gpu.busId && (
                  <span className="font-mono text-xs text-muted-foreground">{gpu.busId}</span>
                )}
              </div>

              <div className="space-y-1">
                <h2 className="text-lg font-semibold tracking-tight">{gpu.name}</h2>
                <p className="text-sm text-muted-foreground">
                  Click to open the hardware detail view for this GPU.
                </p>
              </div>
            </div>

            <dl className="grid grid-cols-2 gap-x-6 gap-y-3 text-sm sm:grid-cols-3">
              <GpuStat label="VRAM" value={formatNumber(gpu.memory.totalMiB, "MiB")} />
              <GpuStat label="Core Count" value={formatNumber(gpu.compute.coreCount)} />
              <GpuStat
                label="Power Limit"
                value={formatNumber(gpu.power.enforcedLimitWatts, "W")}
              />
            </dl>
          </div>
        </CardContent>
      </Card>
    </Link>
  );
}

function GpuStat({ label, value }: { label: string; value: string }) {
  return (
    <div className="space-y-1">
      <dt className="text-xs uppercase tracking-[0.18em] text-muted-foreground">{label}</dt>
      <dd className="font-medium tabular-nums">{value}</dd>
    </div>
  );
}

function GpuOverviewSkeleton() {
  return (
    <div
      data-testid="gpu-overview-skeleton"
      className="overflow-hidden rounded-xl border border-border/80 bg-card/70"
    >
      <div className="space-y-3 p-6">
        <Skeleton className="h-5 w-20 rounded-md" />
        <Skeleton className="h-7 w-64 rounded-md" />
      </div>
      <div className="space-y-2 px-6 pb-6">
        {Array.from({ length: 3 }).map((_, index) => (
          <Skeleton key={index} className="h-28 rounded-md" />
        ))}
      </div>
    </div>
  );
}
