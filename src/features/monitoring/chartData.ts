import type { EChartsOption } from "echarts";
import type { MonitoringHistorySample } from "@/stores/monitoringStore";
import type { FilesystemMetrics, MonitoringSnapshot } from "@/types/contracts";

export type MonitoringMetric = "cpu" | "memory" | "disk";

export type ChartPalette = {
  text: string;
  mutedText: string;
  axis: string;
  grid: string;
  tooltipBackground: string;
  tooltipBorder: string;
  tooltipText: string;
  cpu: string;
  memory: string;
  disk: string;
  fontFamily: string;
};

export type ChartPoint = [timestamp: number, value: number | null];

export type MonitoringChartModel = {
  metric: MonitoringMetric;
  title: string;
  description: string;
  seriesName: string;
  axisName: string;
  valueSuffix: string;
  data: ChartPoint[];
  validSamples: number;
  missingSamples: number;
  gapBreaks: number;
  latestValue: number | null;
  latestAt: number | null;
  diskMount: string | null;
};

export type BuildChartModelsOptions = {
  /** Expected polling cadence, used to break lines across unrecorded pauses. */
  intervalMs: number;
};

const GAP_FACTOR = 1.5;

const metricDefinitions: Record<
  MonitoringMetric,
  {
    title: string;
    description: string;
    seriesName: string;
    axisName: string;
    valueSuffix: string;
  }
> = {
  cpu: {
    title: "CPU load",
    description: "One-minute load average from /proc/loadavg.",
    seriesName: "Load average (1 min)",
    axisName: "Load average",
    valueSuffix: "load",
  },
  memory: {
    title: "Memory usage",
    description: "Used memory as a share of installed memory.",
    seriesName: "Memory used",
    axisName: "Memory used (%)",
    valueSuffix: "%",
  },
  disk: {
    title: "Disk usage",
    description: "Used space on one stable filesystem across the session.",
    seriesName: "Disk used",
    axisName: "Disk used (%)",
    valueSuffix: "%",
  },
};

function isFiniteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isUsablePercent(value: unknown): value is number {
  return isFiniteNumber(value) && value >= 0;
}

function filesystemsFromSnapshot(snapshot: MonitoringSnapshot): FilesystemMetrics[] {
  if (snapshot.filesystems.status !== "available" || !Array.isArray(snapshot.filesystems.data)) {
    return [];
  }

  return snapshot.filesystems.data;
}

/**
 * Keep one filesystem for the whole chart. `/` wins when present; otherwise
 * the largest filesystem in the newest sample is used. Mixing mounts in one
 * line would silently compare unrelated volumes.
 */
export function selectDiskMount(history: MonitoringHistorySample[]): string | null {
  const available = history.flatMap((sample) =>
    sample.snapshot ? filesystemsFromSnapshot(sample.snapshot) : [],
  );

  if (available.some((filesystem) => filesystem.mount === "/")) {
    return "/";
  }

  for (let index = history.length - 1; index >= 0; index -= 1) {
    const snapshot = history[index]?.snapshot;
    if (!snapshot) {
      continue;
    }

    const filesystems = filesystemsFromSnapshot(snapshot);
    if (filesystems.length === 0) {
      continue;
    }

    const largest = [...filesystems].sort((left, right) => {
      const leftTotal = isFiniteNumber(left.totalBytes) ? left.totalBytes : 0;
      const rightTotal = isFiniteNumber(right.totalBytes) ? right.totalBytes : 0;
      return rightTotal - leftTotal || left.mount.localeCompare(right.mount);
    });

    return largest[0]?.mount ?? null;
  }

  return null;
}

function metricValue(
  snapshot: MonitoringSnapshot,
  metric: MonitoringMetric,
  diskMount: string | null,
): number | null {
  if (metric === "cpu") {
    if (snapshot.cpu.status !== "available" || !snapshot.cpu.data) {
      return null;
    }

    const value = snapshot.cpu.data.load1m;
    return isFiniteNumber(value) && value >= 0 ? value : null;
  }

  if (metric === "memory") {
    if (snapshot.memory.status !== "available" || !snapshot.memory.data) {
      return null;
    }

    const value = snapshot.memory.data.usedPercent;
    return isUsablePercent(value) ? value : null;
  }

  if (!diskMount) {
    return null;
  }

  const filesystem = filesystemsFromSnapshot(snapshot).find(
    (candidate) => candidate.mount === diskMount,
  );
  return filesystem && isUsablePercent(filesystem.usedPercent) ? filesystem.usedPercent : null;
}

type InternalPoint = {
  at: number;
  value: number | null;
  fromSnapshot: boolean;
};

function buildPoints(
  history: MonitoringHistorySample[],
  metric: MonitoringMetric,
  diskMount: string | null,
  intervalMs: number,
): { data: ChartPoint[]; validSamples: number; missingSamples: number; gapBreaks: number } {
  const points: InternalPoint[] = [];

  for (const sample of history) {
    if (sample.snapshot) {
      const at = isFiniteNumber(sample.snapshot.sampledAt)
        ? sample.snapshot.sampledAt
        : sample.attemptedAt;
      if (!isFiniteNumber(at)) {
        continue;
      }

      points.push({
        at,
        value: metricValue(sample.snapshot, metric, diskMount),
        fromSnapshot: true,
      });
      continue;
    }

    if (isFiniteNumber(sample.attemptedAt)) {
      points.push({ at: sample.attemptedAt, value: null, fromSnapshot: false });
    }
  }

  points.sort((left, right) => left.at - right.at);

  // A duplicate timestamp is one sample, not two. The newest entry wins.
  const deduped: InternalPoint[] = [];
  for (const point of points) {
    const previous = deduped[deduped.length - 1];
    if (previous && previous.at === point.at) {
      deduped[deduped.length - 1] = point;
    } else {
      deduped.push(point);
    }
  }

  const gapThresholdMs = Math.max(0, intervalMs) * GAP_FACTOR;
  const data: ChartPoint[] = [];
  let validSamples = 0;
  let missingSamples = 0;
  let gapBreaks = 0;
  let previous: InternalPoint | null = null;

  for (const point of deduped) {
    const isGapBetweenSnapshots =
      previous !== null &&
      previous.value !== null &&
      point.value !== null &&
      gapThresholdMs > 0 &&
      point.at - previous.at > gapThresholdMs;

    if (isGapBetweenSnapshots && previous) {
      data.push([previous.at + Math.max(1, Math.floor((point.at - previous.at) / 2)), null]);
      gapBreaks += 1;
    }

    data.push([point.at, point.value]);
    if (point.value === null) {
      missingSamples += 1;
    } else {
      validSamples += 1;
    }
    previous = point;
  }

  return { data, validSamples, missingSamples, gapBreaks };
}

export function buildMonitoringChartModels(
  history: MonitoringHistorySample[],
  options: BuildChartModelsOptions,
): MonitoringChartModel[] {
  const diskMount = selectDiskMount(history);

  return (["cpu", "memory", "disk"] as const).map((metric) => {
    const definition = metricDefinitions[metric];
    const { data, validSamples, missingSamples, gapBreaks } = buildPoints(
      history,
      metric,
      diskMount,
      options.intervalMs,
    );
    const latestPoint = [...data].reverse().find((point) => point[1] !== null) ?? null;

    return {
      metric,
      ...definition,
      data,
      validSamples,
      missingSamples,
      gapBreaks,
      latestValue: latestPoint?.[1] ?? null,
      latestAt: latestPoint?.[0] ?? null,
      diskMount,
    };
  });
}

export function latestSuccessfulSample(
  history: MonitoringHistorySample[],
): MonitoringHistorySample | null {
  for (let index = history.length - 1; index >= 0; index -= 1) {
    const sample = history[index];
    if (sample?.snapshot) {
      return sample;
    }
  }

  return null;
}

export function formatMetricValue(model: MonitoringChartModel, value: number): string {
  if (model.metric === "cpu") {
    return `${value.toFixed(2)} load`;
  }

  return `${value.toFixed(1)}%`;
}

export function formatTooltipTimestamp(timestamp: number): string {
  const date = new Date(timestamp);
  return Number.isNaN(date.getTime()) ? "Unavailable" : date.toLocaleString();
}

export function formatAxisTimestamp(timestamp: number, spanMs: number): string {
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) {
    return "";
  }

  return date.toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: spanMs < 60 * 60 * 1000 ? "2-digit" : undefined,
  });
}

function withAlpha(color: string, alpha: number): string {
  const hex = color.trim();
  const match = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!match) {
    return color;
  }

  const value = match[1];
  const red = Number.parseInt(value.slice(0, 2), 16);
  const green = Number.parseInt(value.slice(2, 4), 16);
  const blue = Number.parseInt(value.slice(4, 6), 16);
  return `rgba(${red}, ${green}, ${blue}, ${alpha})`;
}

type TooltipParam = {
  value?: unknown;
};

function tooltipParams(params: unknown): TooltipParam[] {
  if (Array.isArray(params)) {
    return params as TooltipParam[];
  }

  return params ? [params as TooltipParam] : [];
}

export function buildChartOption(
  model: MonitoringChartModel,
  palette: ChartPalette,
): EChartsOption {
  const color = palette[model.metric];
  const firstTimestamp = model.data[0]?.[0] ?? 0;
  const lastTimestamp = model.data[model.data.length - 1]?.[0] ?? 0;
  const spanMs = Math.max(0, lastTimestamp - firstTimestamp);
  const paddingMs = Math.max(30_000, spanMs * 0.05);
  const isPercent = model.metric !== "cpu";

  return {
    animation: false,
    aria: {
      enabled: true,
      description: `${model.title} over ${model.validSamples} usable monitoring samples. Missing samples are shown as gaps, not interpolated values.`,
    },
    textStyle: {
      color: palette.text,
      fontFamily: palette.fontFamily,
      fontSize: 11,
    },
    grid: {
      left: 8,
      right: 48,
      top: 36,
      bottom: 24,
      containLabel: true,
    },
    tooltip: {
      trigger: "axis",
      backgroundColor: palette.tooltipBackground,
      borderColor: palette.tooltipBorder,
      textStyle: {
        color: palette.tooltipText,
        fontSize: 11,
      },
      axisPointer: {
        type: "line",
        lineStyle: {
          color: palette.axis,
          type: "dashed",
        },
      },
      formatter: (params: unknown) => {
        const point = tooltipParams(params).find((entry) => Array.isArray(entry.value));
        const value = point?.value;
        if (!Array.isArray(value) || value.length < 2) {
          return "";
        }

        const timestamp = Number(value[0]);
        const measurement = value[1];
        if (typeof measurement !== "number" || !Number.isFinite(measurement)) {
          return "";
        }

        return `${formatTooltipTimestamp(timestamp)}<br/>${model.seriesName}: ${formatMetricValue(
          model,
          measurement,
        )}`;
      },
    },
    xAxis: {
      type: "time",
      name: "Sample time",
      nameLocation: "middle",
      nameGap: 28,
      min: firstTimestamp - paddingMs,
      max: lastTimestamp + paddingMs,
      nameTextStyle: {
        color: palette.mutedText,
        fontSize: 10,
      },
      axisLine: {
        lineStyle: {
          color: palette.axis,
        },
      },
      axisTick: {
        lineStyle: {
          color: palette.axis,
        },
      },
      axisLabel: {
        color: palette.mutedText,
        hideOverlap: true,
        formatter: (value: number) => formatAxisTimestamp(value, spanMs),
      },
      splitLine: {
        show: false,
      },
    },
    yAxis: {
      type: "value",
      name: model.axisName,
      nameTextStyle: {
        color: palette.mutedText,
        fontSize: 10,
        align: "left",
      },
      min: 0,
      max: isPercent ? 100 : undefined,
      axisLine: {
        show: false,
      },
      axisTick: {
        show: false,
      },
      axisLabel: {
        color: palette.mutedText,
        formatter: (value: number) => (isPercent ? `${value}%` : `${value}`),
      },
      splitLine: {
        lineStyle: {
          color: palette.grid,
          type: "dashed",
        },
      },
    },
    series: [
      {
        name: model.seriesName,
        type: "line",
        data: model.data,
        connectNulls: false,
        showSymbol: model.validSamples <= 40,
        symbol: "circle",
        symbolSize: 4,
        lineStyle: {
          color,
          width: 1.5,
        },
        itemStyle: {
          color,
        },
        areaStyle: {
          color: withAlpha(color, 0.08),
        },
        emphasis: {
          focus: "series",
        },
      },
    ],
  };
}
