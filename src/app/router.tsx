import { createHashRouter } from "react-router-dom";

import { AppShell } from "@/app/shell/app-shell";
import { GpuDetailPage } from "@/features/gpus/components/gpu-detail-page";
import { GpuOverviewPage } from "@/features/gpus/components/gpu-overview-page";
import { GpuPerformancePage } from "@/features/gpus/components/gpu-performance-page";
import { GpuTuningPage } from "@/features/gpus/components/gpu-tuning-page";
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
      { path: "tuning", element: <GpuTuningPage /> },
      { path: "profiles", element: <PlaceholderPage title="Profiles" /> },
      { path: "settings", element: <SettingsPage /> },
    ],
  },
]);
