export function formatNumber(value: number | null, unit?: string) {
  if (value == null) {
    return "Unavailable";
  }

  const formatted = new Intl.NumberFormat("en-US").format(value);
  return unit ? `${formatted} ${unit}` : formatted;
}

export function formatText(value: string | null) {
  return value ?? "Unavailable";
}

export function formatVendor(vendor: "nvidia" | "amd" | "intel") {
  switch (vendor) {
    case "nvidia":
      return "NVIDIA";
    case "amd":
      return "AMD";
    case "intel":
      return "Intel";
  }
}
