import type { CSSProperties } from "react";

import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import { formatVendor } from "@/features/gpus/lib/formatters";
import type { GpuVendor } from "@/features/gpus/types";

const vendorBadgeStyles: Record<GpuVendor, CSSProperties> = {
  nvidia: {
    color: "#76B900",
    backgroundColor: "rgba(118, 185, 0, 0.16)",
    borderColor: "rgba(118, 185, 0, 0.42)",
  },
  amd: {
    color: "#ED1C24",
    backgroundColor: "rgba(237, 28, 36, 0.16)",
    borderColor: "rgba(237, 28, 36, 0.42)",
  },
  intel: {
    color: "#0071C5",
    backgroundColor: "rgba(0, 113, 197, 0.16)",
    borderColor: "rgba(0, 113, 197, 0.42)",
  },
};

type VendorBadgeProps = {
  vendor: GpuVendor;
  className?: string;
};

export function VendorBadge({ vendor, className }: VendorBadgeProps) {
  return (
    <Badge
      variant="outline"
      data-vendor={vendor}
      className={cn("font-medium", className)}
      style={vendorBadgeStyles[vendor]}
    >
      {formatVendor(vendor)}
    </Badge>
  );
}
