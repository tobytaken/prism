import { AlertTriangle, ArrowLeft, Cpu } from "lucide-react";
import { Link, useParams } from "react-router-dom";
import type { CSSProperties } from "react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { VendorBadge } from "@/features/gpus/components/vendor-badge";
import type { GpuMetricHistory } from "@/features/gpus/hooks/use-gpu-metric-history";
import { useGpuMetricHistory } from "@/features/gpus/hooks/use-gpu-metric-history";
import { useGpuPerformance } from "@/features/gpus/hooks/use-gpu-performance";
import {
  buildSparklinePath,
  calculateMetricStats,
  calculateSparklineBounds,
  findLastSparklinePoint,
} from "@/features/gpus/lib/performance-chart";
import {
  getPerformanceMetricDefinition,
  type PerformanceMetricDefinition,
} from "@/features/gpus/lib/performance-metrics";
import type { GpuPerformanceDevice } from "@/features/gpus/types";

const DETAIL_CHART_WIDTH = 720;
const DETAIL_CHART_PLOT_HEIGHT = 220;
const DETAIL_CHART_AXIS_HEIGHT = 30;
const DETAIL_CHART_HEIGHT = DETAIL_CHART_PLOT_HEIGHT + DETAIL_CHART_AXIS_HEIGHT;
const DETAIL_CHART_INSETS = {
  top: 12,
  right: 0,
  bottom: DETAIL_CHART_AXIS_HEIGHT,
  left: 48,
};
const DETAIL_SAMPLE_INTERVAL_SECONDS = 0.5;
const DETAIL_AXIS_TICK_SECONDS = 10;

export function GpuPerformanceMetricPage() {
  const { gpuId, metricKey } = useParams();
  const query = useGpuPerformance();
  const historyByGpuId = useGpuMetricHistory();
  const gpu = query.data?.gpus.find((candidate) => candidate.id === gpuId);

  return (
    <GpuPerformanceMetricScreen
      gpu={gpu}
      metricKey={metricKey}
      history={gpuId ? historyByGpuId[gpuId] : undefined}
      isLoading={query.isPending}
      errorMessage={query.error instanceof Error ? query.error.message : null}
    />
  );
}

type GpuPerformanceMetricScreenProps = {
  gpu?: GpuPerformanceDevice;
  metricKey?: string;
  history?: GpuMetricHistory;
  isLoading: boolean;
  errorMessage: string | null;
};

export function GpuPerformanceMetricScreen({
  gpu,
  metricKey,
  history,
  isLoading,
  errorMessage,
}: GpuPerformanceMetricScreenProps) {
  const metric = getPerformanceMetricDefinition(metricKey);

  if (isLoading) {
    return <GpuPerformanceMetricSkeleton />;
  }

  if (errorMessage) {
    return (
      <div className="space-y-6">
        <BackButton />
        <Alert variant="destructive">
          <AlertTriangle />
          <AlertTitle>Failed to load performance telemetry</AlertTitle>
          <AlertDescription>{errorMessage}</AlertDescription>
        </Alert>
      </div>
    );
  }

  if (!metric) {
    return (
      <MetricEmptyState
        title="Metric not found"
        message="The selected performance metric is not available."
      />
    );
  }

  if (!gpu) {
    return (
      <MetricEmptyState
        title="GPU not found"
        message="The selected device is no longer present in the current performance telemetry."
      />
    );
  }

  const series = metric.getHistory(history);
  const samples = series?.samples ?? [];
  const stats = calculateMetricStats(samples);
  const currentValue = metric.formatCurrentValue(metric.getValue(gpu));
  const style = {
    "--metric-accent": metric.accentColor,
  } as CSSProperties;

  return (
    <div className="space-y-6" style={style}>
      <BackButton />

      <section className="space-y-6">
        <div className="space-y-4">
          <div className="flex flex-wrap items-center gap-2">
            <VendorBadge vendor={gpu.vendor} />
            {gpu.architecture && (
              <Badge className="rounded-md border-border/70 bg-transparent" variant="outline">
                {gpu.architecture}
              </Badge>
            )}
            {gpu.busId && (
              <span className="font-mono text-xs text-muted-foreground">{gpu.busId}</span>
            )}
          </div>

          <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
            <div className="min-w-0 space-y-2">
              <p className="text-sm font-medium text-muted-foreground">{gpu.name}</p>
              <h1 className="text-3xl font-semibold tracking-tight">{metric.label}</h1>
              <p className="max-w-2xl text-sm text-muted-foreground">
                {metric.detailDescription}
              </p>
            </div>
            <div className="border-l border-border/80 pl-5">
              <p className="text-[11px] font-medium tracking-[0.22em] text-muted-foreground uppercase">
                Current
              </p>
              <p className="mt-1 text-4xl font-semibold tabular-nums tracking-tight">
                {currentValue}
              </p>
            </div>
          </div>
        </div>

        <div className="grid gap-4 md:grid-cols-3">
          <MetricStat label="Minimum" value={formatStat(stats?.min, metric)} />
          <MetricStat label="Maximum" value={formatStat(stats?.max, metric)} />
          <MetricStat label="Average" value={formatStat(stats?.average, metric, true)} />
        </div>

        <MetricDetailChart metric={metric} samples={samples} hasData={series?.hasData} />
      </section>
    </div>
  );
}

function BackButton() {
  return (
    <Button asChild variant="ghost" size="sm" className="w-fit">
      <Link to="/performance">
        <ArrowLeft />
        Back to Performance
      </Link>
    </Button>
  );
}

function MetricEmptyState({
  title,
  message,
}: {
  title: string;
  message: string;
}) {
  return (
    <div className="space-y-6">
      <BackButton />
      <div className="border-y border-dashed border-border/80 bg-muted/10 px-6 py-16 text-center">
        <Cpu className="mx-auto size-8 text-muted-foreground" />
        <p className="mt-4 text-sm font-medium">{title}</p>
        <p className="mt-1 text-sm text-muted-foreground">{message}</p>
      </div>
    </div>
  );
}

function MetricStat({ label, value }: { label: string; value: string }) {
  return (
    <div className="border-b border-border/70 pb-4">
      <p className="text-[11px] font-medium tracking-[0.22em] text-muted-foreground uppercase">
        {label}
      </p>
      <p className="mt-2 text-2xl font-semibold tabular-nums tracking-tight">{value}</p>
    </div>
  );
}

function MetricDetailChart({
  metric,
  samples,
  hasData,
}: {
  metric: PerformanceMetricDefinition;
  samples: Array<number | null>;
  hasData?: boolean;
}) {
  if (!hasData) {
    return (
      <div
        data-testid="gpu-performance-metric-chart-empty"
        className="flex aspect-[720/250] items-center justify-center rounded-md border border-dashed border-border/70 bg-muted/10 text-sm text-muted-foreground"
      >
        Unavailable
      </div>
    );
  }

  const bounds = calculateSparklineBounds(samples);
  if (!bounds) {
    return (
      <div
        data-testid="gpu-performance-metric-chart-empty"
        className="flex aspect-[720/250] items-center justify-center rounded-md border border-dashed border-border/70 bg-muted/10 text-sm text-muted-foreground"
      >
        Unavailable
      </div>
    );
  }

  const plottedSamples = samples.filter((value): value is number => value != null);
  const xTicks = buildXAxisTicks(plottedSamples.length);
  const yTicks = buildYAxisTicks();
  const path = buildSparklinePath(
    plottedSamples,
    DETAIL_CHART_WIDTH,
    DETAIL_CHART_HEIGHT,
    DETAIL_CHART_INSETS,
  );
  const lastPoint = findLastSparklinePoint(
    plottedSamples,
    DETAIL_CHART_WIDTH,
    DETAIL_CHART_HEIGHT,
    DETAIL_CHART_INSETS,
  );

  return (
    <div className="rounded-md border border-border/70 bg-muted/10 px-3 py-4">
      <div>
        <svg
          viewBox={`0 0 ${DETAIL_CHART_WIDTH} ${DETAIL_CHART_HEIGHT}`}
          className="aspect-[720/250] w-full overflow-visible"
          aria-label={`${metric.label} history chart`}
          role="img"
        >
          <text
            x="0"
            y={DETAIL_CHART_INSETS.top}
            dominantBaseline="hanging"
            className="fill-muted-foreground text-[9px] font-medium tabular-nums"
          >
            {metric.formatValue(bounds.max)}
          </text>
          <text
            x="0"
            y={DETAIL_CHART_PLOT_HEIGHT}
            dominantBaseline="alphabetic"
            className="fill-muted-foreground text-[9px] font-medium tabular-nums"
          >
            {metric.formatValue(bounds.min)}
          </text>
          {yTicks.map((tick) => (
            <path
              key={`y-${tick}`}
              d={`M ${DETAIL_CHART_INSETS.left} ${tick} H ${DETAIL_CHART_WIDTH - DETAIL_CHART_INSETS.right}`}
              stroke="color-mix(in srgb, var(--border) 48%, transparent)"
              strokeWidth="1"
            />
          ))}
          {xTicks.map((tick) => (
            <path
              key={`x-grid-${tick.seconds}`}
              d={`M ${tick.x} ${DETAIL_CHART_INSETS.top} V ${DETAIL_CHART_PLOT_HEIGHT}`}
              stroke="color-mix(in srgb, var(--border) 42%, transparent)"
              strokeWidth="1"
            />
          ))}
          <path
            d={`M ${DETAIL_CHART_INSETS.left} ${DETAIL_CHART_PLOT_HEIGHT} H ${DETAIL_CHART_WIDTH - DETAIL_CHART_INSETS.right}`}
            stroke="color-mix(in srgb, var(--border) 75%, transparent)"
            strokeWidth="1"
          />
          <path
            d={path}
            fill="none"
            stroke={metric.accentColor}
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
          {lastPoint && (
            <circle
              cx={lastPoint.x}
              cy={lastPoint.y}
              r="3.25"
              fill={metric.accentColor}
              stroke="var(--card)"
              strokeWidth="1.5"
            />
          )}
          {xTicks.map((tick) => (
            <text
              key={`x-label-${tick.seconds}`}
              x={tick.x}
              y={DETAIL_CHART_HEIGHT - 14}
              textAnchor={tick.textAnchor}
              className="fill-muted-foreground text-[9px] font-medium tabular-nums"
            >
              {tick.seconds}s
            </text>
          ))}
        </svg>
      </div>
    </div>
  );
}

function GpuPerformanceMetricSkeleton() {
  return (
    <div data-testid="gpu-performance-metric-skeleton" className="space-y-6">
      <Skeleton className="h-9 w-44 rounded-md" />
      <div className="space-y-4">
        <Skeleton className="h-5 w-52 rounded-md" />
        <Skeleton className="h-9 w-72 rounded-md" />
        <Skeleton className="h-5 w-full max-w-2xl rounded-md" />
      </div>
      <div className="grid gap-4 md:grid-cols-3">
        {Array.from({ length: 3 }).map((_, index) => (
          <Skeleton key={index} className="h-24 rounded-none" />
        ))}
      </div>
      <Skeleton className="h-72 rounded-none" />
    </div>
  );
}

function formatStat(
  value: number | undefined,
  metric: PerformanceMetricDefinition,
  shouldRound = false,
) {
  return value == null ? "Unavailable" : metric.formatValue(shouldRound ? Math.round(value) : value);
}

function buildXAxisTicks(sampleCount: number) {
  const durationSeconds = Math.max(
    Math.ceil(((sampleCount - 1) * DETAIL_SAMPLE_INTERVAL_SECONDS) / DETAIL_AXIS_TICK_SECONDS) *
      DETAIL_AXIS_TICK_SECONDS,
    0,
  );
  const horizontalSpan = Math.max(
    DETAIL_CHART_WIDTH - DETAIL_CHART_INSETS.left - DETAIL_CHART_INSETS.right,
    1,
  );

  return Array.from(
    { length: Math.floor(durationSeconds / DETAIL_AXIS_TICK_SECONDS) + 1 },
    (_, index) => {
      const seconds = index * DETAIL_AXIS_TICK_SECONDS;
      const ratio = durationSeconds === 0 ? 0 : seconds / durationSeconds;
      const x = DETAIL_CHART_INSETS.left + horizontalSpan * ratio;

      return {
        seconds,
        x,
        textAnchor:
          index === 0
            ? "start"
            : seconds === durationSeconds
              ? "end"
              : "middle",
      } as const;
    },
  );
}

function buildYAxisTicks() {
  const verticalSpan = Math.max(
    DETAIL_CHART_HEIGHT - DETAIL_CHART_INSETS.top - DETAIL_CHART_INSETS.bottom,
    1,
  );

  return Array.from({ length: 5 }, (_, index) => {
    const ratio = index / 4;
    return DETAIL_CHART_HEIGHT - DETAIL_CHART_INSETS.bottom - ratio * verticalSpan;
  });
}
