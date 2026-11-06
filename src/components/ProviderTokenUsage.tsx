import { useEffect, useState } from "react";
import { LoaderCircle, RefreshCw } from "lucide-react";
import { invokeCommand } from "../lib/tauri";
import type {
  ProviderTokenUsageBucket,
  ProviderTokenUsageBucketSize,
  ProviderTokenUsageResponse,
} from "../types/contracts";

interface ProviderTokenUsageProps {
  providerId: string;
  providerName: string;
}

interface TimeRange {
  fromMs: number;
  toMs: number;
}

const PERIOD_OPTIONS: ReadonlyArray<{ value: ProviderTokenUsageBucketSize; label: string }> = [
  { value: "month", label: "月" },
  { value: "week", label: "周" },
  { value: "day", label: "日" },
  { value: "hour", label: "小时" },
];

const CHART_WIDTH = 900;
const CHART_HEIGHT = 248;
const CHART_PADDING = { top: 16, right: 14, bottom: 42, left: 62 };
const IO_SERIES = [
  { key: "inputTokens", label: "输入", color: "#7d9a82" },
  { key: "outputTokens", label: "输出", color: "#c78d73" },
] as const;
const MODEL_COLORS = ["#718e77", "#c08c68", "#7895aa", "#b18191", "#9a945c", "#8173a5", "#689c93", "#aa7955"];

function createRange(bucket: ProviderTokenUsageBucketSize, now: Date): TimeRange {
  const end = new Date(now.getTime());
  const start = new Date(now.getTime());

  if (bucket === "hour") {
    start.setMinutes(0, 0, 0);
    start.setHours(start.getHours() - 23);
  } else if (bucket === "day") {
    start.setHours(0, 0, 0, 0);
    start.setDate(start.getDate() - 29);
  } else if (bucket === "week") {
    start.setHours(0, 0, 0, 0);
    const daysSinceMonday = (start.getDay() + 6) % 7;
    start.setDate(start.getDate() - daysSinceMonday - 77);
  } else {
    start.setFullYear(start.getFullYear(), start.getMonth() - 11, 1);
    start.setHours(0, 0, 0, 0);
  }

  return { fromMs: start.getTime(), toMs: end.getTime() };
}

function formatDateTime(timestampMs: number): string {
  return new Intl.DateTimeFormat("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  }).format(new Date(timestampMs));
}

function formatDate(timestampMs: number): string {
  return new Intl.DateTimeFormat("zh-CN", { month: "numeric", day: "numeric" }).format(new Date(timestampMs));
}

function formatAxisLabel(bucket: ProviderTokenUsageBucket, period: ProviderTokenUsageBucketSize): string {
  const start = new Date(bucket.startMs);
  if (period === "hour") {
    return new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false }).format(start);
  }
  if (period === "month") {
    return start.getFullYear() === new Date().getFullYear()
      ? start.getMonth() + 1 + "月"
      : String(start.getFullYear()).slice(-2) + "/" + (start.getMonth() + 1);
  }
  return formatDate(bucket.startMs);
}

function formatBucketTitle(bucket: ProviderTokenUsageBucket, period: ProviderTokenUsageBucketSize): string {
  const lastIncludedMs = Math.max(bucket.startMs, bucket.endMs - 1);
  if (period === "month") {
    const date = new Date(bucket.startMs);
    return date.getFullYear() + "年" + (date.getMonth() + 1) + "月";
  }
  if (period === "week") {
    return formatDate(bucket.startMs) + "–" + formatDate(lastIncludedMs);
  }
  if (period === "day") return formatDate(bucket.startMs);
  return formatDateTime(bucket.startMs) + "–" + formatDateTime(lastIncludedMs);
}

function formatSummaryTokenCount(value: number | null, unknownCount: number, requestCount: number): string {
  if (requestCount === 0) return "—";
  if (value === null) return "未知";
  const formatted = new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 0 }).format(value);
  return unknownCount > 0 ? "≥" + formatted : formatted;
}

function formatChartRange(bucket: ProviderTokenUsageBucketSize): string {
  switch (bucket) {
    case "hour":
      return "最近24个自然小时 · 含当前小时";
    case "day":
      return "最近30个自然日 · 含今天";
    case "week":
      return "最近12个自然周 · 周一开始，本周统计至今";
    case "month":
      return "最近12个自然月 · 本月统计至今";
  }
}

function getErrorMessage(reason: unknown): string {
  if (reason instanceof Error) return reason.message;
  if (typeof reason === "string") return reason;
  return "暂时无法读取 Token 用量";
}

interface ChartSeries {
  key: string;
  label: string;
  color: string;
  values: number[];
  unknownCounts: number[];
}

function Chart({
  response,
  period,
  selectedStartMs,
  onSelectBucket,
}: {
  response: ProviderTokenUsageResponse;
  period: ProviderTokenUsageBucketSize;
  selectedStartMs: number | null;
  onSelectBucket: (bucket: ProviderTokenUsageBucket) => void;
}) {
  const points = response.buckets;
  const plotWidth = CHART_WIDTH - CHART_PADDING.left - CHART_PADDING.right;
  const plotHeight = CHART_HEIGHT - CHART_PADDING.top - CHART_PADDING.bottom;
  const series: ChartSeries[] = response.modelId === null
    ? response.models.map((modelId, modelIndex) => {
      const model = response.modelBuckets.find((item) => item.modelId === modelId);
      return {
        key: modelId,
        label: modelId,
        color: MODEL_COLORS[modelIndex % MODEL_COLORS.length] ?? "#718e77",
        values: points.map((point) => {
          const stats = model?.buckets.find((item) => item.startMs === point.startMs);
          return (stats?.inputTokens ?? 0) + (stats?.outputTokens ?? 0);
        }),
        unknownCounts: points.map((point) => {
          const stats = model?.buckets.find((item) => item.startMs === point.startMs);
          return (stats?.inputUnknownCount ?? 0) + (stats?.outputUnknownCount ?? 0);
        }),
      };
    })
    : IO_SERIES.map((item) => ({
      ...item,
      values: points.map((point) => point[item.key] ?? 0),
      unknownCounts: points.map((point) => item.key === "inputTokens" ? point.inputUnknownCount : point.outputUnknownCount),
    }));
  const maxValue = Math.max(1, ...points.map((_, pointIndex) => series.reduce((sum, item) => sum + (item.values[pointIndex] ?? 0), 0)));
  const groupWidth = points.length > 0 ? plotWidth / points.length : plotWidth;
  const barWidth = Math.max(3, Math.min(22, groupWidth * 0.58));
  const labelStep = Math.max(1, Math.ceil(points.length / 8));
  const ticks = [0, maxValue / 2, maxValue];
  const numberFormat = new Intl.NumberFormat("zh-CN");

  return (
    <div className="provider-usage-chart-wrap">
      <svg
        className="provider-usage-chart"
        viewBox={"0 0 " + CHART_WIDTH + " " + CHART_HEIGHT}
        role="group"
        aria-label={formatChartRange(period) + (response.modelId === null ? "按模型" : "按输入和输出") + "堆叠的可点击 Token 柱状图"}
        preserveAspectRatio="none"
      >
        {ticks.map((tick, index) => {
          const y = CHART_PADDING.top + plotHeight - (tick / maxValue) * plotHeight;
          return (
            <g key={index}>
              <line className="provider-usage-grid-line" x1={CHART_PADDING.left} y1={y} x2={CHART_WIDTH - CHART_PADDING.right} y2={y} />
              <text className="provider-usage-axis-label" x={CHART_PADDING.left - 9} y={y + 4} textAnchor="end">{new Intl.NumberFormat("zh-CN", { notation: "compact", maximumFractionDigits: 1 }).format(tick)}</text>
            </g>
          );
        })}
        {points.map((point, pointIndex) => {
          const x = CHART_PADDING.left + pointIndex * groupWidth + (groupWidth - barWidth) / 2;
          const showLabel = pointIndex % labelStep === 0 || pointIndex === points.length - 1;
          let stackOffset = 0;
          const total = series.reduce((sum, item) => sum + (item.values[pointIndex] ?? 0), 0);
          const unknownCount = series.reduce((sum, item) => sum + (item.unknownCounts[pointIndex] ?? 0), 0);
          const isSelected = point.startMs === selectedStartMs;
          const bucketStartMs = Math.max(point.startMs, response.fromMs);
          const selectedEndMs = Math.min(point.endMs, response.toMs);
          return (
            <g
              key={point.startMs}
              className={"provider-usage-chart-column" + (isSelected ? " is-selected" : "")}
              role="button"
              tabIndex={0}
              aria-pressed={isSelected}
              aria-label={formatBucketTitle(point, period) + `：${numberFormat.format(total)} Token，${unknownCount} 项用量未知，点击查看详情`}
              onClick={() => onSelectBucket({ ...point, startMs: bucketStartMs, endMs: selectedEndMs })}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  onSelectBucket({ ...point, startMs: bucketStartMs, endMs: selectedEndMs });
                }
              }}
            >
              <rect className="provider-usage-column-hitbox" x={CHART_PADDING.left + pointIndex * groupWidth} y={CHART_PADDING.top} width={groupWidth} height={plotHeight} />
              {series.map((item) => {
                const value = item.values[pointIndex] ?? 0;
                const barHeight = value > 0 ? Math.max(1, (value / maxValue) * plotHeight) : 0;
                stackOffset += barHeight;
                if (value <= 0) return null;
                const y = CHART_PADDING.top + plotHeight - stackOffset;
                return (
                  <rect
                    key={item.key}
                    className="provider-usage-chart-bar"
                    x={x}
                    y={y}
                    width={barWidth}
                    height={barHeight}
                    fill={item.color}
                  >
                    <title>{formatBucketTitle(point, period) + " · " + item.label + "：" + numberFormat.format(value) + " tokens" + ((item.unknownCounts[pointIndex] ?? 0) > 0 ? "（已知小计）" : "")}</title>
                  </rect>
                );
              })}
              {isSelected && <rect className="provider-usage-column-outline" x={x - 2} y={CHART_PADDING.top + plotHeight - Math.max(1, (total / maxValue) * plotHeight) - 2} width={barWidth + 4} height={Math.max(3, (total / maxValue) * plotHeight + 4)} />}
              {showLabel && (
                <text className="provider-usage-axis-label provider-usage-x-label" x={CHART_PADDING.left + pointIndex * groupWidth + groupWidth / 2} y={CHART_HEIGHT - 14} textAnchor="middle">
                  {formatAxisLabel(point, period)}
                </text>
              )}
            </g>
          );
        })}
      </svg>
    </div>
  );
}

function SummaryMetric({
  label,
  value,
  description,
  className = "",
}: {
  label: string;
  value: string;
  description: string;
  className?: string;
}) {
  return (
    <div className={"provider-usage-metric " + className}>
      <span>{label}</span>
      <strong>{value}</strong>
      <small>{description}</small>
    </div>
  );
}

export function ProviderTokenUsage({ providerId, providerName }: ProviderTokenUsageProps) {
  const [period, setPeriod] = useState<ProviderTokenUsageBucketSize>("day");
  const [modelId, setModelId] = useState<string | null>(null);
  const [selectedBucket, setSelectedBucket] = useState<ProviderTokenUsageBucket | null>(null);
  const [availableModels, setAvailableModels] = useState<string[]>([]);
  const [refreshNowMs, setRefreshNowMs] = useState(() => Date.now());
  const [chartResponse, setChartResponse] = useState<ProviderTokenUsageResponse | null>(null);
  const [chartLoading, setChartLoading] = useState(true);
  const [chartError, setChartError] = useState<string | null>(null);
  const [summaryResponse, setSummaryResponse] = useState<ProviderTokenUsageResponse | null>(null);
  const [summaryLoading, setSummaryLoading] = useState(true);
  const [summaryError, setSummaryError] = useState<string | null>(null);
  const chartRange = createRange(period, new Date(refreshNowMs));
  const summaryFromMs = selectedBucket?.startMs ?? refreshNowMs - 24 * 60 * 60 * 1000;
  const summaryToMs = selectedBucket?.endMs ?? refreshNowMs;
  const summaryBucket = selectedBucket ? period : "hour";

  useEffect(() => {
    setModelId(null);
    setSelectedBucket(null);
    setAvailableModels([]);
  }, [providerId]);

  useEffect(() => {
    let active = true;
    setSummaryError(null);
    setSummaryLoading(true);
    void invokeCommand<ProviderTokenUsageResponse>("get_provider_token_usage", {
      providerId,
      modelId,
      fromMs: summaryFromMs,
      toMs: summaryToMs,
      bucket: summaryBucket,
    }).then((response) => {
      if (active && response.providerId === providerId && response.modelId === modelId) {
        setSummaryResponse(response);
        setAvailableModels(response.models);
      }
    }).catch((reason: unknown) => {
      if (active) setSummaryError(getErrorMessage(reason));
    }).finally(() => {
      if (active) setSummaryLoading(false);
    });
    return () => { active = false; };
  }, [modelId, providerId, refreshNowMs, summaryBucket, summaryFromMs, summaryToMs]);

  useEffect(() => {
    let active = true;
    setChartError(null);
    setChartLoading(true);
    void invokeCommand<ProviderTokenUsageResponse>("get_provider_token_usage", {
      providerId,
      modelId,
      fromMs: chartRange.fromMs,
      toMs: chartRange.toMs,
      bucket: period,
    }).then((response) => {
      if (active && response.providerId === providerId && response.modelId === modelId) {
        setChartResponse(response);
        setAvailableModels(response.models);
      }
    }).catch((reason: unknown) => {
      if (active) setChartError(getErrorMessage(reason));
    }).finally(() => {
      if (active) setChartLoading(false);
    });
    return () => { active = false; };
  }, [chartRange.fromMs, chartRange.toMs, modelId, period, providerId]);

  const currentSummaryResponse = summaryResponse?.providerId === providerId
    && summaryResponse.modelId === modelId
    && summaryResponse.fromMs === summaryFromMs
    && summaryResponse.toMs === summaryToMs
    && summaryResponse.bucket === summaryBucket
    ? summaryResponse
    : null;
  const currentChartResponse = chartResponse?.providerId === providerId
    && chartResponse.modelId === modelId
    && chartResponse.bucket === period
    && chartResponse.fromMs === chartRange.fromMs
    && chartResponse.toMs === chartRange.toMs
    ? chartResponse
    : null;
  const previousSummaryResponse = !currentSummaryResponse && summaryResponse?.providerId === providerId ? summaryResponse : null;
  const previousChartResponse = !currentChartResponse && chartResponse?.providerId === providerId ? chartResponse : null;
  const isSummaryLoading = summaryLoading || (!currentSummaryResponse && !summaryError);
  const isChartLoading = chartLoading || (!currentChartResponse && !chartError);
  const summaryForDisplay = currentSummaryResponse ?? previousSummaryResponse;
  const summary = summaryForDisplay?.totals;
  const summaryRangeLabel = selectedBucket
    ? formatBucketTitle(selectedBucket, period) + " · " + formatDateTime(summaryFromMs) + " 至 " + formatDateTime(summaryToMs)
    : summaryForDisplay
      ? formatDateTime(summaryForDisplay.fromMs) + " 至 " + formatDateTime(summaryForDisplay.toMs)
      : "滚动24小时";
  const chartRequestCount = currentChartResponse?.totals.requestCount ?? 0;

  return (
    <section className="provider-token-usage" aria-labelledby="provider-token-usage-heading">
      <div className="provider-usage-header">
        <div>
          <h3 id="provider-token-usage-heading">Token 用量</h3>
          <p>{providerName} · {modelId ?? "全部模型"}</p>
        </div>
        <button
          className="provider-usage-refresh"
          type="button"
          onClick={() => setRefreshNowMs((value) => Math.max(Date.now(), value + 1))}
          disabled={isSummaryLoading && isChartLoading}
          aria-label="刷新 Token 用量"
          title="刷新"
        >
          {isSummaryLoading && isChartLoading ? <LoaderCircle size={15} className="provider-usage-spinner" aria-hidden="true" /> : <RefreshCw size={15} aria-hidden="true" />}
        </button>
      </div>

      <div className="provider-usage-model-filter">
        <label htmlFor="provider-usage-model">模型</label>
        <select
          id="provider-usage-model"
          value={modelId ?? ""}
          onChange={(event) => {
            setSelectedBucket(null);
            setModelId(event.target.value || null);
          }}
          disabled={availableModels.length === 0 && (summaryLoading || chartLoading)}
        >
          <option value="">全部模型</option>
          {availableModels.map((model) => <option key={model} value={model}>{model}</option>)}
        </select>
      </div>

      <div className="provider-usage-summary-heading">
        <div>
          <strong>{selectedBucket ? formatBucketTitle(selectedBucket, period) : "最近24小时"}</strong>
          <span>{summaryRangeLabel} · 本地时间</span>
        </div>
        {summary && <small>{new Intl.NumberFormat("zh-CN").format(summary.requestCount)} 次请求{isSummaryLoading ? " · 更新中" : ""}</small>}
      </div>

      {summaryError ? (
        <div className="provider-usage-error" role="alert">用量摘要读取失败：{summaryError}</div>
      ) : (
        <div className={isSummaryLoading && previousSummaryResponse ? "provider-usage-summary is-refreshing" : "provider-usage-summary"} aria-live="polite" aria-busy={isSummaryLoading}>
          <SummaryMetric
            label="输入"
            value={isSummaryLoading && !summary ? "…" : formatSummaryTokenCount(summary?.inputTokens ?? null, summary?.inputUnknownCount ?? 0, summary?.requestCount ?? 0)}
            description={summary?.inputUnknownCount ? "已知输入小计" : "输入 Token"}
            className="is-input"
          />
          <SummaryMetric
            label="输出"
            value={isSummaryLoading && !summary ? "…" : formatSummaryTokenCount(summary?.outputTokens ?? null, summary?.outputUnknownCount ?? 0, summary?.requestCount ?? 0)}
            description={summary?.outputUnknownCount ? "已知输出小计" : "输出 Token"}
            className="is-output"
          />
        </div>
      )}
      {!isSummaryLoading && summary && summary.inputUnknownCount > 0 && (
        <p className="provider-usage-note" role="note">{new Intl.NumberFormat("zh-CN").format(summary.inputUnknownCount)} 次请求缺少输入 Token，输入显示已知小计</p>
      )}
      {!isSummaryLoading && summary && summary.outputUnknownCount > 0 && (
        <p className="provider-usage-note" role="note">{new Intl.NumberFormat("zh-CN").format(summary.outputUnknownCount)} 次请求缺少输出 Token，输出值为已知小计</p>
      )}

      <div className="provider-usage-trend-heading">
        <div>
          <strong>用量趋势</strong>
          <span>{formatChartRange(period)}</span>
        </div>
        <div className="provider-usage-periods" role="group" aria-label="用量趋势时间粒度">
          {PERIOD_OPTIONS.map((option) => (
            <button
              type="button"
              key={option.value}
              className={period === option.value ? "is-active" : ""}
              aria-pressed={period === option.value}
              onClick={() => {
                setSelectedBucket(null);
                setPeriod(option.value);
              }}
            >
              {option.label}
            </button>
          ))}
        </div>
      </div>

      <div className="provider-usage-legend" aria-hidden="true">
        {modelId === null
          ? availableModels.map((model, index) => <span key={model}><i style={{ backgroundColor: MODEL_COLORS[index % MODEL_COLORS.length] }} />{model}</span>)
          : IO_SERIES.map((series) => <span key={series.key}><i style={{ backgroundColor: series.color }} />{series.label}</span>)}
      </div>
      {chartError ? (
        <div className="provider-usage-error" role="alert">趋势读取失败：{chartError}</div>
      ) : isChartLoading && !previousChartResponse ? (
        <div className="provider-usage-loading" role="status"><LoaderCircle size={16} className="provider-usage-spinner" aria-hidden="true" />正在读取用量趋势</div>
      ) : currentChartResponse && chartRequestCount === 0 ? (
        <div className="provider-usage-empty">所选时间范围内暂无调用记录</div>
      ) : currentChartResponse || previousChartResponse ? (
        <div className="provider-usage-chart-stage" aria-busy={isChartLoading}>
          <div className={isChartLoading && previousChartResponse ? "provider-usage-chart-content is-refreshing" : "provider-usage-chart-content"}>
            <Chart
              response={currentChartResponse ?? previousChartResponse!}
              period={(currentChartResponse ?? previousChartResponse!).bucket}
              selectedStartMs={selectedBucket?.startMs ?? null}
              onSelectBucket={setSelectedBucket}
            />
            <p className="provider-usage-chart-caption">
              {modelId === null ? "按模型堆叠输入 + 输出" : "按输入、输出堆叠"} · 点击柱查看该时间段的输入、输出和请求数
            </p>
          </div>
          {isChartLoading && previousChartResponse && <div className="provider-usage-refresh-overlay" role="status"><LoaderCircle size={15} className="provider-usage-spinner" aria-hidden="true" />正在更新所选趋势</div>}
        </div>
      ) : null}
      {!isChartLoading && currentChartResponse && currentChartResponse.totals.inputUnknownCount > 0 && (
        <p className="provider-usage-note" role="note">趋势中输入仅显示已知小计；{new Intl.NumberFormat("zh-CN").format(currentChartResponse.totals.inputUnknownCount)} 次请求缺少输入用量</p>
      )}
      {!isChartLoading && currentChartResponse && currentChartResponse.totals.outputUnknownCount > 0 && (
        <p className="provider-usage-note" role="note">趋势中输出仅显示已知小计</p>
      )}
    </section>
  );
}
