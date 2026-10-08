import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { Minus, Plus, RotateCcw } from "lucide-react";
import type { ReactNode } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { FanCurveGraph } from "./fan-curve-graph";
import { previewGpuFanCurve } from "@/features/gpus/api";
import type { FanCurveDraftPoint } from "@/features/gpus/hooks/use-tuning-edits";
import type {
  FanCurvePoint,
  FanCurveProblem,
  FanCurveSupport,
  GpuPerformanceDevice,
} from "@/features/gpus/types";

const MIN_POINTS = 2;
const MAX_POINTS = 16;

type AvailableCurve = Extract<FanCurveSupport, { status: "available" }>;
type CurveState = "unapplied" | "applied" | null;

/**
 * Edits one shared curve for every fan on the GPU. The daemon judges what the GPU supports and
 * checks the curve; only Apply asks it to follow the curve.
 */
export function FanCurveSection({
  gpu,
  draft,
  busy,
  onDraftChange,
  onApply,
}: {
  gpu: GpuPerformanceDevice;
  /** The edited curve, or null to show the applied curve, else the starter curve. */
  draft: FanCurveDraftPoint[] | null;
  busy: boolean;
  onDraftChange: (points: FanCurveDraftPoint[] | null) => void;
  onApply: (points: FanCurvePoint[]) => void;
}) {
  const support = gpu.cooling.curveSupport;
  if (support?.status !== "available") {
    return (
      <CurveShell state={null}>
        <p className="border-l border-border/80 bg-muted/20 px-3 py-2 text-xs text-muted-foreground">
          {support?.reason ?? "Fan curves need the Prism daemon."} Automatic and
          fixed fan control stay available.
        </p>
      </CurveShell>
    );
  }
  const applied = appliedCurve(gpu);
  const points = draft ?? applied ?? support.starter;
  return (
    <FanCurveEditor
      gpu={gpu}
      support={support}
      points={points}
      state={draft != null ? "unapplied" : applied ? "applied" : null}
      busy={busy}
      onDraftChange={onDraftChange}
      onApply={onApply}
    />
  );
}

/** The curve every fan follows, when Prism applied one shared curve. */
function appliedCurve(gpu: GpuPerformanceDevice): FanCurvePoint[] | null {
  const applied = gpu.cooling.control?.applied;
  return applied?.arrangement === "shared" && applied.setting.mode === "curve"
    ? applied.setting.points
    : null;
}

function CurveShell({
  state,
  children,
}: {
  state: CurveState;
  children: ReactNode;
}) {
  return (
    <div
      role="group"
      aria-label="Fan curve"
      className="space-y-3 border-t border-border/70 pt-4"
    >
      <div className="flex items-center justify-between gap-3">
        <p className="text-xs font-medium uppercase tracking-[0.22em] text-muted-foreground">
          Fan curve · all fans
        </p>
        {state && (
          <Badge variant="outline">
            {state === "applied" ? "Applied" : "Unapplied"}
          </Badge>
        )}
      </div>
      {children}
    </div>
  );
}

function FanCurveEditor({
  gpu,
  support,
  points,
  state,
  busy,
  onDraftChange,
  onApply,
}: {
  gpu: GpuPerformanceDevice;
  support: AvailableCurve;
  points: FanCurveDraftPoint[];
  state: CurveState;
  busy: boolean;
  onDraftChange: (points: FanCurveDraftPoint[] | null) => void;
  onApply: (points: FanCurvePoint[]) => void;
}) {
  const inputProblems = points.flatMap(inputProblemsAt);
  const complete =
    inputProblems.length === 0 ? (points as FanCurvePoint[]) : null;
  // Keyed by the limits too, so a capability change rechecks the curve.
  const preview = useQuery({
    queryKey: ["fan-curve-preview", gpu.id, complete, support],
    queryFn: () => previewGpuFanCurve({ gpuId: gpu.id, points: complete! }),
    enabled: complete != null,
    // While an edit is rechecked, the previous reasons stay rather than flickering away.
    placeholderData: keepPreviousData,
  });
  const checked = preview.isSuccess && !preview.isPlaceholderData;
  const problems = [
    ...inputProblems,
    ...((complete && preview.data?.problems) || []),
  ];
  const invalidPoints = new Set(problems.map((problem) => problem.point));
  const change = (index: number, point: Partial<FanCurveDraftPoint>) =>
    onDraftChange(
      points.map((current, at) =>
        at === index ? { ...current, ...point } : current,
      ),
    );
  const fanCount = gpu.cooling.fans.length;
  const valid = checked && problems.length === 0;
  const isStarter = sameCurve(points, support.starter);
  const addedPoints =
    points.length < MAX_POINTS ? withAddedPoint(points) : null;
  const thermalEndpoints = points.flatMap((point, index) =>
    point.temperatureC != null &&
    point.temperatureC <= support.fullSpeedTemperatureC &&
    point.speedPercent === support.maxSpeedPercent
      ? [index]
      : [],
  );
  const protectedEndpointIndex =
    thermalEndpoints.length === 1 ? thermalEndpoints[0] : null;

  return (
    <CurveShell state={state}>
      <p className="text-xs text-muted-foreground">
        Reaches {support.maxSpeedPercent}% by {support.fullSpeedTemperatureC}
        °C, {support.maxOperatingTemperatureC - support.fullSpeedTemperatureC}°C
        below the {support.maxOperatingTemperatureC}°C maximum operating
        temperature. Speeds {support.minSpeedPercent}-{support.maxSpeedPercent}%
        for{" "}
        {fanCount === 1
          ? "the fan"
          : fanCount === 2
            ? "both fans"
            : `all ${fanCount} fans`}
        .
      </p>
      <FanCurveGraph
        points={points}
        support={support}
        temperatureC={gpu.telemetry.temperatureC}
        editable={complete != null && problems.length === 0}
        protectedEndpointIndex={protectedEndpointIndex}
        onPointChange={change}
      />
      <p className="text-xs text-muted-foreground">
        Current GPU temperature{" "}
        {gpu.telemetry.temperatureC == null
          ? "unavailable"
          : `${gpu.telemetry.temperatureC}°C`}
      </p>
      <ol className="space-y-2">
        {points.map((point, index) => (
          <li
            key={index}
            className="grid grid-cols-[auto_1fr_1fr_auto] items-center gap-2"
          >
            <span className="w-5 text-xs text-muted-foreground tabular-nums">
              {index + 1}
            </span>
            <PointInput
              label={`Point ${index + 1} temperature (°C)`}
              unit="°C"
              value={point.temperatureC}
              invalid={invalidPoints.has(index)}
              onChange={(temperatureC) => change(index, { temperatureC })}
            />
            <PointInput
              label={`Point ${index + 1} speed (%)`}
              unit="%"
              value={point.speedPercent}
              invalid={invalidPoints.has(index)}
              onChange={(speedPercent) => change(index, { speedPercent })}
            />
            <Button
              size="icon"
              variant="ghost"
              aria-label={`Remove point ${index + 1}`}
              disabled={
                points.length <= MIN_POINTS || index === protectedEndpointIndex
              }
              onClick={() =>
                onDraftChange(points.filter((_, at) => at !== index))
              }
            >
              <Minus className="size-3.5" />
            </Button>
          </li>
        ))}
      </ol>
      <div className="grid grid-cols-2 gap-2">
        <Button
          size="sm"
          variant="outline"
          disabled={addedPoints == null}
          onClick={() => addedPoints && onDraftChange(addedPoints)}
        >
          <Plus className="mr-1 size-3.5" />
          Add point
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={isStarter}
          onClick={() => onDraftChange(support.starter)}
        >
          <RotateCcw className="mr-1 size-3.5" />
          Use starter curve
        </Button>
      </div>
      <Button
        size="sm"
        className="w-full"
        disabled={!valid || busy || !gpu.stableId}
        onClick={() => complete && onApply(complete)}
      >
        Apply curve
      </Button>
      <div aria-live="polite" className="space-y-1 text-xs">
        {problems.length > 0 ? (
          <ul className="space-y-1 text-destructive">
            {problems.map((problem) => (
              <li key={problem.message}>{problem.message}</li>
            ))}
          </ul>
        ) : preview.isError ? (
          <p className="text-destructive">
            Couldn't check this curve: {preview.error.message}
          </p>
        ) : !gpu.stableId ? (
          <p className="text-muted-foreground">
            Prism can't follow a curve on this GPU because its stable identity
            (UUID) couldn't be verified.
          </p>
        ) : checked ? (
          <p className="text-muted-foreground">
            {state === "applied"
              ? "Valid curve. Prism follows it for every fan, even after you close the app."
              : "Valid curve. Apply it to have Prism follow it for every fan, even after you close the app."}
          </p>
        ) : (
          <p className="text-muted-foreground">Checking the curve…</p>
        )}
      </div>
    </CurveShell>
  );
}

function sameCurve(points: FanCurveDraftPoint[], curve: FanCurvePoint[]) {
  return (
    points.length === curve.length &&
    points.every(
      (point, index) =>
        point.temperatureC === curve[index].temperatureC &&
        point.speedPercent === curve[index].speedPercent,
    )
  );
}

function PointInput({
  label,
  unit,
  value,
  invalid,
  onChange,
}: {
  label: string;
  unit: string;
  value: number | null;
  invalid: boolean;
  onChange: (value: number | null) => void;
}) {
  return (
    <label className="flex items-center gap-1">
      <Input
        type="number"
        aria-label={label}
        aria-invalid={invalid}
        className="h-8 rounded-none border-x-0 border-t-0 bg-transparent px-0 tabular-nums shadow-none focus-visible:ring-0"
        step={1}
        value={value ?? ""}
        onChange={(event) => {
          const next = event.currentTarget.valueAsNumber;
          if (event.currentTarget.value === "") onChange(null);
          else if (Number.isFinite(next)) onChange(next);
        }}
      />
      <span className="text-xs text-muted-foreground">{unit}</span>
    </label>
  );
}

/** Problems the daemon can't check: empty inputs and fractions. */
function inputProblemsAt(
  point: FanCurveDraftPoint,
  index: number,
): FanCurveProblem[] {
  const problem = (message: string) => ({
    point: index,
    message: `Point ${index + 1}: ${message}`,
  });
  return [
    point.temperatureC == null
      ? problem("enter a temperature.")
      : !Number.isInteger(point.temperatureC)
        ? problem("temperature must be a whole number of degrees.")
        : null,
    point.speedPercent == null
      ? problem("enter a speed.")
      : !Number.isInteger(point.speedPercent)
        ? problem("speed must be a whole percentage.")
        : null,
  ].filter((problem) => problem != null);
}

/**
 * Splits the widest temperature gap at its midpoint, which keeps a valid curve valid. Without
 * room for another whole degree, addition is unavailable. Existing invalid inputs are untouched.
 */
function withAddedPoint(
  points: FanCurveDraftPoint[],
): FanCurveDraftPoint[] | null {
  let widest = -1;
  for (let index = 0; index + 1 < points.length; index++) {
    const [low, high] = [points[index], points[index + 1]];
    const gap = gapBetween(low, high);
    if (
      gap >= 2 &&
      (widest < 0 || gap > gapBetween(points[widest], points[widest + 1]))
    )
      widest = index;
  }
  if (widest < 0) return null;
  const [low, high] = [points[widest], points[widest + 1]];
  const midpoint = {
    temperatureC: Math.floor((low.temperatureC! + high.temperatureC!) / 2),
    speedPercent: Math.floor((low.speedPercent! + high.speedPercent!) / 2),
  };
  return [
    ...points.slice(0, widest + 1),
    midpoint,
    ...points.slice(widest + 1),
  ];
}

function gapBetween(low: FanCurveDraftPoint, high: FanCurveDraftPoint) {
  return [
    low.temperatureC,
    high.temperatureC,
    low.speedPercent,
    high.speedPercent,
  ].every((value) => value != null && Number.isInteger(value))
    ? high.temperatureC! - low.temperatureC!
    : -1;
}
