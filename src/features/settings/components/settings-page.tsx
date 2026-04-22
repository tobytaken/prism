import { SettingsScreen } from "@/features/settings/components/settings-screen";
import { useAppearance } from "@/features/settings/lib/appearance-provider";

export function SettingsPage() {
  const {
    accentPreference,
    effectiveAccent,
    detectedAccent,
    isDetectingAccent,
    detectionErrorMessage,
    setAccentPreference,
  } = useAppearance();

  return (
    <SettingsScreen
      accentPreference={accentPreference}
      effectiveAccent={effectiveAccent}
      detectedAccent={detectedAccent}
      isDetectingAccent={isDetectingAccent}
      detectionErrorMessage={detectionErrorMessage}
      onAccentPreferenceChange={setAccentPreference}
    />
  );
}
