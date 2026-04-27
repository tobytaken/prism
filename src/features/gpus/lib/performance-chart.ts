export type SparklineInsets = {
  top: number;
  right: number;
  bottom: number;
  left: number;
};

export function buildSparklinePath(
  samples: Array<number | null>,
  width: number,
  height: number,
  insets: SparklineInsets,
) {
  const points = calculateSparklinePoints(samples, width, height, insets);
  if (points.length === 0) {
    return `M ${insets.left} ${height - insets.bottom}`;
  }

  return points
    .map((point, index) => `${index === 0 ? "M" : "L"} ${point.x} ${point.y}`)
    .join(" ");
}

export function findLastSparklinePoint(
  samples: Array<number | null>,
  width: number,
  height: number,
  insets: SparklineInsets,
) {
  const points = calculateSparklinePoints(samples, width, height, insets);
  return points.at(-1);
}

export function calculateSparklinePoints(
  samples: Array<number | null>,
  width: number,
  height: number,
  insets: SparklineInsets,
) {
  const bounds = calculateSparklineBounds(samples);
  if (!bounds) {
    return [];
  }

  const range = bounds.max - bounds.min || 1;
  const horizontalSpan = Math.max(width - insets.left - insets.right, 1);
  const verticalSpan = Math.max(height - insets.top - insets.bottom, 1);

  return samples.flatMap((value, index) => {
    if (value == null) {
      return [];
    }

    const x =
      insets.left +
      (samples.length === 1 ? 0 : (horizontalSpan * index) / (samples.length - 1));
    const normalized = (value - bounds.min) / range;
    const y = height - insets.bottom - normalized * verticalSpan;

    return [{ x, y }];
  });
}

export function calculateSparklineBounds(samples: Array<number | null>) {
  const values = samples.filter((value): value is number => value != null);
  if (values.length === 0) {
    return null;
  }

  return {
    min: Math.min(...values),
    max: Math.max(...values),
  };
}

export function calculateMetricStats(samples: Array<number | null>) {
  const values = samples.filter((value): value is number => value != null);
  if (values.length === 0) {
    return null;
  }

  const total = values.reduce((sum, value) => sum + value, 0);

  return {
    min: Math.min(...values),
    max: Math.max(...values),
    average: total / values.length,
  };
}
