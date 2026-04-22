import type { PropsWithChildren } from "react";
import { createContext, useContext, useEffect, useState } from "react";

import { useGpuInventory } from "@/features/gpus/hooks/use-gpu-inventory";

import {
  applyAccentTheme,
  getStoredAccentPreference,
  resolveDetectedAccent,
  resolveEffectiveAccent,
  setStoredAccentPreference,
  type AccentPreference,
  type ResolvedAccent,
} from "./accent-theme";

type AppearanceContextValue = {
  accentPreference: AccentPreference;
  effectiveAccent: ResolvedAccent | null;
  detectedAccent: ResolvedAccent | null;
  isDetectingAccent: boolean;
  detectionErrorMessage: string | null;
  setAccentPreference: (preference: AccentPreference) => void;
};

const AppearanceContext = createContext<AppearanceContextValue | null>(null);

export function AppearanceProvider({ children }: PropsWithChildren) {
  const [accentPreference, setAccentPreferenceState] = useState<AccentPreference>(() =>
    getStoredAccentPreference(),
  );
  const inventoryQuery = useGpuInventory();
  const detectedAccent = resolveDetectedAccent(inventoryQuery.data?.gpus);
  const effectiveAccent = resolveEffectiveAccent(accentPreference, detectedAccent);
  const detectionErrorMessage =
    inventoryQuery.error instanceof Error ? inventoryQuery.error.message : null;

  useEffect(() => {
    applyAccentTheme(effectiveAccent);
  }, [effectiveAccent]);

  return (
    <AppearanceContext.Provider
      value={{
        accentPreference,
        effectiveAccent,
        detectedAccent,
        isDetectingAccent: inventoryQuery.isPending,
        detectionErrorMessage,
        setAccentPreference: (preference) => {
          setAccentPreferenceState(preference);
          setStoredAccentPreference(preference);
        },
      }}
    >
      {children}
    </AppearanceContext.Provider>
  );
}

export function useAppearance() {
  const context = useContext(AppearanceContext);

  if (context === null) {
    throw new Error("useAppearance must be used within an AppearanceProvider.");
  }

  return context;
}
