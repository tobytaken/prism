import { useId } from "react";
import { DatabaseZap } from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import type {
  TuningStoreHealth,
  TuningStoreProblemKind,
} from "@/features/gpus/types";

const PROBLEM_COPY: Record<
  TuningStoreProblemKind,
  { cause: string; remedy: string }
> = {
  unreadable: {
    cause: "Prism couldn't read its saved tuning state.",
    remedy:
      "Restore the file from a backup or move it aside, then restart prismd.",
  },
  newerVersion: {
    cause:
      "Prism's saved tuning state was written by a newer version of Prism.",
    remedy:
      "Update Prism to use it, or move the file aside and restart prismd to start over.",
  },
};

/** Explains why the daemon can't use its saved tuning state, if it can't. */
export function TuningStoreProblemAlert({
  health,
}: {
  health: TuningStoreHealth | null | undefined;
}) {
  const titleId = useId();
  if (health?.status !== "unhealthy") {
    return null;
  }
  const copy = PROBLEM_COPY[health.problem];

  return (
    <Alert
      variant="destructive"
      aria-labelledby={titleId}
      className="border-destructive/50 bg-destructive/12"
    >
      <DatabaseZap className="text-destructive" />
      <AlertTitle id={titleId}>Saved tuning state can't be used</AlertTitle>
      <AlertDescription className="text-destructive/90">
        <div className="space-y-1">
          <p>
            {copy.cause} It left the file untouched and won't change it until
            this is resolved.
          </p>
          <p>Manual tuning still works. {copy.remedy}</p>
          <p className="font-mono text-xs text-destructive/80">{health.path}</p>
          <p className="text-xs text-destructive/80">{health.detail}</p>
        </div>
      </AlertDescription>
    </Alert>
  );
}
