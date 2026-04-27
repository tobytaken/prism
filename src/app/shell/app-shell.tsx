import {
  BookCopy,
  Gem,
  Gauge,
  HardDriveDownload,
  LayoutGrid,
  Settings2,
} from "lucide-react";
import { NavLink, Outlet } from "react-router-dom";
import { useEffect, useState } from "react";

import { TitleBar } from "@/components/title-bar";
import { Separator } from "@/components/ui/separator";
import { cn } from "@/lib/utils";

const navigation = [
  { label: "Overview", to: "/", icon: LayoutGrid },
  { label: "Drivers", to: "/drivers", icon: HardDriveDownload },
  { label: "Performance", to: "/performance", icon: Gauge },
  { label: "Profiles", to: "/profiles", icon: BookCopy },
  { label: "Settings", to: "/settings", icon: Settings2 },
] as const;

export function AppShell() {
  const [isMaximized, setIsMaximized] = useState(false);

  useEffect(() => {
    let isActive = true;

    const refreshMaximizedState = async () => {
      const nextState = await window.prism.window.isMaximized();
      if (isActive) {
        setIsMaximized(nextState);
      }
    };

    void refreshMaximizedState();

    const cleanup = window.prism.window.onResized((nextState) => {
      setIsMaximized(nextState);
    });

    return () => {
      isActive = false;
      cleanup();
    };
  }, []);

  return (
    <div className="h-dvh bg-transparent text-foreground">
      <div
        className={cn(
          "flex h-full flex-col overflow-hidden border border-border/80 bg-background shadow-2xl shadow-black/20",
          isMaximized ? "rounded-none" : "rounded-[6px]",
        )}
      >
        <TitleBar isMaximized={isMaximized} />
        <div className="flex flex-1 overflow-hidden">
          <aside className="flex w-56 shrink-0 flex-col border-r border-border bg-sidebar">
            <div className="px-4 pb-4 pt-5">
              <div className="flex items-center gap-3">
                <div className="flex size-8 items-center justify-center rounded-lg border border-border bg-muted">
                  <Gem className="size-4 text-primary" />
                </div>
                <div>
                  <p className="text-sm font-semibold text-foreground">Prism</p>
                  <p className="text-xs text-muted-foreground">Linux GPU Manager</p>
                </div>
              </div>
            </div>

            <Separator />

            <nav className="flex flex-1 flex-col gap-1 p-3">
              {navigation.map((item) => {
                const Icon = item.icon;

                return (
                  <NavLink
                    key={item.to}
                    to={item.to}
                    end={item.to === "/"}
                    className={({ isActive }) =>
                      cn(
                        "flex items-center gap-3 rounded-sm px-3 py-2 text-sm font-medium transition-colors",
                        isActive
                          ? "bg-muted text-foreground"
                          : "text-muted-foreground hover:bg-muted/70 hover:text-foreground",
                      )
                    }
                  >
                    {({ isActive }) => (
                      <>
                        <Icon
                          className={cn(
                            "size-4",
                            isActive ? "text-primary" : "text-muted-foreground",
                          )}
                        />
                        <span>{item.label}</span>
                      </>
                    )}
                  </NavLink>
                );
              })}
            </nav>
          </aside>

          <div className="flex-1 overflow-y-auto">
            <main className="mx-auto max-w-7xl p-5 md:p-6">
              <Outlet />
            </main>
          </div>
        </div>
      </div>
    </div>
  );
}
