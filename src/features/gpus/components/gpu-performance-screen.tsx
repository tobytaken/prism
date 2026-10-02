import { UnavailableGpuEdits } from "./unavailable-gpu-edits";
import { GpuProfilePanel, GpuProfileAvailability } from "./gpu-profile-panel";
import type { GpuProfileWorkflow } from "@/features/gpus/hooks/use-gpu-profiles";
import {
  useTuningEdits,
  type TuningEditStore,
} from "@/features/gpus/hooks/use-tuning-edits";
import { unavailableTuningReadback } from "@/features/gpus/lib/tuning-readback";
import type { TuningMutationHandler } from "@/features/gpus/lib/tuning-mutation";
import { useEffect, useState, type CSSProperties } from "react";
import {
  AlertTriangle,
  ArrowUpRight,
  ChevronDown,
  Cpu,
  Gauge,
  ShieldAlert,
} from "lucide-react";
import { Link } from "react-router-dom";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { VendorBadge } from "@/features/gpus/components/vendor-badge";
import {
  type MetricHistorySeries,
  type GpuMetricHistory,
  useGpuMetricHistory,
} from "@/features/gpus/hooks/use-gpu-metric-history";
import { formatNumber } from "@/features/gpus/lib/formatters";
import {
  buildSparklinePath,
  calculateSparklineBounds,
  findLastSparklinePoint,
} from "@/features/gpus/lib/performance-chart";
import {
  PERFORMANCE_METRICS,
  type PerformanceMetricDefinition,
} from "@/features/gpus/lib/performance-metrics";
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
const OVERVIEW_SPARKLINE_SAMPLE_LIMIT = 120;
const SPARKLINE_INSETS = {
  top: 5,
  right: 3,
  bottom: 5,
  left: 3,
};

type GpuPerformanceScreenProps = {
  profiles?: GpuProfileWorkflow;
  inventory?: GpuPerformanceResponse;
  daemonStatus?: GpuDaemonStatus;
  isLoading: boolean;
  errorMessage: string | null;
  mutationErrorMessage?: string | null;
  pendingAction?: PerformanceMutationAction | null;
  onMutationAction?: TuningMutationHandler;
};

export function GpuPerformanceScreen({
  profiles,
  inventory,
  daemonStatus,
  isLoading,
  errorMessage,
  mutationErrorMessage = null,
  pendingAction = null,
  onMutationAction,
}: GpuPerformanceScreenProps) {
  const editStore = useTuningEdits();
  const gpus = inventory?.gpus ?? EMPTY_PERFORMANCE_GPUS;
  const warnings = inventory?.warnings ?? EMPTY_WARNINGS;
  const historyByGpuId = useGpuMetricHistory();
  const daemonDisconnected = daemonStatus?.reachable === false;
  const daemonMessage =
    daemonStatus?.message ??
    "Privileged GPU controls require prismd to be installed and reachable.";

  return (
    <div className="space-y-8">
      <div className="flex flex-col gap-4 md:flex-row md:items-start md:justify-between">
        <div className="max-w-3xl space-y-2">
          <h1 className="text-3xl font-semibold tracking-tight">Performance</h1>
        </div>
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

      <UnavailableGpuEdits
        devices={editStore.devices}
        gpus={gpus}
        onDiscard={editStore.discard}
        pending={pendingAction != null}
      />
      {profiles && <GpuProfileAvailability workflow={profiles} gpus={gpus} />}

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
            gpu={
              errorMessage ? unavailableTuningReadback(gpu, errorMessage) : gpu
            }
            editStore={editStore}
            profiles={profiles}
            history={historyByGpuId[gpu.id]}
            controlsDisabled={daemonDisconnected}
            profileLoadingDisabled={
              daemonStatus?.reachable !== true || pendingAction != null
            }
            readError={errorMessage}
            pendingAction={pendingAction}
            onMutationAction={onMutationAction}
          />
        ))}
    </div>
  );
}

function GpuTelemetrySurface({
  gpu,
  profiles,
  editStore,
  history,
  controlsDisabled,
  profileLoadingDisabled,
  readError,
  pendingAction,
  onMutationAction,
}: {
  gpu: GpuPerformanceDevice;
  profiles?: GpuProfileWorkflow;
  editStore: TuningEditStore;
  history?: GpuMetricHistory;
  controlsDisabled: boolean;
  profileLoadingDisabled: boolean;
  readError: string | null;
  pendingAction: PerformanceMutationAction | null;
  onMutationAction?: TuningMutationHandler;
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
        {profiles && (
          <div className="mb-7">
            <GpuProfilePanel
              gpu={gpu}
              workflow={profiles}
              edits={editStore.devices[gpu.id]?.edits ?? {}}
              readError={readError}
              pendingAction={pendingAction}
              loadDisabled={profileLoadingDisabled}
              onLoad={
                onMutationAction
                  ? (profile, settings) => {
                      void editStore.apply(
                        {
                          kind: "applyProfile",
                          gpuId: gpu.id,
                          stableId: profile.gpuId,
                          settings,
                        },
                        gpu,
                        onMutationAction,
                      );
                    }
                  : undefined
              }
            />
          </div>
        )}
        <div className="flex flex-col gap-8 xl:flex-row">
          <OcControlRail
            gpu={gpu}
            editStore={editStore}
            controlsDisabled={controlsDisabled}
            pendingAction={pendingAction}
            onMutationAction={onMutationAction}
          />

          <div
            data-testid="gpu-performance-telemetry"
            className="min-w-0 flex-1 space-y-7"
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
  editStore,
  controlsDisabled,
  pendingAction,
  onMutationAction,
}: {
  gpu: GpuPerformanceDevice;
  editStore: TuningEditStore;
  controlsDisabled: boolean;
  pendingAction: PerformanceMutationAction | null;
  onMutationAction?: TuningMutationHandler;
}) {
  const [open, setOpen] = useState(() => {
    if (
      typeof window === "undefined" ||
      typeof window.matchMedia !== "function"
    ) {
      return true;
    }

    return window.matchMedia("(min-width: 1280px)").matches;
  });

  useEffect(() => {
    if (
      typeof window === "undefined" ||
      typeof window.matchMedia !== "function"
    ) {
      return;
    }

    const query = window.matchMedia("(min-width: 1280px)");
    const handleChange = () => setOpen(query.matches);
    query.addEventListener("change", handleChange);

    return () => query.removeEventListener("change", handleChange);
  }, []);

  return (
    <aside
      className={cn(
        "min-w-0 shrink-0 overflow-hidden border-y border-border/70 transition-[width] duration-200 ease-out xl:border-r xl:border-y-0 xl:pr-5",
        open ? "w-full xl:w-[560px]" : "w-full xl:w-12",
      )}
    >
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((current) => !current)}
        className={cn(
          "flex w-full items-center justify-between gap-3 pb-3 text-left transition-colors hover:text-foreground",
          open ? "-mt-1.5 pt-0" : "pt-0",
          !open && "xl:h-full xl:min-h-80 xl:flex-col xl:justify-start xl:px-0",
        )}
      >
        <div
          className={cn(
            "flex items-center gap-2",
            !open && "xl:flex-col xl:gap-3",
          )}
        >
          <Gauge className="size-4 text-primary" />
          <span
            className={cn(
              "text-lg font-semibold tracking-tight",
              !open && "xl:[writing-mode:vertical-rl]",
            )}
          >
            Overclocking
          </span>
        </div>
        <ChevronDown
          className={cn(
            "size-4 text-muted-foreground transition-transform",
            open ? "rotate-90" : "-rotate-90 xl:rotate-0",
          )}
        />
      </button>

      {!open &&
        Object.keys(editStore.devices[gpu.id]?.edits ?? {}).length > 0 && (
          <Badge variant="outline">Unapplied changes</Badge>
        )}
      {open && (
        <fieldset
          disabled={controlsDisabled || !onMutationAction}
          aria-disabled={controlsDisabled || !onMutationAction}
          className={cn(
            "pb-5 transition-opacity xl:pb-0",
            (controlsDisabled || !onMutationAction) &&
              "pointer-events-none opacity-45 saturate-50",
          )}
        >
          <GpuTuningControls
            gpu={gpu}
            editStore={editStore}
            layout="rail"
            pendingAction={pendingAction}
            onMutationAction={onMutationAction ?? (() => undefined)}
          />
        </fieldset>
      )}
    </aside>
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
      {PERFORMANCE_METRICS.map((metric) => (
        <TelemetryMetricCard
          key={metric.key}
          gpu={gpu}
          metric={metric}
          subtitle={getMetricSubtitle(metric, gpu, totalMemoryLabel)}
          history={metric.getHistory(history)}
        />
      ))}
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
  gpu,
  metric,
  subtitle,
  history,
}: {
  gpu: GpuPerformanceDevice;
  metric: PerformanceMetricDefinition;
  subtitle: string;
  history: MetricHistorySeries | undefined;
}) {
  const style = {
    "--telemetry-accent": metric.accentColor,
  } as CSSProperties;
  const value = metric.formatCurrentValue(metric.getValue(gpu));
  const to = `/performance/${encodeURIComponent(gpu.id)}/${metric.key}`;

  return (
    <Link
      aria-label={`Open ${metric.label} details for ${gpu.name}`}
      draggable={false}
      style={style}
      to={to}
      className="group min-w-0 border-b border-border/70 pb-5 outline-none transition-colors hover:border-foreground/35 focus-visible:border-primary focus-visible:ring-2 focus-visible:ring-ring/50"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="text-[11px] font-medium tracking-[0.22em] text-muted-foreground uppercase">
            {metric.label}
          </p>
          <p className="mt-2 text-2xl font-semibold tabular-nums tracking-tight">
            {value}
          </p>
        </div>
        <div className="mt-1 flex items-start gap-2">
          <ArrowUpRight className="size-3.5 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100" />
          <span
            className="block h-8 w-px opacity-80 transition-opacity group-hover:opacity-100"
            style={{ backgroundColor: metric.accentColor }}
          />
        </div>
      </div>
      <p className="mt-1 text-xs text-muted-foreground">{subtitle}</p>
      <div className="mt-4">
        <MetricSparkline
          series={history}
          accentColor={metric.accentColor}
          formatValue={metric.formatValue}
        />
      </div>
    </Link>
  );
}

function getMetricSubtitle(
  metric: PerformanceMetricDefinition,
  gpu: GpuPerformanceDevice,
  totalMemoryLabel: string,
) {
  if (metric.key === "fanSpeedPercent" && gpu.cooling.fanCount > 1) {
    return `${gpu.cooling.fanCount} fans averaged`;
  }

  if (metric.key === "memoryUsedMiB") {
    return totalMemoryLabel;
  }

  return metric.subtitle;
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
          <Badge
            className="rounded-md border-border/70 bg-transparent"
            variant="outline"
          >
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
  const samples = series?.samples.slice(-OVERVIEW_SPARKLINE_SAMPLE_LIMIT);
  const hasData = samples?.some((value) => value != null) ?? false;

  if (!hasData || !samples) {
    return (
      <div
        className="flex items-center justify-center border border-dashed border-border/70 bg-muted/10 text-xs text-muted-foreground"
        style={{ height: SPARKLINE_HEIGHT }}
      >
        Unavailable
      </div>
    );
  }

  const bounds = calculateSparklineBounds(samples);
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
    samples,
    SPARKLINE_WIDTH,
    SPARKLINE_HEIGHT,
    SPARKLINE_INSETS,
  );
  const lastPoint = findLastSparklinePoint(
    samples,
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
    <div
      data-testid="gpu-performance-skeleton"
      className="border-y border-border/80 py-7"
    >
      <div className="flex flex-col gap-8 xl:flex-row">
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

function formatCompactPercent(value: number | null) {
  return value == null ? "N/A" : `${value} %`;
}

function formatCompactCelsius(value: number | null) {
  return value == null ? "N/A" : `${value}C`;
}

function formatVoltage(value: number | null) {
  if (value == null) {
    return "Unavailable";
  }

  return `${new Intl.NumberFormat("en-US", {
    maximumFractionDigits: 3,
  }).format(value / 1000)} V`;
}
