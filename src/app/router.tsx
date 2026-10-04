import { createHashRouter } from "react-router-dom";

import { AppShell } from "@/app/shell/app-shell";
import { DriversPage } from "@/features/drivers/components/drivers-page";
import { GpuDetailPage } from "@/features/gpus/components/gpu-detail-page";
import { GpuOverviewPage } from "@/features/gpus/components/gpu-overview-page";
import { GpuPerformanceMetricPage } from "@/features/gpus/components/gpu-performance-metric-page";
// PROTOTYPE (#7): these wrappers render the real pages unless ?variant= is set.
import {
  PerformancePrototypeRoute,
  ProfilesPrototypeRoute,
} from "@/features/gpus/prototype/profile-prototype-routes";
import { SettingsPage } from "@/features/settings/components/settings-page";

export const appRouter = createHashRouter([
  {
    path: "/",
    element: <AppShell />,
    children: [
      { index: true, element: <GpuOverviewPage /> },
      { path: "gpus/:gpuId", element: <GpuDetailPage /> },
      { path: "drivers", element: <DriversPage /> },
      { path: "performance", element: <PerformancePrototypeRoute /> },
      {
        path: "performance/:gpuId/:metricKey",
        element: <GpuPerformanceMetricPage />,
      },
      { path: "profiles", element: <ProfilesPrototypeRoute /> },
      { path: "settings", element: <SettingsPage /> },
    ],
  },
]);
