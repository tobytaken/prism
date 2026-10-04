// PROTOTYPE helper: floating variant switcher for throwaway UI prototypes. Never ship.
import { ChevronLeft, ChevronRight } from "lucide-react";
import { useEffect, type ReactNode } from "react";

export function PrototypeSwitcher({
  variants,
  current,
  onChange,
  children,
}: {
  variants: { key: string; name: string }[];
  current: string;
  onChange: (key: string) => void;
  children?: ReactNode;
}) {
  const index = Math.max(
    0,
    variants.findIndex((v) => v.key === current),
  );
  const step = (delta: number) =>
    onChange(variants[(index + delta + variants.length) % variants.length].key);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (
        target?.closest("input, textarea, select, [contenteditable='true']")
      )
        return;
      if (event.key === "ArrowLeft") step(-1);
      if (event.key === "ArrowRight") step(1);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  if (import.meta.env.PROD) return null;

  return (
    <div className="fixed bottom-4 left-1/2 z-[100] flex -translate-x-1/2 items-center gap-2 rounded-full border-2 border-fuchsia-400 bg-black px-2 py-1.5 text-sm text-white shadow-2xl shadow-fuchsia-500/30">
      <button
        type="button"
        aria-label="Previous variant"
        className="rounded-full p-1 hover:bg-white/15"
        onClick={() => step(-1)}
      >
        <ChevronLeft className="size-4" />
      </button>
      <span className="min-w-56 text-center font-medium">
        <span className="text-fuchsia-300">PROTOTYPE</span> {variants[index].key}{" "}
        ({variants[index].name})
      </span>
      <button
        type="button"
        aria-label="Next variant"
        className="rounded-full p-1 hover:bg-white/15"
        onClick={() => step(1)}
      >
        <ChevronRight className="size-4" />
      </button>
      {children}
    </div>
  );
}
