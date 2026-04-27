import { Minus, Square, X } from "lucide-react";

import { cn } from "@/lib/utils";

type TitleBarProps = {
  isMaximized: boolean;
};

export function TitleBar({ isMaximized }: TitleBarProps) {
  return (
    <div
      className={cn(
        "app-region-drag flex h-9 shrink-0 select-none items-center justify-end border-b border-border bg-background px-1",
        isMaximized ? "rounded-none" : "rounded-t-[6px]",
      )}
    >
      <button
        onClick={() => void window.prism.window.minimize()}
        className={cn(
          "app-region-no-drag flex h-7 w-8 items-center justify-center rounded-sm text-muted-foreground transition-colors",
          "hover:bg-muted hover:text-foreground",
        )}
        tabIndex={-1}
      >
        <Minus className="size-3.5" />
      </button>
      <button
        onClick={() =>
          void (isMaximized
            ? window.prism.window.unmaximize()
            : window.prism.window.maximize())
        }
        className={cn(
          "app-region-no-drag flex h-7 w-8 items-center justify-center rounded-sm text-muted-foreground transition-colors",
          "hover:bg-muted hover:text-foreground",
        )}
        tabIndex={-1}
      >
        <Square className="size-3" />
      </button>
      <button
        onClick={() => void window.prism.window.close()}
        className={cn(
          "app-region-no-drag flex h-7 w-8 items-center justify-center rounded-sm text-muted-foreground transition-colors",
          "transition-colors hover:bg-red-500/80 hover:text-white",
        )}
        tabIndex={-1}
      >
        <X className="size-3.5" />
      </button>
    </div>
  );
}
