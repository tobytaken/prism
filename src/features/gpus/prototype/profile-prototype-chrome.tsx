// PROTOTYPE (#7) — static stand-ins for page chrome and telemetry so the variants
// are judged at realistic density. Not part of the decision.
export function PageTitle({ title, children }: { title: string; children?: React.ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-4">
      <h1 className="text-3xl font-semibold tracking-tight">{title}</h1>
      {children}
    </div>
  );
}

export function Headline() {
  return (
    <div className="hidden items-center divide-x divide-border/70 border-x border-border/70 md:flex">
      {[
        ["Power", "212 W"],
        ["GPU", "64 %"],
        ["Temp", "61C"],
      ].map(([label, value]) => (
        <div key={label} className="px-4 py-1.5">
          <p className="text-[10px] font-medium tracking-[0.18em] text-muted-foreground uppercase">{label}</p>
          <p className="mt-0.5 text-sm font-semibold tabular-nums">{value}</p>
        </div>
      ))}
    </div>
  );
}

export function FakeTelemetry() {
  return (
    <div className="grid min-w-0 flex-1 gap-x-7 gap-y-6 opacity-60 md:grid-cols-2 2xl:grid-cols-3">
      {["Power draw", "GPU utilization", "Temperature", "Graphics clock", "Memory clock", "Fan speed"].map((label) => (
        <div key={label} className="border-b border-border/70 pb-5">
          <p className="text-[11px] font-medium tracking-[0.22em] text-muted-foreground uppercase">{label}</p>
          <p className="mt-2 text-2xl font-semibold tabular-nums">—</p>
          <div className="mt-4 h-[77px] border border-dashed border-border/70 bg-muted/10" />
        </div>
      ))}
    </div>
  );
}
