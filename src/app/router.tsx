import { createHashRouter } from "react-router-dom";

import { AppShell } from "@/app/shell/app-shell";
import { GpuDetailPage } from "@/features/gpus/components/gpu-detail-page";
import { GpuOverviewPage } from "@/features/gpus/components/gpu-overview-page";
import { GpuPerformanceMetricPage } from "@/features/gpus/components/gpu-performance-metric-page";
import { GpuPerformancePage } from "@/features/gpus/components/gpu-performance-page";
import { PlaceholderPage } from "@/features/gpus/components/placeholder-page";
import { SettingsPage } from "@/features/settings/components/settings-page";

export const appRouter = createHashRouter([
  {
    path: "/",
    element: <AppShell />,
    children: [
      { index: true, element: <GpuOverviewPage /> },
      { path: "gpus/:gpuId", element: <GpuDetailPage /> },
      { path: "drivers", element: <PlaceholderPage title="Drivers" /> },
      { path: "performance", element: <GpuPerformancePage /> },
      { path: "performance/:gpuId/:metricKey", element: <GpuPerformanceMetricPage /> },
      { path: "profiles", element: <PlaceholderPage title="Profiles" /> },
      { path: "settings", element: <SettingsPage /> },
    ],
  },
]);
