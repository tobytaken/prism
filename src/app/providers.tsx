import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";

import { GpuMetricHistoryProvider } from "@/features/gpus/hooks/use-gpu-metric-history";
import { AppearanceProvider } from "@/features/settings/lib/appearance-provider";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: Number.POSITIVE_INFINITY,
      refetchOnWindowFocus: false,
      retry: false,
    },
  },
});

export function AppProviders({ children }: PropsWithChildren) {
  return (
    <QueryClientProvider client={queryClient}>
      <GpuMetricHistoryProvider>
        <AppearanceProvider>{children}</AppearanceProvider>
      </GpuMetricHistoryProvider>
    </QueryClientProvider>
  );
}
