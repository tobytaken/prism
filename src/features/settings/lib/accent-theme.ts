export type AccentPreference = "auto" | "amd" | "nvidia" | "intel";
export type ResolvedAccent = Exclude<AccentPreference, "auto">;

type AccentThemeDefinition = {
  label: string;
  tone: string;
  description: string;
  color: string;
};

const STORAGE_KEY = "prism.accent-preference";

export const accentThemes: Record<ResolvedAccent, AccentThemeDefinition> = {
  amd: {
    label: "AMD",
    tone: "Red",
    description: "High-contrast highlights inspired by AMD's signature red.",
    color: "#ED1C24",
  },
  nvidia: {
    label: "NVIDIA",
    tone: "Green",
    description: "Apply NVIDIA green to primary actions, charts, and focus states.",
    color: "#76B900",
  },
  intel: {
    label: "Intel",
    tone: "Blue",
    description: "Use Intel blue for a cooler, more technical visual profile.",
    color: "#0071C5",
  },
};

export function getStoredAccentPreference(): AccentPreference {
  if (typeof window === "undefined") {
    return "auto";
  }

  const storedPreference = window.localStorage.getItem(STORAGE_KEY);

  if (
    storedPreference === "auto" ||
    storedPreference === "amd" ||
    storedPreference === "nvidia" ||
    storedPreference === "intel"
  ) {
    return storedPreference;
  }

  return "auto";
}

export function setStoredAccentPreference(preference: AccentPreference) {
  if (typeof window === "undefined") {
    return;
  }

  window.localStorage.setItem(STORAGE_KEY, preference);
}

export function resolveDetectedAccent(
  gpus?: Array<{ vendor: string | null | undefined }> | null,
): ResolvedAccent | null {
  for (const gpu of gpus ?? []) {
    const vendor = gpu.vendor?.toLowerCase();

    if (vendor === "amd" || vendor === "nvidia" || vendor === "intel") {
      return vendor;
    }
  }

  return null;
}

export function resolveEffectiveAccent(
  preference: AccentPreference,
  detectedAccent: ResolvedAccent | null,
): ResolvedAccent | null {
  if (preference === "auto") {
    return detectedAccent;
  }

  return preference;
}

export function applyAccentTheme(accent: ResolvedAccent | null) {
  if (typeof document === "undefined") {
    return;
  }

  const root = document.documentElement;

  if (accent === null) {
    root.style.removeProperty("--primary");
    root.style.removeProperty("--ring");
    root.style.removeProperty("--sidebar-primary");
    root.style.removeProperty("--chart-1");
    return;
  }

  const { color } = accentThemes[accent];

  root.style.setProperty("--primary", color);
  root.style.setProperty("--ring", `color-mix(in srgb, ${color} 55%, transparent)`);
  root.style.setProperty("--sidebar-primary", color);
  root.style.setProperty("--chart-1", color);
}
