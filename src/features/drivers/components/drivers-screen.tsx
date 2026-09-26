import {
  AlertTriangle,
  CheckCircle2,
  Download,
  HardDriveDownload,
  Loader2,
  Power,
  ShieldAlert,
  X,
} from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import type {
  DriverAction,
  DriverJob,
  DriverPlan,
  DriverStatusResponse,
  NvidiaGpuInfo,
} from "@/features/drivers/types";

type DriversScreenProps = {
  status?: DriverStatusResponse;
  isLoading: boolean;
  errorMessage: string | null;
  isStarting: boolean;
  isCancelling: boolean;
  isRebooting: boolean;
  actionError: string | null;
  onStartAction: (action: DriverAction) => void;
  onCancelJob: (jobId: string) => void;
  onRequestReboot: () => void;
};

export function DriversScreen({
  status,
  isLoading,
  errorMessage,
  isStarting,
  isCancelling,
  isRebooting,
  actionError,
  onStartAction,
  onCancelJob,
  onRequestReboot,
}: DriversScreenProps) {
  const plans = status?.nvidia.plans ?? [];
  const primaryPlan =
    plans.find((plan) => plan.recommended) ??
    plans.find((plan) => plan.action === "installRecommended");
  const activeJob = status?.activeJob ?? null;
  const needsReboot =
    activeJob?.status === "rebootRequired" ||
    activeJob?.status === "manualStepRequired";

  return (
    <div className="space-y-6">
      <div className="flex flex-col gap-4 md:flex-row md:items-start md:justify-between">
        <div className="space-y-1">
          <h1 className="text-2xl font-semibold tracking-tight">Drivers</h1>
          <p className="text-sm text-muted-foreground">
            NVIDIA driver install and update status for this host.
          </p>
        </div>
        {status && (
          <Badge
            variant={status.host.supported ? "default" : "outline"}
            className="h-6"
          >
            {status.host.distroId} {status.host.distroVersion}
          </Badge>
        )}
      </div>

      {errorMessage && (
        <Alert variant="destructive">
          <AlertTriangle />
          <AlertTitle>Failed to load driver status</AlertTitle>
          <AlertDescription>{errorMessage}</AlertDescription>
        </Alert>
      )}

      {actionError && (
        <Alert variant="destructive">
          <AlertTriangle />
          <AlertTitle>Driver action failed</AlertTitle>
          <AlertDescription>{actionError}</AlertDescription>
        </Alert>
      )}

      {isLoading && <DriversSkeleton />}

      {!isLoading && status && (
        <>
          <HostSummary status={status} />

          {status.warnings.length > 0 && (
            <WarningList title="Driver warnings" warnings={status.warnings} />
          )}
          {status.nvidia.warnings.length > 0 && (
            <WarningList
              title="NVIDIA diagnostics"
              warnings={status.nvidia.warnings}
            />
          )}

          {activeJob && (
            <DriverJobPanel
              job={activeJob}
              isCancelling={isCancelling}
              onCancelJob={onCancelJob}
            />
          )}

          {primaryPlan && (
            <DriverPlanPanel
              plan={primaryPlan}
              isStarting={isStarting}
              disabled={Boolean(
                activeJob?.status === "queued" ||
                activeJob?.status === "running",
              )}
              onStartAction={onStartAction}
            />
          )}

          {needsReboot && (
            <div className="flex justify-end">
              <Button onClick={() => onRequestReboot()} disabled={isRebooting}>
                <Power className="size-4" />
                {isRebooting ? "Requesting reboot" : "Reboot now"}
              </Button>
            </div>
          )}
        </>
      )}
    </div>
  );
}

function HostSummary({ status }: { status: DriverStatusResponse }) {
  const driverVersion = status.nvidia.driverVersion ?? "Not loaded";

  return (
    <div className="grid gap-4 lg:grid-cols-[1.1fr_0.9fr]">
      <Card className="border-border/80 bg-card/70">
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <HardDriveDownload className="size-4 text-primary" />
            Host
          </CardTitle>
        </CardHeader>
        <CardContent>
          <dl className="grid gap-3 text-sm sm:grid-cols-2">
            <SummaryItem
              label="Package manager"
              value={status.host.packageManager}
            />
            <SummaryItem label="Kernel" value={status.host.kernelRelease} />
            <SummaryItem label="Secure Boot" value={status.host.secureBoot} />
            <SummaryItem label="NVIDIA driver" value={driverVersion} />
          </dl>
          {status.host.unsupportedReason && (
            <p className="mt-4 text-sm text-muted-foreground">
              {status.host.unsupportedReason}
            </p>
          )}
        </CardContent>
      </Card>

      <Card className="border-border/80 bg-card/70">
        <CardHeader>
          <CardTitle className="text-base">Detected NVIDIA GPUs</CardTitle>
        </CardHeader>
        <CardContent>
          {status.nvidia.gpus.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No NVIDIA GPU detected.
            </p>
          ) : (
            <div className="space-y-3">
              {status.nvidia.gpus.map((gpu) => (
                <GpuRow key={gpu.id} gpu={gpu} />
              ))}
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

function DriverPlanPanel({
  plan,
  isStarting,
  disabled,
  onStartAction,
}: {
  plan: DriverPlan;
  isStarting: boolean;
  disabled: boolean;
  onStartAction: (action: DriverAction) => void;
}) {
  const canRun = plan.blockers.length === 0 && !disabled;

  return (
    <Card className="border-border/80 bg-card/70">
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
          {plan.blockers.length > 0 ? (
            <ShieldAlert className="size-4 text-primary" />
          ) : (
            <Download className="size-4 text-primary" />
          )}
          {plan.label}
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        {plan.blockers.length > 0 && (
          <WarningList title="Blocked" warnings={plan.blockers} />
        )}
        {plan.warnings.length > 0 && (
          <WarningList title="Notes" warnings={plan.warnings} />
        )}
        {plan.manualSteps.length > 0 && (
          <div className="space-y-2">
            <p className="text-xs uppercase tracking-[0.18em] text-muted-foreground">
              Completion
            </p>
            <ul className="space-y-1 text-sm text-muted-foreground">
              {plan.manualSteps.map((step) => (
                <li key={step}>{step}</li>
              ))}
            </ul>
          </div>
        )}
        <div className="flex justify-end">
          <Button
            onClick={() => onStartAction(plan.action)}
            disabled={!canRun || isStarting}
            aria-label={
              plan.action === "updateInstalled"
                ? "Update driver"
                : "Install recommended driver"
            }
          >
            {isStarting ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <Download className="size-4" />
            )}
            {plan.action === "updateInstalled"
              ? "Update driver"
              : "Install recommended driver"}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

function DriverJobPanel({
  job,
  isCancelling,
  onCancelJob,
}: {
  job: DriverJob;
  isCancelling: boolean;
  onCancelJob: (jobId: string) => void;
}) {
  const active = job.status === "queued" || job.status === "running";

  return (
    <Card className="border-border/80 bg-card/70">
      <CardHeader>
        <CardTitle className="flex items-center justify-between gap-3 text-base">
          <span className="flex items-center gap-2">
            {active ? (
              <Loader2 className="size-4 animate-spin text-primary" />
            ) : (
              <CheckCircle2 className="size-4 text-primary" />
            )}
            Driver job
          </span>
          <Badge variant="outline">{job.status}</Badge>
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
          <p className="text-sm text-muted-foreground">
            {job.currentStep ?? job.error ?? "No active step"}
          </p>
          {active && (
            <Button
              variant="outline"
              onClick={() => onCancelJob(job.id)}
              disabled={isCancelling}
            >
              <X className="size-4" />
              Cancel
            </Button>
          )}
        </div>
        {job.logTail.length > 0 && (
          <pre className="max-h-64 overflow-auto rounded-md border border-border bg-muted/35 p-3 text-xs leading-5 text-muted-foreground">
            {job.logTail.join("\n")}
          </pre>
        )}
      </CardContent>
    </Card>
  );
}

function WarningList({
  title,
  warnings,
}: {
  title: string;
  warnings: string[];
}) {
  return (
    <Alert>
      <AlertTriangle className="text-primary" />
      <AlertTitle>{title}</AlertTitle>
      <AlertDescription className="space-y-1">
        {warnings.map((warning) => (
          <p key={warning}>{warning}</p>
        ))}
      </AlertDescription>
    </Alert>
  );
}

function SummaryItem({ label, value }: { label: string; value: string }) {
  return (
    <div className="space-y-1">
      <dt className="text-xs uppercase tracking-[0.18em] text-muted-foreground">
        {label}
      </dt>
      <dd className="font-medium tabular-nums">{value}</dd>
    </div>
  );
}

function GpuRow({ gpu }: { gpu: NvidiaGpuInfo }) {
  return (
    <div className="rounded-md border border-border bg-background/45 p-3">
      <div className="flex flex-wrap items-center gap-2">
        <p className="font-medium">{gpu.name}</p>
        {gpu.architecture && (
          <Badge variant="outline">{gpu.architecture}</Badge>
        )}
      </div>
      {gpu.busId && (
        <p className="mt-1 font-mono text-xs text-muted-foreground">
          {gpu.busId}
        </p>
      )}
    </div>
  );
}

function DriversSkeleton() {
  return (
    <div data-testid="drivers-skeleton" className="space-y-4">
      <div className="grid gap-4 lg:grid-cols-2">
        <Skeleton className="h-48 rounded-lg" />
        <Skeleton className="h-48 rounded-lg" />
      </div>
      <Skeleton className="h-52 rounded-lg" />
    </div>
  );
}
