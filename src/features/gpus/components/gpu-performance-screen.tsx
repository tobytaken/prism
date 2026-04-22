import { type CSSProperties } from "react";
import { AlertTriangle, Cpu } from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { VendorBadge } from "@/features/gpus/components/vendor-badge";
import {
  type MetricHistorySeries,
  type GpuMetricHistory,
  useGpuMetricHistory,
} from "@/features/gpus/hooks/use-gpu-metric-history";
import { formatNumber } from "@/features/gpus/lib/formatters";
import type {
  GpuPerformanceDevice,
  GpuPerformanceResponse,
} from "@/features/gpus/types";

const EMPTY_PERFORMANCE_GPUS: GpuPerformanceDevice[] = [];
const EMPTY_WARNINGS: string[] = [];

type GpuPerformanceScreenProps = {
  inventory?: GpuPerformanceResponse;
  isLoading: boolean;
  errorMessage: string | null;
};

export function GpuPerformanceScreen({
  inventory,
  isLoading,
  errorMessage,
}: GpuPerformanceScreenProps) {
  const gpus = inventory?.gpus ?? EMPTY_PERFORMANCE_GPUS;
  const warnings = inventory?.warnings ?? EMPTY_WARNINGS;
  const historyByGpuId = useGpuMetricHistory(gpus);

  return (
    <div className="space-y-6">
      <div className="flex flex-col gap-4 md:flex-row md:items-start md:justify-between">
        <h1 className="text-2xl font-semibold tracking-tight">Performance</h1>
      </div>

      {errorMessage && (
        <Alert variant="destructive">
          <AlertTriangle />
          <AlertTitle>Failed to load performance telemetry</AlertTitle>
          <AlertDescription>{errorMessage}</AlertDescription>
        </Alert>
      )}

      {warnings.length > 0 && (
        <Alert>
          <AlertTriangle className="text-primary" />
          <AlertTitle>Driver warnings</AlertTitle>
          <AlertDescription className="space-y-1">
            {warnings.map((warning) => (
              <p key={warning}>{warning}</p>
            ))}
          </AlertDescription>
        </Alert>
      )}

      {isLoading && <GpuPerformanceSkeleton />}

      {!isLoading && gpus.length === 0 && !errorMessage && (
        <div className="rounded-xl border border-dashed border-border bg-card/50 px-6 py-16 text-center">
          <Cpu className="mx-auto size-8 text-muted-foreground" />
          <p className="mt-4 text-sm font-medium">No supported GPUs detected</p>
          <p className="mt-1 text-sm text-muted-foreground">
            Performance telemetry is currently available for NVIDIA GPUs exposed
            through NVML.
          </p>
        </div>
      )}

      {!isLoading &&
        gpus.length > 0 &&
        gpus.map((gpu) => (
          <GpuTelemetryCard
            key={gpu.id}
            gpu={gpu}
            history={historyByGpuId[gpu.id]}
          />
        ))}
    </div>
  );
}

function GpuTelemetryCard({
  gpu,
  history,
}: {
  gpu: GpuPerformanceDevice;
  history?: GpuMetricHistory;
}) {
  return (
    <Card className="border-border/80 bg-card/80 py-0">
      <CardHeader className="border-b border-border/80 py-5">
        <div className="flex flex-wrap items-center gap-2">
          <VendorBadge vendor={gpu.vendor} />
          {gpu.architecture && <Badge variant="outline">{gpu.architecture}</Badge>}
          {gpu.busId && (
            <span className="font-mono text-xs text-muted-foreground">
              {gpu.busId}
            </span>
          )}
        </div>
        <div className="space-y-1">
          <CardTitle>{gpu.name}</CardTitle>
          <p className="text-sm text-muted-foreground">
            Live telemetry updates every 500 ms with a rolling 60 second view.
          </p>
        </div>
      </CardHeader>

      <CardContent className="py-6">
        <TelemetryPanel gpu={gpu} history={history} />
      </CardContent>
    </Card>
  );
}

function TelemetryPanel({
  gpu,
  history,
}: {
  gpu: GpuPerformanceDevice;
  history?: GpuMetricHistory;
}) {
  const totalMemoryLabel =
    gpu.telemetry.memoryTotalMiB != null
      ? `${formatNumber(gpu.telemetry.memoryTotalMiB, "MiB")} total`
      : "Frame buffer usage";

  return (
    <section
      data-testid="gpu-performance-telemetry"
      className="space-y-4 rounded-xl border border-border bg-background/45 p-4"
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="space-y-1">
          <CardTitle className="text-sm uppercase tracking-[0.18em] text-foreground/85">
            Live Telemetry
          </CardTitle>
          <p className="text-sm text-muted-foreground">
            Compact history for the metrics that matter while tuning this GPU.
          </p>
        </div>
        <Badge variant="outline">60 s</Badge>
      </div>

      <div className="grid gap-3 md:grid-cols-2 2xl:grid-cols-3">
        <TelemetryMetricCard
          label="Power"
          value={formatNumber(gpu.telemetry.powerDrawWatts, "W")}
          subtitle="Board power draw"
          history={history?.powerDrawWatts}
          accentColor="var(--color-chart-1)"
        />
        <TelemetryMetricCard
          label="Core Clock"
          value={formatNumber(gpu.telemetry.coreClockMHz, "MHz")}
          subtitle="Graphics domain clock"
          history={history?.coreClockMHz}
          accentColor="var(--color-chart-2)"
        />
        <TelemetryMetricCard
          label="Memory Clock"
          value={formatNumber(gpu.telemetry.memoryClockMHz, "MHz")}
          subtitle="Current memory clock"
          history={history?.memoryClockMHz}
          accentColor="var(--color-chart-3)"
        />
        <TelemetryMetricCard
          label="Fan Speed"
          value={formatPercent(gpu.telemetry.fanSpeedPercent)}
          subtitle={
            gpu.cooling.fanCount > 1
              ? `${gpu.cooling.fanCount} fans averaged`
              : "Current programmable fan speed"
          }
          history={history?.fanSpeedPercent}
          accentColor="var(--color-chart-4)"
        />
        <TelemetryMetricCard
          label="Temperature"
          value={formatNumber(gpu.telemetry.temperatureC, "°C")}
          subtitle="GPU die temperature"
          history={history?.temperatureC}
          accentColor="var(--color-chart-5)"
        />
        <TelemetryMetricCard
          label="GPU Utilization"
          value={formatPercent(gpu.telemetry.gpuUtilizationPercent)}
          subtitle="Active compute time"
          history={history?.gpuUtilizationPercent}
          accentColor="var(--primary)"
        />
        <TelemetryMetricCard
          label="VRAM Used"
          value={formatNumber(gpu.telemetry.memoryUsedMiB, "MiB")}
          subtitle={totalMemoryLabel}
          history={history?.memoryUsedMiB}
          accentColor="var(--color-chart-2)"
        />
        <TelemetryStatusCard
          label="Voltage"
          value={formatVoltage(gpu.telemetry.voltageMillivolts)}
          subtitle="Best-effort driver reading"
        />
        <TelemetryStatusCard
          label="Performance State"
          value={gpu.telemetry.performanceState ?? "Unavailable"}
          subtitle="P0 is the highest performance state"
          badge={gpu.telemetry.performanceState ?? undefined}
        />
      </div>
    </section>
  );
}

function TelemetryMetricCard({
  label,
  value,
  subtitle,
  history,
  accentColor,
}: {
  label: string;
  value: string;
  subtitle: string;
  history: MetricHistorySeries | undefined;
  accentColor: string;
}) {
  const style = {
    "--telemetry-accent": accentColor,
  } as CSSProperties;

  return (
    <div
      style={style}
      className="rounded-xl border border-border bg-card/65 p-4 shadow-sm shadow-black/10"
    >
      <div className="flex items-start justify-between gap-3">
        <div>
          <p className="text-xs uppercase tracking-[0.18em] text-muted-foreground">
            {label}
          </p>
          <p className="mt-2 text-xl font-semibold tabular-nums tracking-tight">
            {value}
          </p>
        </div>
        <span
          className="mt-1 block size-2 rounded-full"
          style={{ backgroundColor: accentColor }}
        />
      </div>
      <p className="mt-1 text-xs text-muted-foreground">{subtitle}</p>
      <div className="mt-4">
        <MetricSparkline series={history} accentColor={accentColor} />
      </div>
    </div>
  );
}

function TelemetryStatusCard({
  label,
  value,
  subtitle,
  badge,
}: {
  label: string;
  value: string;
  subtitle: string;
  badge?: string;
}) {
  return (
    <div className="rounded-xl border border-border bg-card/65 p-4 shadow-sm shadow-black/10">
      <div className="flex items-start justify-between gap-3">
        <div>
          <p className="text-xs uppercase tracking-[0.18em] text-muted-foreground">
            {label}
          </p>
          <p className="mt-2 text-xl font-semibold tabular-nums tracking-tight">
            {value}
          </p>
        </div>
        {badge && <Badge variant="outline">{badge}</Badge>}
      </div>
      <p className="mt-4 text-xs text-muted-foreground">{subtitle}</p>
    </div>
  );
}

function MetricSparkline({
  series,
  accentColor,
}: {
  series: MetricHistorySeries | undefined;
  accentColor: string;
}) {
  if (!series?.hasData) {
    return (
      <div className="flex h-16 items-center justify-center rounded-lg border border-dashed border-border/70 bg-background/35 text-xs text-muted-foreground">
        Unavailable
      </div>
    );
  }

  const width = 240;
  const height = 64;
  const padding = 6;
  const path = buildSparklinePath(series.samples, width, height, padding);
  const lastPoint = findLastSparklinePoint(series.samples, width, height, padding);

  return (
    <div className="rounded-lg border border-border/70 bg-background/35 px-2 py-2">
      <svg
        viewBox={`0 0 ${width} ${height}`}
        className="h-16 w-full"
        aria-hidden="true"
      >
        <path
          d={`M ${padding} ${height - padding} H ${width - padding}`}
          stroke="color-mix(in srgb, var(--border) 75%, transparent)"
          strokeWidth="1"
        />
        <path
          d={path}
          fill="none"
          stroke={accentColor}
          strokeWidth="2.25"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
        {lastPoint && (
          <circle
            cx={lastPoint.x}
            cy={lastPoint.y}
            r="3"
            fill={accentColor}
            stroke="var(--card)"
            strokeWidth="1.5"
          />
        )}
      </svg>
    </div>
  );
}

function GpuPerformanceSkeleton() {
  return (
    <div
      data-testid="gpu-performance-skeleton"
      className="overflow-hidden rounded-xl border border-border/80 bg-card/80"
    >
      <div className="space-y-3 p-6">
        <Skeleton className="h-5 w-24 rounded-md" />
        <Skeleton className="h-8 w-80 rounded-md" />
        <Skeleton className="h-4 w-80 rounded-md" />
      </div>
      <Separator />
      <div className="space-y-4 p-6">
        <Skeleton className="h-5 w-32 rounded-md" />
        <div className="grid gap-3 md:grid-cols-2 2xl:grid-cols-3">
          {Array.from({ length: 9 }).map((_, index) => (
            <Skeleton key={index} className="h-40 rounded-xl" />
          ))}
        </div>
      </div>
    </div>
  );
}

function formatPercent(value: number | null) {
  return value == null ? "Unavailable" : `${value}%`;
}

function formatVoltage(value: number | null) {
  if (value == null) {
    return "Unavailable";
  }

  return `${new Intl.NumberFormat("en-US", {
    maximumFractionDigits: 3,
  }).format(value / 1000)} V`;
}

function buildSparklinePath(
  samples: Array<number | null>,
  width: number,
  height: number,
  padding: number,
) {
  const points = calculateSparklinePoints(samples, width, height, padding);
  if (points.length === 0) {
    return `M ${padding} ${height - padding}`;
  }

  return points
    .map((point, index) => `${index === 0 ? "M" : "L"} ${point.x} ${point.y}`)
    .join(" ");
}

function findLastSparklinePoint(
  samples: Array<number | null>,
  width: number,
  height: number,
  padding: number,
) {
  const points = calculateSparklinePoints(samples, width, height, padding);
  return points.at(-1);
}

function calculateSparklinePoints(
  samples: Array<number | null>,
  width: number,
  height: number,
  padding: number,
) {
  const values = samples.filter((value): value is number => value != null);
  if (values.length === 0) {
    return [];
  }

  const min = Math.min(...values);
  const max = Math.max(...values);
  const range = max - min || 1;
  const horizontalSpan = Math.max(width - padding * 2, 1);
  const verticalSpan = Math.max(height - padding * 2, 1);

  return samples.flatMap((value, index) => {
    if (value == null) {
      return [];
    }

    const x =
      padding +
      (samples.length === 1 ? 0 : (horizontalSpan * index) / (samples.length - 1));
    const normalized = (value - min) / range;
    const y = height - padding - normalized * verticalSpan;

    return [{ x, y }];
  });
}
