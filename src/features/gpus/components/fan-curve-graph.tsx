import { useId, useRef } from "react";

import type { FanCurveDraftPoint } from "@/features/gpus/hooks/use-tuning-edits";
import type { FanCurvePoint, FanCurveSupport } from "@/features/gpus/types";

const GRAPH = {
  width: 320,
  height: 180,
  left: 30,
  right: 10,
  top: 10,
  bottom: 22,
};

/** The curve on temperature/speed axes, with the full-speed boundary and the live temperature. */
export function FanCurveGraph({
  points,
  support,
  temperatureC,
  editable,
  protectedEndpointIndex,
  onPointChange,
}: {
  points: FanCurveDraftPoint[];
  support: Extract<FanCurveSupport, { status: "available" }>;
  temperatureC: number | null;
  editable: boolean;
  protectedEndpointIndex: number | null;
  onPointChange: (index: number, point: FanCurvePoint) => void;
}) {
  const instructionsId = useId();
  const graphRef = useRef<HTMLDivElement>(null);
  const drag = useRef<{ index: number; pointerId: number } | null>(null);
  const drawn = points.filter(
    (point): point is FanCurvePoint =>
      point.temperatureC != null && point.speedPercent != null,
  );
  const maxTemperature = support.maxOperatingTemperatureC;
  const plotWidth = GRAPH.width - GRAPH.left - GRAPH.right;
  const plotHeight = GRAPH.height - GRAPH.top - GRAPH.bottom;
  const x = (temperature: number) =>
    GRAPH.left +
    (Math.min(Math.max(temperature, 0), maxTemperature) / maxTemperature) *
      plotWidth;
  const y = (speed: number) =>
    GRAPH.top + (1 - Math.min(Math.max(speed, 0), 100) / 100) * plotHeight;
  // Beyond its end points a curve holds the nearest end point's speed.
  const line = drawn.length
    ? [
        [0, drawn[0].speedPercent],
        ...drawn.map((point) => [point.temperatureC, point.speedPercent]),
        [maxTemperature, drawn[drawn.length - 1].speedPercent],
      ]
        .map(([temperature, speed]) => `${x(temperature)},${y(speed)}`)
        .join(" ")
    : "";
  const label = `Fan curve graph: ${
    drawn
      .map((point) => `${point.temperatureC}°C ${point.speedPercent}%`)
      .join(", ") || "no complete points"
  }. GPU temperature ${temperatureC == null ? "unavailable" : `now ${temperatureC}°C`}.`;

  const movePoint = (index: number, temperature: number, speed: number) => {
    if (!editable) return;
    const point = points[index];
    const previous = points[index - 1];
    const next = points[index + 1];
    // Keep the sole point meeting the thermal endpoint at maximum speed and at or below T.
    const holdsEndpoint = index === protectedEndpointIndex;
    const minTemperature =
      previous?.temperatureC != null ? previous.temperatureC + 1 : 0;
    const maxTemperature = Math.min(
      next?.temperatureC != null
        ? next.temperatureC - 1
        : support.maxOperatingTemperatureC,
      holdsEndpoint
        ? support.fullSpeedTemperatureC
        : support.maxOperatingTemperatureC,
    );
    const minSpeed = holdsEndpoint
      ? support.maxSpeedPercent
      : Math.max(
          support.minSpeedPercent,
          previous?.speedPercent ?? support.minSpeedPercent,
        );
    const maxSpeed = Math.min(
      support.maxSpeedPercent,
      next?.speedPercent ?? support.maxSpeedPercent,
    );
    if (minTemperature > maxTemperature || minSpeed > maxSpeed) return;
    const changed = {
      temperatureC: Math.min(
        maxTemperature,
        Math.max(minTemperature, Math.round(temperature)),
      ),
      speedPercent: Math.min(maxSpeed, Math.max(minSpeed, Math.round(speed))),
    };
    if (
      changed.temperatureC !== point.temperatureC ||
      changed.speedPercent !== point.speedPercent
    )
      onPointChange(index, changed);
  };

  return (
    <div>
      <p id={instructionsId} className="mb-2 text-xs text-muted-foreground">
        Drag a point, or focus it and use Left/Right for temperature and Up/Down
        for speed.
        {editable ? "" : " Graph editing is available once the curve is valid."}
      </p>
      <div
        ref={graphRef}
        role="group"
        aria-label="Fan curve graph editor"
        className="relative"
      >
        <svg
          role="img"
          aria-label={label}
          viewBox={`0 0 ${GRAPH.width} ${GRAPH.height}`}
          className="block w-full text-[9px]"
        >
          <g aria-hidden="true">
            {[0, 50, 100].map((speed) => (
              <g key={speed}>
                <line
                  x1={GRAPH.left}
                  x2={GRAPH.width - GRAPH.right}
                  y1={y(speed)}
                  y2={y(speed)}
                  className="stroke-border"
                />
                <text
                  x={GRAPH.left - 4}
                  y={y(speed) + 3}
                  textAnchor="end"
                  className="fill-muted-foreground"
                >
                  {speed}%
                </text>
              </g>
            ))}
            {[0, support.fullSpeedTemperatureC, maxTemperature].map(
              (temperature) => (
                <text
                  key={temperature}
                  x={x(temperature)}
                  y={GRAPH.height - 6}
                  textAnchor="middle"
                  className="fill-muted-foreground"
                >
                  {temperature}°C
                </text>
              ),
            )}
            <line
              x1={x(support.fullSpeedTemperatureC)}
              x2={x(support.fullSpeedTemperatureC)}
              y1={GRAPH.top}
              y2={GRAPH.top + plotHeight}
              strokeDasharray="3 3"
              className="stroke-amber-500"
            />
            {temperatureC != null && (
              <line
                x1={x(temperatureC)}
                x2={x(temperatureC)}
                y1={GRAPH.top}
                y2={GRAPH.top + plotHeight}
                className="stroke-sky-500"
              />
            )}
            {line && (
              <polyline
                points={line}
                fill="none"
                strokeWidth={2}
                className="stroke-primary"
              />
            )}
          </g>
        </svg>
        {points.map(
          (point, index) =>
            point.temperatureC != null &&
            point.speedPercent != null && (
              <button
                key={index}
                type="button"
                aria-label={`Point ${index + 1}: ${point.temperatureC}°C, ${point.speedPercent}%`}
                aria-describedby={instructionsId}
                aria-disabled={!editable}
                className="absolute flex size-6 -translate-x-1/2 -translate-y-1/2 touch-none items-center justify-center rounded-full cursor-grab active:cursor-grabbing focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring"
                style={{
                  left: `${(x(point.temperatureC) / GRAPH.width) * 100}%`,
                  top: `${(y(point.speedPercent) / GRAPH.height) * 100}%`,
                }}
                onPointerDown={(event) => {
                  if (!editable || event.button !== 0 || drag.current) return;
                  event.currentTarget.focus();
                  event.currentTarget.setPointerCapture(event.pointerId);
                  drag.current = { index, pointerId: event.pointerId };
                  event.preventDefault();
                }}
                onPointerMove={(event) => {
                  if (
                    drag.current?.index !== index ||
                    drag.current.pointerId !== event.pointerId
                  )
                    return;
                  const bounds = graphRef.current?.getBoundingClientRect();
                  if (!bounds?.width || !bounds.height) return;
                  const localX =
                    ((event.clientX - bounds.left) / bounds.width) *
                    GRAPH.width;
                  const localY =
                    ((event.clientY - bounds.top) / bounds.height) *
                    GRAPH.height;
                  movePoint(
                    index,
                    ((localX - GRAPH.left) / plotWidth) * maxTemperature,
                    (1 - (localY - GRAPH.top) / plotHeight) * 100,
                  );
                }}
                onPointerUp={(event) => {
                  if (drag.current?.pointerId !== event.pointerId) return;
                  drag.current = null;
                  event.currentTarget.releasePointerCapture(event.pointerId);
                }}
                onPointerCancel={() => {
                  drag.current = null;
                }}
                onLostPointerCapture={() => {
                  drag.current = null;
                }}
                onKeyDown={(event) => {
                  const temperature = point.temperatureC!;
                  const speed = point.speedPercent!;
                  switch (event.key) {
                    case "ArrowLeft":
                      movePoint(index, temperature - 1, speed);
                      break;
                    case "ArrowRight":
                      movePoint(index, temperature + 1, speed);
                      break;
                    case "ArrowUp":
                      movePoint(index, temperature, speed + 1);
                      break;
                    case "ArrowDown":
                      movePoint(index, temperature, speed - 1);
                      break;
                    default:
                      return;
                  }
                  event.preventDefault();
                }}
              >
                <span className="size-2 rounded-full bg-primary ring-2 ring-background" />
              </button>
            ),
        )}
      </div>
    </div>
  );
}
