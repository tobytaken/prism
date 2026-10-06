import { createHashRouter, type RouteObject } from "react-router-dom";

import { AppShell } from "@/app/shell/app-shell";
import { DriversPage } from "@/features/drivers/components/drivers-page";
import { GamesPage } from "@/features/games/components/games-page";
import { GpuDetailPage } from "@/features/gpus/components/gpu-detail-page";
import { GpuOverviewPage } from "@/features/gpus/components/gpu-overview-page";
import { GpuPerformanceMetricPage } from "@/features/gpus/components/gpu-performance-metric-page";
import { GpuPerformancePage } from "@/features/gpus/components/gpu-performance-page";
import { SettingsPage } from "@/features/settings/components/settings-page";

export const appRoutes: RouteObject[] = [
  {
    path: "/",
    element: <AppShell />,
    children: [
      { index: true, element: <GpuOverviewPage /> },
      { path: "gpus/:gpuId", element: <GpuDetailPage /> },
      { path: "drivers", element: <DriversPage /> },
      { path: "performance", element: <GpuPerformancePage /> },
      {
        path: "performance/:gpuId/:metricKey",
        element: <GpuPerformanceMetricPage />,
      },
      { path: "games", element: <GamesPage /> },
      { path: "settings", element: <SettingsPage /> },
    ],
  },
];

export const appRouter = createHashRouter(appRoutes);
