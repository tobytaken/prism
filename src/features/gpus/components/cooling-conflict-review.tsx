import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import { describeCooling } from "./cooling-control-status";
import type { CoolingControl, FanControlInfo } from "@/features/gpus/types";
import type {
  TuningEdits,
  FanCurveDraftPoint,
} from "@/features/gpus/hooks/use-tuning-edits";

export function CoolingConflictReview({
  control,
  fans,
  edits,
  busy,
  onReview,
}: {
  control: CoolingControl;
  fans: FanControlInfo[];
  edits: TuningEdits;
  busy: boolean;
  onReview: (revision: number) => void;
}) {
  const [review, setReview] = useState<
    (CoolingControl & { fans: FanControlInfo[] }) | null
  >(null);
  return (
    <div role="alert" className="space-y-2 text-xs">
      <p>
        Cooling changed since this draft was opened. Your draft is kept. Review
        the current controls before applying.
      </p>
      <Button
        size="sm"
        disabled={busy}
        onClick={() => setReview(structuredClone({ ...control, fans }))}
      >
        Review cooling change
      </Button>
      <Dialog
        open={review != null}
        onOpenChange={(open) => {
          if (!open) setReview(null);
        }}
      >
        <DialogContent>
          <DialogTitle>Review cooling change</DialogTitle>
          <DialogDescription>
            Compare the current cooling configuration with your draft. Keeping
            the draft for applying makes no changes to the GPU.
          </DialogDescription>
          {review && (
            <>
              <p>
                Prism configuration:{" "}
                {review.applied
                  ? describeCooling(review.applied)
                  : "No Prism cooling configuration"}
              </p>
              {review.applied?.arrangement === "shared" &&
                review.applied.setting.mode === "curve" && (
                  <p>{curveText(review.applied.setting.points)}</p>
                )}
              {review.applied?.arrangement === "perFan" &&
                review.applied.fans.map(
                  ({ fanIndex, setting }) =>
                    setting.mode === "curve" && (
                      <p key={fanIndex}>
                        Fan {fanIndex + 1}: {curveText(setting.points)}
                      </p>
                    ),
                )}
              {review.fans.map((fan) => (
                <p key={fan.index}>
                  Driver reports fan {fan.index + 1}: {fan.mode ?? "unknown"}{" "}
                  mode,{" "}
                  {fan.currentSpeedPercent == null
                    ? "unknown speed"
                    : `${fan.currentSpeedPercent}%`}
                  .
                </p>
              ))}
              {review.suspension && (
                <p>Suspended: {review.suspension.reason}</p>
              )}
              <p>
                Your draft:{" "}
                {edits.cooling?.mode === "curve"
                  ? curveText(edits.cooling.points)
                  : edits.cooling?.mode === "automatic"
                    ? "Automatic cooling"
                    : "Individual fan changes"}
              </p>
              {Object.entries(edits)
                .filter(([key]) => key.startsWith("fan:"))
                .map(([key, value]) => (
                  <p key={key}>
                    Fan {Number(key.slice(4)) + 1}:{" "}
                    {value === "auto"
                      ? "Automatic"
                      : `${value ?? "Incomplete"}%`}
                  </p>
                ))}
              {review.revision !== control.revision && (
                <p>
                  Cooling changed again. Close and review the latest controls.
                </p>
              )}
              <Button
                disabled={busy || review.revision !== control.revision}
                onClick={() => {
                  onReview(review.revision);
                  setReview(null);
                }}
              >
                Keep draft for applying
              </Button>
              <Button variant="outline" onClick={() => setReview(null)}>
                Cancel
              </Button>
            </>
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}

function curveText(points: FanCurveDraftPoint[]) {
  return points
    .map(
      (point) =>
        `${point.temperatureC ?? "?"}°C → ${point.speedPercent ?? "?"}%`,
    )
    .join(", ");
}
