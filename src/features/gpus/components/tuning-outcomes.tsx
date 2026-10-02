import type { TuningMutationResult } from "@/features/gpus/lib/tuning-mutation";

export function TuningOutcomes({
  outcomes,
}: {
  outcomes: TuningMutationResult;
}) {
  return Object.entries(outcomes).map(([setting, outcome]) =>
    outcome ? (
      <p key={setting}>
        {tuningSettingLabel(setting)}:{" "}
        {outcome.status === "succeeded"
          ? "Applied"
          : outcome.status === "failed"
            ? "Failed"
            : outcome.status === "unattempted"
              ? "Not attempted"
              : "Outcome unknown"}
        {outcome.error ? ` — ${outcome.error}` : ""}
      </p>
    ) : null,
  );
}

export function tuningSettingLabel(setting: string) {
  return setting === "power"
    ? "Power limit"
    : setting === "graphics"
      ? "Graphics clock offset"
      : setting === "memory"
        ? "Memory clock offset"
        : `Fan ${Number(setting.slice(4)) + 1}`;
}
