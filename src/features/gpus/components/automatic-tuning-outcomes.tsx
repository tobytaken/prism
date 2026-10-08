import type {
  ProfileApplicationResponse,
  RollbackResult,
} from "@/features/gpus/types";
import { TuningOutcomes } from "./tuning-outcomes";

function settingOutcomes(
  application: ProfileApplicationResponse | RollbackResult,
) {
  return {
    power: application.power ?? undefined,
    graphics: application.graphics ?? undefined,
    memory: application.memory ?? undefined,
    cooling: application.cooling?.outcome,
    ...Object.fromEntries(
      (application.cooling?.fans ?? []).map((fan) => [
        `fan:${fan.fanIndex}`,
        fan.outcome,
      ]),
    ),
  };
}

export function AutomaticTuningOutcomes({
  application,
}: {
  application: ProfileApplicationResponse;
}) {
  const rollback = application.rollback;
  return (
    <div className="space-y-1">
      <TuningOutcomes outcomes={settingOutcomes(application)} />
      {rollback && (
        <div className="space-y-1">
          <p>
            Rollback:{" "}
            {rollback.status === "restored"
              ? "Restored"
              : rollback.status === "restoreFailed"
                ? "Restore failed"
                : "Not needed"}
          </p>
          <TuningOutcomes outcomes={settingOutcomes(rollback)} />
          {rollback.coolingRestoration?.map((fan) => (
            <p key={fan.fanIndex}>
              Fan {fan.fanIndex + 1} automatic cooling:{" "}
              {fan.status === "restored"
                ? "Restored"
                : fan.status === "unverified"
                  ? "Unverified"
                  : "Failed"}
              {fan.detail && ` — ${fan.detail}`}
            </p>
          ))}
        </div>
      )}
    </div>
  );
}
