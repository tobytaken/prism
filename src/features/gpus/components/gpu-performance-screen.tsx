import { useEffect, useState, type CSSProperties } from "react";
import {
  AlertTriangle,
  ChevronDown,
  Cpu,
  Gauge,
  LoaderCircle,
  RefreshCw,
  ShieldAlert,
} from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { VendorBadge } from "@/features/gpus/components/vendor-badge";
import {
  type MetricHistorySeries,
  type GpuMetricHistory,
  useGpuMetricHistory,
} from "@/features/gpus/hooks/use-gpu-metric-history";
import { formatNumber } from "@/features/gpus/lib/formatters";
import { cn } from "@/lib/utils";
import {
  GpuTuningControls,
  type PerformanceMutationAction,
} from "./gpu-tuning-screen";
import type {
  GpuDaemonStatus,
  GpuPerformanceDevice,
  GpuPerformanceResponse,
} from "@/features/gpus/types";

const EMPTY_PERFORMANCE_GPUS: GpuPerformanceDevice[] = [];
const EMPTY_WARNINGS: string[] = [];
const SPARKLINE_WIDTH = 240;
const SPARKLINE_HEIGHT = 76.8;
const SPARKLINE_INSETS = {
  top: 5,
  right: 3,
  bottom: 5,
  left: 3,
};

type GpuPerformanceScreenProps = {
  inventory?: GpuPerformanceResponse;
  daemonStatus?: GpuDaemonStatus;
  isLoading: boolean;
  errorMessage: string | null;
  mutationErrorMessage?: string | null;
  reapplyPending?: boolean;
  reapplyErrorMessage?: string | null;
  pendingAction?: PerformanceMutationAction | null;
  onReapplySavedSettings?: () => void;
  onMutationAction?: (action: PerformanceMutationAction) => void;
};

export function GpuPerformanceScreen({
  inventory,
  daemonStatus,
  isLoading,
  errorMessage,
  mutationErrorMessage = null,
  reapplyPending = false,
  reapplyErrorMessage = null,
  pendingAction = null,
  onReapplySavedSettings,
  onMutationAction,
}: GpuPerformanceScreenProps) {
  const gpus = inventory?.gpus ?? EMPTY_PERFORMANCE_GPUS;
  const warnings = inventory?.warnings ?? EMPTY_WARNINGS;
  const historyByGpuId = useGpuMetricHistory();
  const daemonDisconnected = daemonStatus?.reachable === false;
  const daemonConnected = daemonStatus?.reachable === true;
  const daemonMessage =
    daemonStatus?.message ??
    "Privileged GPU controls require prismd to be installed and reachable.";

  return (
    <div className="space-y-8">
      <div className="flex flex-col gap-4 md:flex-row md:items-start md:justify-between">
        <div className="max-w-3xl space-y-2">
          <p className="text-xs font-medium tracking-[0.26em] text-primary uppercase">
            Signal Monitor
          </p>
          <h1 className="text-3xl font-semibold tracking-tight">Performance</h1>
        </div>
        {daemonConnected && onReapplySavedSettings && (
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={reapplyPending}
            onClick={onReapplySavedSettings}
          >
            {reapplyPending ? (
              <LoaderCircle className="animate-spin" />
            ) : (
              <RefreshCw />
            )}
            Reapply Saved Settings
          </Button>
        )}
      </div>

      {daemonDisconnected && daemonStatus && (
        <Alert
          variant="destructive"
          className="border-destructive/50 bg-destructive/12"
        >
          <ShieldAlert className="text-destructive" />
          <AlertTitle>Privileged GPU controls are unavailable</AlertTitle>
          <AlertDescription className="text-destructive/90">
            <div className="space-y-1">
              <p>{daemonMessage}</p>
              <p>
                Controls stay visible so you can inspect what the driver exposes
                on this system, but changes stay locked until prismd is
                reachable.
              </p>
              <p className="font-mono text-xs text-destructive/80">
                {daemonStatus.socketPath}
              </p>
            </div>
          </AlertDescription>
        </Alert>
      )}

      {errorMessage && (
        <Alert variant="destructive">
          <AlertTriangle />
          <AlertTitle>Failed to load performance telemetry</AlertTitle>
          <AlertDescription>{errorMessage}</AlertDescription>
        </Alert>
      )}
      {mutationErrorMessage && (
        <Alert variant="destructive">
          <AlertTriangle />
          <AlertTitle>Failed to apply change</AlertTitle>
          <AlertDescription>{mutationErrorMessage}</AlertDescription>
        </Alert>
      )}
      {reapplyErrorMessage && (
        <Alert variant="destructive">
          <AlertTriangle />
          <AlertTitle>Failed to reapply saved settings</AlertTitle>
          <AlertDescription>{reapplyErrorMessage}</AlertDescription>
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
        <div className="border-y border-dashed border-border/80 bg-muted/10 px-6 py-16 text-center">
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
          <GpuTelemetrySurface
            key={gpu.id}
            gpu={gpu}
            history={historyByGpuId[gpu.id]}
            controlsDisabled={daemonDisconnected}
            pendingAction={pendingAction}
            onMutationAction={onMutationAction}
          />
        ))}
    </div>
  );
}

function GpuTelemetrySurface({
  gpu,
  history,
  controlsDisabled,
  pendingAction,
  onMutationAction,
}: {
  gpu: GpuPerformanceDevice;
  history?: GpuMetricHistory;
  controlsDisabled: boolean;
  pendingAction: PerformanceMutationAction | null;
  onMutationAction?: (action: PerformanceMutationAction) => void;
}) {
  const headlineMetrics = [
    {
      label: "Power",
      value: formatNumber(gpu.telemetry.powerDrawWatts, "W"),
      color: "var(--color-chart-1)",
    },
    {
      label: "GPU",
      value: formatCompactPercent(gpu.telemetry.gpuUtilizationPercent),
      color: "var(--primary)",
    },
    {
      label: "Temp",
      value: formatCompactCelsius(gpu.telemetry.temperatureC),
      color: "var(--color-chart-5)",
    },
  ];

  return (
    <details
      className="group/gpu overflow-hidden rounded-lg border border-border/80 bg-card/55 shadow-sm shadow-black/10"
      open
    >
      <summary className="flex cursor-pointer list-none items-center justify-between gap-4 border-b border-border/70 px-5 py-4 transition-colors hover:bg-muted/25 [&::-webkit-details-marker]:hidden">
        <div className="min-w-0 space-y-2">
          <div className="flex flex-wrap items-center gap-2">
            <VendorBadge vendor={gpu.vendor} />
            {gpu.architecture && (
              <Badge
                className="rounded-md border-border/70 bg-transparent"
                variant="outline"
              >
                {gpu.architecture}
              </Badge>
            )}
          </div>
          <div className="flex min-w-0 flex-wrap items-baseline gap-x-3 gap-y-1">
            <h2 className="truncate text-lg font-semibold tracking-tight">
              {gpu.name}
            </h2>
            {gpu.busId && (
              <span className="font-mono text-xs text-muted-foreground">
                {gpu.busId}
              </span>
            )}
          </div>
        </div>

        <div className="flex shrink-0 items-center gap-4">
          <div className="hidden items-center divide-x divide-border/70 border-x border-border/70 md:flex">
            {headlineMetrics.map((metric) => (
              <div key={metric.label} className="px-4 py-1.5">
                <p className="text-[10px] font-medium tracking-[0.18em] text-muted-foreground uppercase">
                  {metric.label}
                </p>
                <p className="mt-0.5 text-sm font-semibold tabular-nums">
                  {metric.value}
                </p>
              </div>
            ))}
          </div>
          <ChevronDown className="size-4 text-muted-foreground transition-transform group-open/gpu:rotate-180" />
        </div>
      </summary>

      <section className="relative overflow-hidden px-5 py-7">
        <div className="pointer-events-none absolute inset-x-0 top-0 h-px bg-linear-to-r from-transparent via-primary/70 to-transparent" />
        <div className="grid gap-8 xl:grid-cols-[minmax(340px,0.9fr)_minmax(680px,1.8fr)]">
          <OcControlRail
            gpu={gpu}
            controlsDisabled={controlsDisabled}
            pendingAction={pendingAction}
            onMutationAction={onMutationAction}
          />

          <div
            data-testid="gpu-performance-telemetry"
            className="min-w-0 space-y-7"
          >
            <TelemetryPanel gpu={gpu} history={history} />
          </div>
        </div>
      </section>
    </details>
  );
}

function OcControlRail({
  gpu,
  controlsDisabled,
  pendingAction,
  onMutationAction,
}: {
  gpu: GpuPerformanceDevice;
  controlsDisabled: boolean;
  pendingAction: PerformanceMutationAction | null;
  onMutationAction?: (action: PerformanceMutationAction) => void;
}) {
  const [open, setOpen] = useState(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
      return true;
    }

    return window.matchMedia("(min-width: 1280px)").matches;
  });

  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
      return;
    }

    const query = window.matchMedia("(min-width: 1280px)");
    const handleChange = () => setOpen(query.matches);
    query.addEventListener("change", handleChange);

    return () => query.removeEventListener("change", handleChange);
  }, []);

  return (
    <details
      open={open}
      onToggle={(event) => setOpen(event.currentTarget.open)}
      className="group/oc min-w-0 border-y border-border/70"
    >
      <summary className="flex cursor-pointer list-none items-center justify-between gap-3 py-3 transition-colors hover:text-foreground [&::-webkit-details-marker]:hidden">
        <div className="flex items-center gap-2">
          <Gauge className="size-4 text-primary" />
          <span className="text-sm font-semibold tracking-[0.18em] uppercase">
            OC Controls
          </span>
        </div>
        <ChevronDown className="size-4 text-muted-foreground transition-transform group-open/oc:rotate-180" />
      </summary>

      <fieldset
        disabled={controlsDisabled || !onMutationAction}
        aria-disabled={controlsDisabled || !onMutationAction}
        className={cn(
          "pb-5 transition-opacity",
          (controlsDisabled || !onMutationAction) &&
            "pointer-events-none opacity-45 saturate-50",
        )}
      >
        <GpuTuningControls
          gpu={gpu}
          pendingAction={pendingAction}
          onMutationAction={onMutationAction ?? (() => undefined)}
        />
      </fieldset>
    </details>
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
      <div className="grid gap-x-7 gap-y-6 md:grid-cols-2 2xl:grid-cols-3">
        <TelemetryMetricCard
          label="Power"
          value={formatNumber(gpu.telemetry.powerDrawWatts, "W")}
          subtitle="Board power draw"
          history={history?.powerDrawWatts}
          accentColor="var(--color-chart-1)"
          formatHistoryValue={formatWatts}
        />
        <TelemetryMetricCard
          label="Core Clock"
          value={formatNumber(gpu.telemetry.coreClockMHz, "MHz")}
          subtitle="Graphics domain clock"
          history={history?.coreClockMHz}
          accentColor="var(--color-chart-2)"
          formatHistoryValue={formatMegahertz}
        />
        <TelemetryMetricCard
          label="Memory Clock"
          value={formatNumber(gpu.telemetry.memoryClockMHz, "MHz")}
          subtitle="Current memory clock"
          history={history?.memoryClockMHz}
          accentColor="var(--color-chart-3)"
          formatHistoryValue={formatMegahertz}
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
          formatHistoryValue={formatPercentValue}
        />
        <TelemetryMetricCard
          label="Temperature"
          value={formatNumber(gpu.telemetry.temperatureC, "°C")}
          subtitle="GPU die temperature"
          history={history?.temperatureC}
          accentColor="var(--color-chart-5)"
          formatHistoryValue={formatCelsius}
        />
        <TelemetryMetricCard
          label="GPU Utilization"
          value={formatPercent(gpu.telemetry.gpuUtilizationPercent)}
          subtitle="Active compute time"
          history={history?.gpuUtilizationPercent}
          accentColor="var(--primary)"
          formatHistoryValue={formatPercentValue}
        />
        <TelemetryMetricCard
          label="VRAM Used"
          value={formatNumber(gpu.telemetry.memoryUsedMiB, "MiB")}
          subtitle={totalMemoryLabel}
          history={history?.memoryUsedMiB}
          accentColor="var(--color-chart-2)"
          formatHistoryValue={formatMebibytes}
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
  );
}

function TelemetryMetricCard({
  label,
  value,
  subtitle,
  history,
  accentColor,
  formatHistoryValue,
}: {
  label: string;
  value: string;
  subtitle: string;
  history: MetricHistorySeries | undefined;
  accentColor: string;
  formatHistoryValue: (value: number) => string;
}) {
  const style = {
    "--telemetry-accent": accentColor,
  } as CSSProperties;

  return (
    <div
      style={style}
      className="group min-w-0 border-b border-border/70 pb-5 transition-colors hover:border-foreground/35"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="text-[11px] font-medium tracking-[0.22em] text-muted-foreground uppercase">
            {label}
          </p>
          <p className="mt-2 text-2xl font-semibold tabular-nums tracking-tight">
            {value}
          </p>
        </div>
        <span
          className="mt-1 block h-8 w-px opacity-80 transition-opacity group-hover:opacity-100"
          style={{ backgroundColor: accentColor }}
        />
      </div>
      <p className="mt-1 text-xs text-muted-foreground">{subtitle}</p>
      <div className="mt-4">
        <MetricSparkline
          series={history}
          accentColor={accentColor}
          formatValue={formatHistoryValue}
        />
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
    <div className="min-w-0 border-b border-border/70 pb-5">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="text-[11px] font-medium tracking-[0.22em] text-muted-foreground uppercase">
            {label}
          </p>
          <p className="mt-2 text-2xl font-semibold tabular-nums tracking-tight">
            {value}
          </p>
        </div>
        {badge && (
          <Badge className="rounded-md border-border/70 bg-transparent" variant="outline">
            {badge}
          </Badge>
        )}
      </div>
      <p className="mt-4 text-xs text-muted-foreground">{subtitle}</p>
    </div>
  );
}

function MetricSparkline({
  series,
  accentColor,
  formatValue,
}: {
  series: MetricHistorySeries | undefined;
  accentColor: string;
  formatValue: (value: number) => string;
}) {
  if (!series?.hasData) {
    return (
      <div
        className="flex items-center justify-center border border-dashed border-border/70 bg-muted/10 text-xs text-muted-foreground"
        style={{ height: SPARKLINE_HEIGHT }}
      >
        Unavailable
      </div>
    );
  }

  const bounds = calculateSparklineBounds(series.samples);
  if (!bounds) {
    return (
      <div
        className="flex items-center justify-center border border-dashed border-border/70 bg-muted/10 text-xs text-muted-foreground"
        style={{ height: SPARKLINE_HEIGHT }}
      >
        Unavailable
      </div>
    );
  }

  const path = buildSparklinePath(
    series.samples,
    SPARKLINE_WIDTH,
    SPARKLINE_HEIGHT,
    SPARKLINE_INSETS,
  );
  const lastPoint = findLastSparklinePoint(
    series.samples,
    SPARKLINE_WIDTH,
    SPARKLINE_HEIGHT,
    SPARKLINE_INSETS,
  );

  return (
    <div className="border border-border/70 bg-muted/10 px-1.5 py-2">
      <div className="flex items-stretch gap-2">
        <div
          className="flex shrink-0 flex-col items-end justify-between text-right text-[11px] leading-none font-medium tabular-nums text-muted-foreground"
          style={{ height: SPARKLINE_HEIGHT }}
        >
          <span className="whitespace-nowrap">{formatValue(bounds.max)}</span>
          <span className="whitespace-nowrap">{formatValue(bounds.min)}</span>
        </div>
        <svg
          viewBox={`0 0 ${SPARKLINE_WIDTH} ${SPARKLINE_HEIGHT}`}
          className="min-w-0 flex-1"
          style={{ height: SPARKLINE_HEIGHT }}
          aria-hidden="true"
        >
          <path
            d={`M ${SPARKLINE_INSETS.left} ${SPARKLINE_HEIGHT - SPARKLINE_INSETS.bottom} H ${SPARKLINE_WIDTH - SPARKLINE_INSETS.right}`}
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
    </div>
  );
}

function GpuPerformanceSkeleton() {
  return (
    <div data-testid="gpu-performance-skeleton" className="border-y border-border/80 py-7">
      <div className="grid gap-8 xl:grid-cols-[minmax(340px,0.9fr)_minmax(680px,1.8fr)]">
        <div className="space-y-6">
          <Skeleton className="h-11 w-full rounded-none" />
          <div className="space-y-5">
            {Array.from({ length: 3 }).map((_, index) => (
              <Skeleton key={index} className="h-48 rounded-none" />
            ))}
          </div>
        </div>
        <div className="space-y-7">
          <Skeleton className="h-10 w-full rounded-none" />
          <div className="grid gap-x-7 gap-y-6 md:grid-cols-2 2xl:grid-cols-3">
            {Array.from({ length: 9 }).map((_, index) => (
              <Skeleton key={index} className="h-36 rounded-none" />
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

function formatPercent(value: number | null) {
  return value == null ? "Unavailable" : `${value}%`;
}

function formatCompactPercent(value: number | null) {
  return value == null ? "N/A" : `${value} %`;
}

function formatCompactCelsius(value: number | null) {
  return value == null ? "N/A" : `${value}C`;
}

function formatWatts(value: number) {
  return formatNumber(value, "W");
}

function formatMegahertz(value: number) {
  return formatNumber(value, "MHz");
}

function formatCelsius(value: number) {
  return formatNumber(value, "°C");
}

function formatMebibytes(value: number) {
  return formatNumber(value, "MiB");
}

function formatPercentValue(value: number) {
  return `${value}%`;
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
  insets: SparklineInsets,
) {
  const points = calculateSparklinePoints(samples, width, height, insets);
  if (points.length === 0) {
    return `M ${insets.left} ${height - insets.bottom}`;
  }

  return points
    .map((point, index) => `${index === 0 ? "M" : "L"} ${point.x} ${point.y}`)
    .join(" ");
}

function findLastSparklinePoint(
  samples: Array<number | null>,
  width: number,
  height: number,
  insets: SparklineInsets,
) {
  const points = calculateSparklinePoints(samples, width, height, insets);
  return points.at(-1);
}

type SparklineInsets = {
  top: number;
  right: number;
  bottom: number;
  left: number;
};

function calculateSparklinePoints(
  samples: Array<number | null>,
  width: number,
  height: number,
  insets: SparklineInsets,
) {
  const bounds = calculateSparklineBounds(samples);
  if (!bounds) {
    return [];
  }

  const range = bounds.max - bounds.min || 1;
  const horizontalSpan = Math.max(width - insets.left - insets.right, 1);
  const verticalSpan = Math.max(height - insets.top - insets.bottom, 1);

  return samples.flatMap((value, index) => {
    if (value == null) {
      return [];
    }

    const x =
      insets.left +
      (samples.length === 1 ? 0 : (horizontalSpan * index) / (samples.length - 1));
    const normalized = (value - bounds.min) / range;
    const y = height - insets.bottom - normalized * verticalSpan;

    return [{ x, y }];
  });
}

function calculateSparklineBounds(samples: Array<number | null>) {
  const values = samples.filter((value): value is number => value != null);
  if (values.length === 0) {
    return null;
  }

  return {
    min: Math.min(...values),
    max: Math.max(...values),
  };
}
