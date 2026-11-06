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
const CHART_SERIES = [
  { key: "inputTokens", label: "输入（含 CR）", color: "#7d9a82" },
  { key: "cacheReadTokens", label: "CR", color: "#b0b978" },
  { key: "outputTokens", label: "输出", color: "#c78d73" },
] as const;

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

function formatTokenCount(value: number | null, unknownCount = 0): string {
  if (value === null) return unknownCount > 0 ? "未知" : "—";
  const formatted = new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 0 }).format(value);
  return unknownCount > 0 ? "≥" + formatted : formatted;
}

function formatSummaryTokenCount(value: number | null, unknownCount: number, requestCount: number): string {
  if (requestCount === 0) return "—";
  return formatTokenCount(value, unknownCount);
}

function formatRate(cacheKnownReadTokens: number, cacheKnownInputTokens: number): string {
  if (cacheKnownInputTokens <= 0) return "—";
  const rate = (cacheKnownReadTokens / cacheKnownInputTokens) * 100;
  return new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 1 }).format(rate) + "%";
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

function Chart({ response, period }: { response: ProviderTokenUsageResponse; period: ProviderTokenUsageBucketSize }) {
  const points = response.buckets;
  const plotWidth = CHART_WIDTH - CHART_PADDING.left - CHART_PADDING.right;
  const plotHeight = CHART_HEIGHT - CHART_PADDING.top - CHART_PADDING.bottom;
  const knownValues = points.flatMap((point) => CHART_SERIES.map((series) => point[series.key]).filter((value): value is number => value !== null));
  const maxValue = Math.max(1, ...knownValues);
  const groupWidth = points.length > 0 ? plotWidth / points.length : plotWidth;
  const barWidth = Math.max(2, Math.min(10, groupWidth * 0.18));
  const barGap = Math.max(1, Math.min(4, groupWidth * 0.035));
  const groupBarsWidth = CHART_SERIES.length * barWidth + (CHART_SERIES.length - 1) * barGap;
  const labelStep = Math.max(1, Math.ceil(points.length / 8));
  const ticks = [0, maxValue / 2, maxValue];

  return (
    <div className="provider-usage-chart-wrap">
      <svg
        className="provider-usage-chart"
        viewBox={"0 0 " + CHART_WIDTH + " " + CHART_HEIGHT}
        role="img"
        aria-label={formatChartRange(period) + "的输入、CR 和输出 Token 柱状图"}
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
        {points.map((point, index) => {
          const groupX = CHART_PADDING.left + index * groupWidth + (groupWidth - groupBarsWidth) / 2;
          const xLabelY = CHART_HEIGHT - 14;
          const showLabel = index % labelStep === 0 || index === points.length - 1;
          return (
            <g key={point.startMs}>
              {CHART_SERIES.map((series, seriesIndex) => {
                const value = point[series.key];
                if (value === null) return null;
                const barHeight = value > 0 ? Math.max(1, (value / maxValue) * plotHeight) : 0;
                const x = groupX + seriesIndex * (barWidth + barGap);
                const y = CHART_PADDING.top + plotHeight - barHeight;
                return (
                  <rect
                    key={series.key}
                    className="provider-usage-chart-bar"
                    x={x}
                    y={y}
                    width={barWidth}
                    height={barHeight}
                    rx={Math.min(2, barWidth / 2)}
                    fill={series.color}
                  >
                    <title>{formatBucketTitle(point, period) + " · " + series.label + "：" + new Intl.NumberFormat("zh-CN").format(value) + " tokens"}</title>
                  </rect>
                );
              })}
              {showLabel && (
                <text className="provider-usage-axis-label provider-usage-x-label" x={CHART_PADDING.left + index * groupWidth + groupWidth / 2} y={xLabelY} textAnchor="middle">
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
  const [availableModels, setAvailableModels] = useState<string[]>([]);
  const [refreshNowMs, setRefreshNowMs] = useState(() => Date.now());
  const [chartResponse, setChartResponse] = useState<ProviderTokenUsageResponse | null>(null);
  const [chartLoading, setChartLoading] = useState(true);
  const [chartError, setChartError] = useState<string | null>(null);
  const [summaryResponse, setSummaryResponse] = useState<ProviderTokenUsageResponse | null>(null);
  const [summaryLoading, setSummaryLoading] = useState(true);
  const [summaryError, setSummaryError] = useState<string | null>(null);
  const chartRange = createRange(period, new Date(refreshNowMs));

  useEffect(() => {
    setModelId(null);
    setAvailableModels([]);
  }, [providerId]);

  useEffect(() => {
    let active = true;
    const now = refreshNowMs;
    const fromMs = now - 24 * 60 * 60 * 1000;
    setSummaryError(null);
    setSummaryLoading(true);
    void invokeCommand<ProviderTokenUsageResponse>("get_provider_token_usage", {
      providerId,
      modelId,
      fromMs,
      toMs: now,
      bucket: "hour",
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
  }, [modelId, providerId, refreshNowMs]);

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

  const currentSummaryResponse = summaryResponse?.providerId === providerId && summaryResponse.modelId === modelId ? summaryResponse : null;
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
  const summaryRangeLabel = summaryForDisplay
    ? formatDateTime(summaryForDisplay.fromMs) + " 至 " + formatDateTime(summaryForDisplay.toMs)
    : "滚动24小时";
  const chartRequestCount = currentChartResponse?.totals.requestCount ?? 0;
  const summaryCacheRate = formatRate(summary?.cacheKnownReadTokens ?? 0, summary?.cacheKnownInputTokens ?? 0);
  const hasPairedCacheDetails = (summary?.cachePairedRequestCount ?? 0) > 0;
  const uncachedInputTokens = hasPairedCacheDetails && summary
    ? summary.cacheKnownInputTokens - summary.cacheKnownReadTokens
    : null;
  const uncachedInputUnknownCount = summary && (summary.inputUnknownCount > 0 || summary.cacheDetailUnknownCount > 0) ? 1 : 0;

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
          onChange={(event) => setModelId(event.target.value || null)}
          disabled={availableModels.length === 0 && (summaryLoading || chartLoading)}
        >
          <option value="">全部模型</option>
          {availableModels.map((model) => <option key={model} value={model}>{model}</option>)}
        </select>
      </div>

      <div className="provider-usage-summary-heading">
        <div>
          <strong>最近24小时</strong>
          <span>{summaryRangeLabel} · 本地时间</span>
        </div>
        {summary && <small>{new Intl.NumberFormat("zh-CN").format(summary.requestCount)} 次请求{isSummaryLoading ? " · 更新中" : ""}</small>}
      </div>

      {summaryError ? (
        <div className="provider-usage-error" role="alert">用量摘要读取失败：{summaryError}</div>
      ) : (
        <div className={isSummaryLoading && previousSummaryResponse ? "provider-usage-summary is-refreshing" : "provider-usage-summary"} aria-live="polite" aria-busy={isSummaryLoading}>
          <SummaryMetric
            label="CR"
            value={isSummaryLoading && !summary ? "…" : formatSummaryTokenCount(summary?.cacheReadTokens ?? null, summary?.cacheDetailUnknownCount ?? 0, summary?.requestCount ?? 0)}
            description={summary?.cacheReadTokens === null && (summary?.cacheDetailUnknownCount ?? 0) > 0
              ? "Provider 未返回缓存明细"
              : summary?.cacheDetailUnknownCount ? "已知 CR 小计" : "缓存读取 Token"}
            className="is-cache"
          />
          <SummaryMetric
            label="输入"
            value={isSummaryLoading && !summary ? "…" : formatSummaryTokenCount(summary?.inputTokens ?? null, summary?.inputUnknownCount ?? 0, summary?.requestCount ?? 0)}
            description={summary?.inputUnknownCount ? "已知输入小计，含 CR" : "API 总输入，含 CR"}
            className="is-input"
          />
          {(!isSummaryLoading || previousSummaryResponse) && hasPairedCacheDetails && (
            <SummaryMetric
              label="非缓存输入"
              value={formatTokenCount(uncachedInputTokens, uncachedInputUnknownCount)}
              description={uncachedInputUnknownCount > 0 ? "已知配对明细小计" : "已知输入 − CR"}
              className="is-uncached-input"
            />
          )}
          <SummaryMetric
            label="输出"
            value={isSummaryLoading && !summary ? "…" : formatSummaryTokenCount(summary?.outputTokens ?? null, summary?.outputUnknownCount ?? 0, summary?.requestCount ?? 0)}
            description="输出 Token"
            className="is-output"
          />
          <SummaryMetric
            label="缓存率"
            value={isSummaryLoading && !summary ? "…" : summaryCacheRate}
            description={summary?.cacheDetailUnknownCount || summary?.inputUnknownCount ? "按两项明细均已知的请求计算" : "CR ÷ 已知输入"}
            className="is-rate"
          />
        </div>
      )}
      {!isSummaryLoading && summary && summary.cacheDetailUnknownCount > 0 && (
        <p className="provider-usage-note" role="note">
          {new Intl.NumberFormat("zh-CN").format(summary.cacheDetailUnknownCount)} 次请求缺少缓存明细，非缓存输入和缓存率仅按已知配对明细计算
        </p>
      )}
      {!isSummaryLoading && summary && summary.inputUnknownCount > 0 && (
        <p className="provider-usage-note" role="note">{new Intl.NumberFormat("zh-CN").format(summary.inputUnknownCount)} 次请求缺少输入 Token，缓存率不计入这些请求</p>
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
              onClick={() => setPeriod(option.value)}
            >
              {option.label}
            </button>
          ))}
        </div>
      </div>

      <div className="provider-usage-legend" aria-hidden="true">
        {CHART_SERIES.map((series) => <span key={series.key}><i style={{ backgroundColor: series.color }} />{series.label}</span>)}
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
            <Chart response={currentChartResponse ?? previousChartResponse!} period={(currentChartResponse ?? previousChartResponse!).bucket} />
            <p className="provider-usage-chart-caption">输入包含 CR，柱形并列展示以便比较 · 数据按本地时间汇总</p>
          </div>
          {isChartLoading && previousChartResponse && <div className="provider-usage-refresh-overlay" role="status"><LoaderCircle size={15} className="provider-usage-spinner" aria-hidden="true" />正在更新所选趋势</div>}
        </div>
      ) : null}
      {!isChartLoading && currentChartResponse && currentChartResponse.totals.cacheDetailUnknownCount > 0 && (
        <p className="provider-usage-note" role="note">
          {new Intl.NumberFormat("zh-CN").format(currentChartResponse.totals.cacheDetailUnknownCount)} 次请求缺少缓存明细，趋势中的 CR 仅统计已知明细
        </p>
      )}
      {!isChartLoading && currentChartResponse && currentChartResponse.totals.inputUnknownCount > 0 && (
        <p className="provider-usage-note" role="note">趋势中输入仅显示已知小计；{new Intl.NumberFormat("zh-CN").format(currentChartResponse.totals.inputUnknownCount)} 次请求不计入缓存率</p>
      )}
      {!isChartLoading && currentChartResponse && currentChartResponse.totals.outputUnknownCount > 0 && (
        <p className="provider-usage-note" role="note">趋势中输出仅显示已知小计</p>
      )}
    </section>
  );
}
