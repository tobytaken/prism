import { Check, Info, Palette, ShieldCheck } from "lucide-react";
import { useState } from "react";

import { Separator } from "@/components/ui/separator";
import {
  accentThemes,
  type AccentPreference,
  type ResolvedAccent,
} from "@/features/settings/lib/accent-theme";
import { cn } from "@/lib/utils";

type SettingsScreenProps = {
  accentPreference: AccentPreference;
  effectiveAccent: ResolvedAccent | null;
  detectedAccent: ResolvedAccent | null;
  isDetectingAccent: boolean;
  detectionErrorMessage: string | null;
  onAccentPreferenceChange: (preference: AccentPreference) => void;
};

type SettingsTab = "appearance" | "daemon" | "about";

const settingsTabs = [
  {
    id: "appearance",
    label: "Appearance",
    icon: Palette,
  },
  {
    id: "daemon",
    label: "Daemon",
    icon: ShieldCheck,
  },
  {
    id: "about",
    label: "About",
    icon: Info,
  },
] as const satisfies Array<{
  id: SettingsTab;
  label: string;
  icon: typeof Palette;
}>;

const unresolvedAutoSwatch = "#7C8DBB";

export function SettingsScreen({
  accentPreference,
  detectedAccent,
  isDetectingAccent,
  detectionErrorMessage,
  onAccentPreferenceChange,
}: SettingsScreenProps) {
  const [activeTab, setActiveTab] = useState<SettingsTab>("appearance");

  return (
    <div className="mx-auto flex h-full max-w-5xl flex-col px-4 py-8 md:px-8">
      <div className="mb-8">
        <h1 className="text-3xl font-bold tracking-tight">Settings</h1>
        <p className="mt-2 text-muted-foreground">
          Manage your Prism preferences and configuration.
        </p>
      </div>

      <div className="flex flex-1 flex-col gap-8 md:flex-row lg:gap-12">
        <aside className="w-full shrink-0 md:w-[200px] lg:w-[240px]">
          <nav
            role="tablist"
            aria-label="Settings sections"
            className="flex overflow-x-auto pb-2 md:flex-col md:gap-1 md:pb-0"
          >
            {settingsTabs.map((tab) => {
              const Icon = tab.icon;
              const isActive = activeTab === tab.id;

              return (
                <button
                  key={tab.id}
                  type="button"
                  role="tab"
                  aria-selected={isActive}
                  aria-controls={`panel-${tab.id}`}
                  id={`tab-${tab.id}`}
                  onClick={() => setActiveTab(tab.id)}
                  className={cn(
                    "flex items-center gap-3 whitespace-nowrap rounded-md px-3 py-2 text-sm font-medium transition-colors",
                    isActive
                      ? "bg-secondary text-secondary-foreground"
                      : "text-muted-foreground hover:bg-secondary/50 hover:text-foreground",
                  )}
                >
                  <Icon className="size-4" />
                  {tab.label}
                </button>
              );
            })}
          </nav>
        </aside>

        <main className="flex-1 overflow-y-auto">
          {activeTab === "appearance" && (
            <div id="panel-appearance" role="tabpanel" aria-labelledby="tab-appearance">
              <AppearancePanel
                accentPreference={accentPreference}
                detectedAccent={detectedAccent}
                isDetectingAccent={isDetectingAccent}
                detectionErrorMessage={detectionErrorMessage}
                onAccentPreferenceChange={onAccentPreferenceChange}
              />
            </div>
          )}
          {activeTab === "daemon" && (
            <div id="panel-daemon" role="tabpanel" aria-labelledby="tab-daemon">
              <DaemonPanel />
            </div>
          )}
          {activeTab === "about" && (
            <div id="panel-about" role="tabpanel" aria-labelledby="tab-about">
              <AboutPanel />
            </div>
          )}
        </main>
      </div>
    </div>
  );
}

function AppearancePanel({
  accentPreference,
  detectedAccent,
  isDetectingAccent,
  detectionErrorMessage,
  onAccentPreferenceChange,
}: {
  accentPreference: AccentPreference;
  detectedAccent: ResolvedAccent | null;
  isDetectingAccent: boolean;
  detectionErrorMessage: string | null;
  onAccentPreferenceChange: (preference: AccentPreference) => void;
}) {
  const autoSwatch = detectedAccent ? accentThemes[detectedAccent].color : unresolvedAutoSwatch;

  const autoSummary = getAutoSummary({
    detectedAccent,
    isDetectingAccent,
    detectionErrorMessage,
  });

  const accentOptions = [
    {
      id: "amd" as const,
      label: accentThemes.amd.label,
      color: accentThemes.amd.color,
      summary: undefined,
    },
    {
      id: "nvidia" as const,
      label: accentThemes.nvidia.label,
      color: accentThemes.nvidia.color,
      summary: undefined,
    },
    {
      id: "intel" as const,
      label: accentThemes.intel.label,
      color: accentThemes.intel.color,
      summary: undefined,
    },
    {
      id: "auto" as const,
      label: "Auto",
      color: autoSwatch,
      summary: autoSummary,
    },
  ];

  return (
    <div className="max-w-2xl space-y-6">
      <div>
        <h2 className="text-xl font-semibold tracking-tight">Appearance</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Customize the visual style of the application.
        </p>
      </div>

      <Separator />

      <div className="space-y-4">
        <h3 className="text-sm font-medium">Accent Color</h3>
        <div
          role="radiogroup"
          aria-label="Accent Color"
          className="grid grid-cols-2 gap-4 sm:grid-cols-4"
        >
          {accentOptions.map((option) => (
            <button
              key={option.id}
              type="button"
              role="radio"
              aria-checked={accentPreference === option.id}
              onClick={() => onAccentPreferenceChange(option.id)}
              className={cn(
                "group relative flex flex-col items-center gap-3 rounded-xl border p-4 text-center transition-all hover:bg-secondary/40",
                accentPreference === option.id
                  ? "border-primary bg-secondary/20 shadow-sm"
                  : "border-border/60",
              )}
            >
              <div
                className="flex size-10 items-center justify-center rounded-full shadow-inner ring-1 ring-black/10 dark:ring-white/10"
                style={{ backgroundColor: option.color }}
              >
                {accentPreference === option.id && (
                  <Check className="size-5 text-white drop-shadow-md" />
                )}
              </div>
              <span className="text-sm font-medium">{option.label}</span>
              {option.id === "auto" && option.summary && (
                <span className="sr-only">{option.summary}</span>
              )}
            </button>
          ))}
        </div>
        {accentPreference === "auto" && (
          <p className="text-sm text-muted-foreground">{autoSummary}</p>
        )}
      </div>
    </div>
  );
}

function DaemonPanel() {
  return (
    <div className="max-w-2xl space-y-6">
      <div>
        <h2 className="text-xl font-semibold tracking-tight">Daemon</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Manage background service configuration.
        </p>
      </div>
      <Separator />
      <div className="rounded-lg border border-dashed p-8 text-center">
        <p className="text-sm text-muted-foreground">
          Daemon settings will appear here in a future update.
        </p>
      </div>
    </div>
  );
}

function AboutPanel() {
  return (
    <div className="max-w-2xl space-y-6">
      <div>
        <h2 className="text-xl font-semibold tracking-tight">About</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Platform Scope and system information.
        </p>
      </div>
      <Separator />
      <div className="space-y-2 text-sm text-muted-foreground">
        <p>Prism is a modern control center for your GPU.</p>
        <p>Currently supporting NVIDIA on Linux, with AMD support planned.</p>
      </div>
    </div>
  );
}

function getAutoSummary({
  detectedAccent,
  isDetectingAccent,
  detectionErrorMessage,
}: {
  detectedAccent: ResolvedAccent | null;
  isDetectingAccent: boolean;
  detectionErrorMessage: string | null;
}) {
  if (detectedAccent) {
    const theme = accentThemes[detectedAccent];
    return `Currently resolves to ${theme.label} ${theme.tone}.`;
  }

  if (isDetectingAccent) {
    return "Waiting for GPU inventory before choosing an accent automatically.";
  }

  if (detectionErrorMessage) {
    return "Hardware detection is unavailable right now, so Auto keeps the default accent.";
  }

  return "No supported GPU vendor is available yet. Auto will switch once one is detected.";
}
