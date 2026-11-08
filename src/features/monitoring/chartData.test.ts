import { describe, expect, it } from "vitest";
import {
  buildChartOption,
  buildMonitoringChartModels,
  formatAxisTimestamp,
  latestSuccessfulSample,
  selectDiskMount,
  type ChartPalette,
} from "@/features/monitoring/chartData";
import type { MonitoringHistorySample } from "@/stores/monitoringStore";
import type {
  CpuMetrics,
  FilesystemMetrics,
  MemoryMetrics,
  MetricStatus,
  MonitoringSnapshot,
} from "@/types/contracts";

const sampledAt = 1_762_665_720_000;
const intervalMs = 30_000;

const cpuData: CpuMetrics = {
  load1m: 0.42,
  load5m: 0.31,
  load15m: 0.28,
  logicalCpus: 8,
};

const memoryData: MemoryMetrics = {
  totalBytes: 8 * 1024 ** 3,
  usedBytes: 2 * 1024 ** 3,
  availableBytes: 6 * 1024 ** 3,
  usedPercent: 25,
};

const rootFilesystem: FilesystemMetrics = {
  filesystem: "/dev/vda1",
  mount: "/",
  totalBytes: 100 * 1024 ** 3,
  usedBytes: 40 * 1024 ** 3,
  availableBytes: 60 * 1024 ** 3,
  usedPercent: 40,
};

const dataFilesystem: FilesystemMetrics = {
  filesystem: "/dev/vdb1",
  mount: "/data",
  totalBytes: 500 * 1024 ** 3,
  usedBytes: 100 * 1024 ** 3,
  availableBytes: 400 * 1024 ** 3,
  usedPercent: 20,
};

function snapshotAt(
  timestamp: number,
  options: {
    cpuStatus?: MetricStatus;
    memoryStatus?: MetricStatus;
    filesystems?: FilesystemMetrics[] | null;
    filesystemStatus?: MetricStatus;
    cpu?: Partial<CpuMetrics>;
    memory?: Partial<MemoryMetrics>;
  } = {},
): MonitoringSnapshot {
  const cpuStatus = options.cpuStatus ?? "available";
  const memoryStatus = options.memoryStatus ?? "available";
  const filesystemStatus = options.filesystemStatus ?? "available";
  const filesystems = options.filesystems === undefined ? [rootFilesystem] : options.filesystems;

  return {
    serverId: "server-1",
    sampledAt: timestamp,
    durationMs: 42,
    connectionState: "connected",
    cpu: {
      status: cpuStatus,
      sampledAt: timestamp,
      detail: cpuStatus === "available" ? null : "CPU load unavailable.",
      data: cpuStatus === "available" ? { ...cpuData, ...options.cpu } : null,
    },
    memory: {
      status: memoryStatus,
      sampledAt: timestamp,
      detail: memoryStatus === "available" ? null : "Memory unavailable.",
      data: memoryStatus === "available" ? { ...memoryData, ...options.memory } : null,
    },
    filesystems: {
      status: filesystemStatus,
      sampledAt: timestamp,
      detail: filesystemStatus === "available" ? null : "Filesystems unavailable.",
      data: filesystemStatus === "available" ? filesystems : null,
    },
    network: {
      status: "available",
      sampledAt: timestamp,
      detail: null,
      data: [],
    },
  };
}

function sample(timestamp: number, snapshot: MonitoringSnapshot | null): MonitoringHistorySample {
  return { attemptedAt: timestamp, snapshot };
}

function modelFor(history: MonitoringHistorySample[], metric: "cpu" | "memory" | "disk") {
  const model = buildMonitoringChartModels(history, { intervalMs }).find(
    (candidate) => candidate.metric === metric,
  );
  if (!model) {
    throw new Error(`Missing chart model for ${metric}`);
  }
  return model;
}

const palette: ChartPalette = {
  text: "#151d19",
  mutedText: "#5c6861",
  axis: "#d3dad5",
  grid: "#d3dad5",
  tooltipBackground: "#fbfcfa",
  tooltipBorder: "#d3dad5",
  tooltipText: "#151d19",
  cpu: "#c2410c",
  memory: "#1f5f8b",
  disk: "#236b34",
  fontFamily: "IBM Plex Sans",
};

type OptionShape = {
  animation: boolean;
  aria: { enabled: boolean };
  xAxis: {
    name: string;
    min: number;
    max: number;
    axisLabel: { formatter: (value: number) => string };
  };
  yAxis: {
    name: string;
    max?: number;
    axisLabel: { formatter: (value: number) => string };
  };
  series: Array<{
    data: Array<[number, number | null]>;
    connectNulls: boolean;
    showSymbol: boolean;
    lineStyle: { color: string };
  }>;
};

function optionFor(history: MonitoringHistorySample[], metric: "cpu" | "memory" | "disk") {
  return buildChartOption(modelFor(history, metric), palette) as unknown as OptionShape;
}

describe("monitoring chart data", () => {
  it("charts real CPU, memory, and disk values from snapshot history", () => {
    const history = [
      sample(sampledAt, snapshotAt(sampledAt)),
      sample(
        sampledAt + intervalMs,
        snapshotAt(sampledAt + intervalMs, {
          cpu: { load1m: 1.25 },
          memory: { usedPercent: 62.5 },
          filesystems: [{ ...rootFilesystem, usedPercent: 55.5 }],
        }),
      ),
    ];

    const cpu = modelFor(history, "cpu");
    const memory = modelFor(history, "memory");
    const disk = modelFor(history, "disk");

    expect(cpu.data).toEqual([
      [sampledAt, 0.42],
      [sampledAt + intervalMs, 1.25],
    ]);
    expect(memory.data[1][1]).toBe(62.5);
    expect(disk.data[1][1]).toBe(55.5);
    expect(disk.diskMount).toBe("/");
    expect(cpu.validSamples).toBe(2);
    expect(cpu.latestValue).toBe(1.25);
    expect(cpu.latestAt).toBe(sampledAt + intervalMs);
  });

  it("keeps failed refresh attempts as explicit gaps", () => {
    const history = [
      sample(sampledAt, snapshotAt(sampledAt)),
      sample(sampledAt + intervalMs, null),
      sample(sampledAt + intervalMs * 2, snapshotAt(sampledAt + intervalMs * 2)),
    ];

    const cpu = modelFor(history, "cpu");
    const option = optionFor(history, "cpu");

    expect(cpu.data).toEqual([
      [sampledAt, 0.42],
      [sampledAt + intervalMs, null],
      [sampledAt + intervalMs * 2, 0.42],
    ]);
    expect(cpu.missingSamples).toBe(1);
    expect(cpu.validSamples).toBe(2);
    expect(option.series[0].connectNulls).toBe(false);
  });

  it("inserts an empty point when successful samples are further apart than the polling cadence", () => {
    const history = [
      sample(sampledAt, snapshotAt(sampledAt)),
      sample(sampledAt + intervalMs * 4, snapshotAt(sampledAt + intervalMs * 4)),
    ];

    const cpu = modelFor(history, "cpu");

    expect(cpu.data).toHaveLength(3);
    expect(cpu.data[1][1]).toBeNull();
    expect(cpu.data[1][0]).toBeGreaterThan(sampledAt);
    expect(cpu.data[1][0]).toBeLessThan(sampledAt + intervalMs * 4);
    expect(cpu.gapBreaks).toBe(1);
    expect(cpu.validSamples).toBe(2);
  });

  it("does not invent a point for a single sparse sample", () => {
    const history = [sample(sampledAt, snapshotAt(sampledAt))];

    const cpu = modelFor(history, "cpu");
    const option = optionFor(history, "cpu");

    expect(cpu.data).toEqual([[sampledAt, 0.42]]);
    expect(cpu.gapBreaks).toBe(0);
    expect(option.series[0].showSymbol).toBe(true);
    expect(option.xAxis.min).toBeLessThan(sampledAt);
    expect(option.xAxis.max).toBeGreaterThan(sampledAt);
  });

  it("marks unavailable metric groups as null instead of zero", () => {
    const history = [
      sample(sampledAt, snapshotAt(sampledAt, { memoryStatus: "unavailable" })),
      sample(
        sampledAt + intervalMs,
        snapshotAt(sampledAt + intervalMs, { memoryStatus: "unavailable" }),
      ),
    ];

    const memory = modelFor(history, "memory");
    const cpu = modelFor(history, "cpu");

    expect(memory.data).toEqual([
      [sampledAt, null],
      [sampledAt + intervalMs, null],
    ]);
    expect(memory.validSamples).toBe(0);
    expect(memory.latestValue).toBeNull();
    expect(cpu.validSamples).toBe(2);
  });

  it("handles snapshots without any filesystem data", () => {
    const history = [
      sample(sampledAt, snapshotAt(sampledAt, { filesystems: null })),
      sample(
        sampledAt + intervalMs,
        snapshotAt(sampledAt + intervalMs, { filesystemStatus: "unsupported", filesystems: null }),
      ),
    ];

    const disk = modelFor(history, "disk");

    expect(disk.diskMount).toBeNull();
    expect(disk.data.every((point) => point[1] === null)).toBe(true);
    expect(disk.validSamples).toBe(0);
  });

  it("keeps one stable filesystem for the disk chart", () => {
    const noRoot = [dataFilesystem];
    const history = [
      sample(sampledAt, snapshotAt(sampledAt, { filesystems: noRoot })),
      sample(
        sampledAt + intervalMs,
        snapshotAt(sampledAt + intervalMs, { filesystems: [rootFilesystem, dataFilesystem] }),
      ),
    ];

    expect(selectDiskMount(history)).toBe("/");
    expect(modelFor(history, "disk").data).toEqual([
      [sampledAt, null],
      [sampledAt + intervalMs, 40],
    ]);
  });

  it("selects the largest filesystem when no root mount is present", () => {
    const history = [sample(sampledAt, snapshotAt(sampledAt, { filesystems: [dataFilesystem] }))];

    expect(selectDiskMount(history)).toBe("/data");
  });

  it("deduplicates repeated backend sample timestamps", () => {
    const history = [
      sample(sampledAt, snapshotAt(sampledAt)),
      sample(sampledAt, snapshotAt(sampledAt, { cpu: { load1m: 0.5 } })),
    ];

    const cpu = modelFor(history, "cpu");
    expect(cpu.data).toEqual([[sampledAt, 0.5]]);
    expect(cpu.validSamples).toBe(1);
  });

  it("builds readable axis units and time labels", () => {
    const history = [
      sample(sampledAt, snapshotAt(sampledAt)),
      sample(sampledAt + intervalMs, snapshotAt(sampledAt + intervalMs)),
    ];

    const cpu = optionFor(history, "cpu");
    const memory = optionFor(history, "memory");
    const disk = optionFor(history, "disk");

    expect(cpu.xAxis.name).toBe("Sample time");
    expect(cpu.yAxis.name).toBe("Load average");
    expect(memory.yAxis.name).toBe("Memory used (%)");
    expect(disk.yAxis.name).toBe("Disk used (%)");
    expect(memory.yAxis.max).toBe(100);
    expect(memory.yAxis.axisLabel.formatter(25)).toBe("25%");
    expect(cpu.yAxis.axisLabel.formatter(1.5)).toBe("1.5");
    expect(cpu.xAxis.axisLabel.formatter(sampledAt)).toMatch(/\d{1,2}:\d{2}:\d{2}/);
    expect(formatAxisTimestamp(sampledAt, 2 * 60 * 60 * 1000)).toMatch(/\d{1,2}:\d{2}/);
    expect(cpu.series[0].lineStyle.color).toBe(palette.cpu);
    expect(cpu.animation).toBe(false);
    expect(cpu.aria.enabled).toBe(true);
  });

  it("returns the latest successful sample and ignores trailing gaps", () => {
    const history = [
      sample(sampledAt, snapshotAt(sampledAt)),
      sample(sampledAt + intervalMs, snapshotAt(sampledAt + intervalMs)),
      sample(sampledAt + intervalMs * 2, null),
    ];

    expect(latestSuccessfulSample(history)?.snapshot?.sampledAt).toBe(sampledAt + intervalMs);
    expect(latestSuccessfulSample([sample(sampledAt, null)])).toBeNull();
  });

  it("uses the dark palette passed by the wrapper", () => {
    const history = [sample(sampledAt, snapshotAt(sampledAt))];
    const dark = {
      ...palette,
      text: "#e6ece8",
      cpu: "#f08a4b",
    };

    const option = buildChartOption(modelFor(history, "cpu"), dark) as unknown as OptionShape;

    expect(option.series[0].lineStyle.color).toBe("#f08a4b");
  });
});
