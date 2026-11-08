import { useMemo } from "react";
import ReactEChartsCoreModule from "echarts-for-react/lib/core";
import * as echarts from "echarts/core";
import { LineChart } from "echarts/charts";
import { AriaComponent, GridComponent, TooltipComponent } from "echarts/components";
import { CanvasRenderer } from "echarts/renderers";
import { ChartLineIcon } from "@phosphor-icons/react";
import { useResolvedTheme } from "@/hooks/useResolvedTheme";
import { cn } from "@/lib/utils";
import type { MonitoringHistorySample } from "@/stores/monitoringStore";
import {
  buildChartOption,
  buildMonitoringChartModels,
  formatMetricValue,
  formatTooltipTimestamp,
  type ChartPalette,
  type MonitoringChartModel,
} from "@/features/monitoring/chartData";

// The package ships CommonJS; dev and production module interop can wrap the
// class in an extra `default` object.
const ReactEChartsCore =
  (
    ReactEChartsCoreModule as unknown as {
      default?: typeof ReactEChartsCoreModule;
    }
  ).default ?? ReactEChartsCoreModule;

echarts.use([LineChart, AriaComponent, GridComponent, TooltipComponent, CanvasRenderer]);

type MonitoringChartsProps = {
  history: MonitoringHistorySample[];
  intervalSeconds: number;
  stale: boolean;
};

function readCssVariable(name: string, fallback: string) {
  const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return value || fallback;
}

function readPalette(theme: "light" | "dark"): ChartPalette {
  const dark = theme === "dark";

  return {
    text: readCssVariable("--foreground", dark ? "#e6ece8" : "#151d19"),
    mutedText: readCssVariable("--muted-foreground", dark ? "#94a39a" : "#5c6861"),
    axis: readCssVariable("--border", dark ? "#26332c" : "#d3dad5"),
    grid: readCssVariable("--border", dark ? "#26332c" : "#d3dad5"),
    tooltipBackground: readCssVariable("--popover", dark ? "#16211c" : "#fbfcfa"),
    tooltipBorder: readCssVariable("--border", dark ? "#26332c" : "#d3dad5"),
    tooltipText: readCssVariable("--popover-foreground", dark ? "#e6ece8" : "#151d19"),
    cpu: readCssVariable("--primary", dark ? "#f08a4b" : "#c2410c"),
    memory: readCssVariable("--info", dark ? "#6bb6e8" : "#1f5f8b"),
    disk: readCssVariable("--success", dark ? "#5bc46d" : "#236b34"),
    fontFamily:
      getComputedStyle(document.body).fontFamily ||
      '"IBM Plex Sans Variable", ui-sans-serif, system-ui, sans-serif',
  };
}

function ChartCard({
  model,
  option,
  theme,
  stale,
  className,
}: {
  model: MonitoringChartModel;
  option: ReturnType<typeof buildChartOption>;
  theme: "light" | "dark";
  stale: boolean;
  className?: string;
}) {
  const available = model.validSamples > 0;
  const latestTime =
    model.latestAt !== null ? formatTooltipTimestamp(model.latestAt) : "No usable sample";

  return (
    <article
      className={cn(
        "flex min-w-0 flex-col overflow-hidden rounded-md border border-border bg-card",
        className,
      )}
    >
      <header className="flex min-h-[68px] flex-wrap items-start justify-between gap-3 border-b border-border px-3 py-2.5">
        <div className="min-w-0">
          <h2 className="flex items-center gap-1.5 text-[12px] font-semibold text-foreground">
            <ChartLineIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
            {model.title}
            {stale ? <span className="text-[10px] font-medium text-warning">stale</span> : null}
          </h2>
          <p className="mt-0.5 max-w-[46ch] text-[10px] leading-relaxed text-muted-foreground">
            {model.description}
            {model.metric === "disk" && model.diskMount ? ` Mount: ${model.diskMount}.` : null}
          </p>
        </div>
        <div className="min-w-0 shrink-0 text-right">
          <p className="font-mono text-[11px] text-foreground">
            {model.latestValue !== null ? formatMetricValue(model, model.latestValue) : "—"}
          </p>
          <p
            className="mt-0.5 max-w-[210px] truncate text-[10px] text-muted-foreground"
            title={latestTime}
          >
            {latestTime}
          </p>
        </div>
      </header>

      <div className="h-[220px] w-full px-2 pt-2" aria-label={`${model.title} sample history`}>
        {available ? (
          <ReactEChartsCore
            key={`${model.metric}-${theme}`}
            echarts={echarts}
            option={option}
            notMerge
            lazyUpdate
            style={{ height: 212, width: "100%" }}
            opts={{ renderer: "canvas" }}
          />
        ) : (
          <div className="flex h-full items-center justify-center rounded-sm border border-dashed border-border/80 bg-muted/20 px-4 text-center text-[11px] text-muted-foreground">
            No usable {model.metric} samples yet. Failed, paused, or unavailable samples appear as
            gaps rather than invented values.
          </div>
        )}
      </div>

      <footer className="flex flex-wrap items-center gap-x-2 gap-y-1 border-t border-border px-3 py-2 text-[10px] text-muted-foreground">
        <span>
          {model.validSamples} usable sample{model.validSamples === 1 ? "" : "s"}
        </span>
        <span aria-hidden="true">·</span>
        <span>{model.missingSamples} missing or unavailable</span>
        <span aria-hidden="true">·</span>
        <span>
          {model.gapBreaks > 0
            ? `${model.gapBreaks} line gap${model.gapBreaks === 1 ? "" : "s"} not interpolated`
            : "No interpolated values"}
        </span>
      </footer>
    </article>
  );
}

function MonitoringCharts({ history, intervalSeconds, stale }: MonitoringChartsProps) {
  const theme = useResolvedTheme();
  const models = useMemo(
    () =>
      buildMonitoringChartModels(history, {
        intervalMs: Math.max(1, intervalSeconds) * 1000,
      }),
    [history, intervalSeconds],
  );
  const palette = useMemo(() => readPalette(theme), [theme]);
  const charts = useMemo(
    () =>
      models.map((model) => ({
        model,
        option: buildChartOption(model, palette),
      })),
    [models, palette],
  );

  return (
    <div className="grid min-w-0 gap-4 xl:grid-cols-2">
      {charts.map(({ model, option }) => (
        <ChartCard
          key={model.metric}
          model={model}
          option={option}
          theme={theme}
          stale={stale}
          className={model.metric === "disk" ? "xl:col-span-2" : undefined}
        />
      ))}
    </div>
  );
}

export default MonitoringCharts;
