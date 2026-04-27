import type {
  GpuHistoryKey,
  GpuMetricHistory,
  MetricHistorySeries,
} from "@/features/gpus/hooks/use-gpu-metric-history";
import { formatNumber } from "@/features/gpus/lib/formatters";
import type { GpuPerformanceDevice } from "@/features/gpus/types";

export type PerformanceMetricKey = GpuHistoryKey;

export type PerformanceMetricDefinition = {
  key: PerformanceMetricKey;
  label: string;
  subtitle: string;
  detailDescription: string;
  accentColor: string;
  getValue: (gpu: GpuPerformanceDevice) => number | null;
  getHistory: (history: GpuMetricHistory | undefined) => MetricHistorySeries | undefined;
  formatValue: (value: number) => string;
  formatCurrentValue: (value: number | null) => string;
};

export const PERFORMANCE_METRICS: PerformanceMetricDefinition[] = [
  {
    key: "powerDrawWatts",
    label: "Power",
    subtitle: "Board power draw",
    detailDescription: "Board power draw sampled from the driver telemetry feed.",
    accentColor: "var(--color-chart-1)",
    getValue: (gpu) => gpu.telemetry.powerDrawWatts,
    getHistory: (history) => history?.powerDrawWatts,
    formatValue: formatWatts,
    formatCurrentValue: (value) => formatNumber(value, "W"),
  },
  {
    key: "coreClockMHz",
    label: "Core Clock",
    subtitle: "Graphics domain clock",
    detailDescription: "Current graphics clock reported for the GPU core domain.",
    accentColor: "var(--color-chart-2)",
    getValue: (gpu) => gpu.telemetry.coreClockMHz,
    getHistory: (history) => history?.coreClockMHz,
    formatValue: formatMegahertz,
    formatCurrentValue: (value) => formatNumber(value, "MHz"),
  },
  {
    key: "memoryClockMHz",
    label: "Memory Clock",
    subtitle: "Current memory clock",
    detailDescription: "Current memory clock reported by the active driver.",
    accentColor: "var(--color-chart-3)",
    getValue: (gpu) => gpu.telemetry.memoryClockMHz,
    getHistory: (history) => history?.memoryClockMHz,
    formatValue: formatMegahertz,
    formatCurrentValue: (value) => formatNumber(value, "MHz"),
  },
  {
    key: "fanSpeedPercent",
    label: "Fan Speed",
    subtitle: "Current programmable fan speed",
    detailDescription: "Average programmable fan speed across the reported cooling devices.",
    accentColor: "var(--color-chart-4)",
    getValue: (gpu) => gpu.telemetry.fanSpeedPercent,
    getHistory: (history) => history?.fanSpeedPercent,
    formatValue: formatPercentValue,
    formatCurrentValue: formatPercent,
  },
  {
    key: "temperatureC",
    label: "Temperature",
    subtitle: "GPU die temperature",
    detailDescription: "GPU die temperature sampled from the driver thermal sensor.",
    accentColor: "var(--color-chart-5)",
    getValue: (gpu) => gpu.telemetry.temperatureC,
    getHistory: (history) => history?.temperatureC,
    formatValue: formatCelsius,
    formatCurrentValue: (value) => formatNumber(value, "°C"),
  },
  {
    key: "gpuUtilizationPercent",
    label: "GPU Utilization",
    subtitle: "Active compute time",
    detailDescription: "Percent of recent sample time spent executing GPU work.",
    accentColor: "var(--primary)",
    getValue: (gpu) => gpu.telemetry.gpuUtilizationPercent,
    getHistory: (history) => history?.gpuUtilizationPercent,
    formatValue: formatPercentValue,
    formatCurrentValue: formatPercent,
  },
  {
    key: "memoryUsedMiB",
    label: "VRAM Used",
    subtitle: "Frame buffer usage",
    detailDescription: "Amount of frame buffer memory currently allocated.",
    accentColor: "var(--color-chart-2)",
    getValue: (gpu) => gpu.telemetry.memoryUsedMiB,
    getHistory: (history) => history?.memoryUsedMiB,
    formatValue: formatMebibytes,
    formatCurrentValue: (value) => formatNumber(value, "MiB"),
  },
];

export function getPerformanceMetricDefinition(metricKey: string | undefined) {
  return PERFORMANCE_METRICS.find((metric) => metric.key === metricKey);
}

export function formatPercent(value: number | null) {
  return value == null ? "Unavailable" : `${value}%`;
}

export function formatWatts(value: number) {
  return formatNumber(value, "W");
}

export function formatMegahertz(value: number) {
  return formatNumber(value, "MHz");
}

export function formatCelsius(value: number) {
  return formatNumber(value, "°C");
}

export function formatMebibytes(value: number) {
  return formatNumber(value, "MiB");
}

export function formatPercentValue(value: number) {
  return `${value}%`;
}
