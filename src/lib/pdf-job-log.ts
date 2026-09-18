import type {
  DocumentContext,
  PdfJobEvent,
  PdfJobLogEntry,
  PdfJobLogKind,
  PdfJobLogLevel,
  PdfJobProgress,
  PdfQualityDiagnostic,
} from "../types/contracts";

export const MAX_PDF_JOB_LOG_ENTRIES = 300;

function nextSequence(logs: PdfJobLogEntry[]): number {
  return (logs.at(-1)?.seq ?? 0) + 1;
}

function appendEntry(
  logs: PdfJobLogEntry[],
  kind: PdfJobLogKind,
  level: PdfJobLogLevel,
  message: string,
): PdfJobLogEntry[] {
  const normalizedMessage = message.trim();
  if (!normalizedMessage) return logs;
  const previous = logs.at(-1);
  if (
    previous?.kind === kind &&
    previous.level === level &&
    previous.message === normalizedMessage
  ) {
    return logs;
  }
  return [
    ...logs,
    {
      seq: nextSequence(logs),
      kind,
      level,
      message: normalizedMessage,
    },
  ].slice(-MAX_PDF_JOB_LOG_ENTRIES);
}

export function appendPdfJobLogMessage(
  logs: PdfJobLogEntry[],
  kind: PdfJobLogKind,
  level: PdfJobLogLevel,
  message: string,
): PdfJobLogEntry[] {
  return appendEntry(logs, kind, level, message);
}

function stageLabel(stage: string): string {
  const labels: Record<string, string> = {
    preparing: "准备文档",
    preflight: "文档预检",
    translating: "分段翻译",
    assembling: "组装 PDF",
    completed: "完成",
  };
  return labels[stage] ?? stage;
}

function contextTitle(context: DocumentContext): string {
  return context.title?.trim() || "未识别标题";
}

function formatContextSummary(context: DocumentContext): string[] {
  const metadata = [
    context.documentType ? `类型：${context.documentType}` : null,
    context.domain ? `领域：${context.domain}` : null,
    `术语 ${context.keyTerms.length}`,
    `缩写 ${context.abbreviations.length}`,
    context.headings.length > 0 ? `标题层级 ${context.headings.length}` : null,
  ].filter((item): item is string => item !== null);
  const messages = [
    `文档上下文：${contextTitle(context)}${metadata.length > 0 ? ` · ${metadata.join(" · ")}` : ""}`,
    context.abstract?.trim() ? `摘要：${context.abstract.trim()}` : null,
    context.translationNotes.length > 0 ? `翻译注意事项：${context.translationNotes.join("；")}` : null,
    context.keyTerms.length > 0
      ? `任务术语：${context.keyTerms.map((term) => term.target ? `${term.source} → ${term.target}` : term.source).join("、")}`
      : null,
    context.abbreviations.length > 0
      ? `任务缩写：${context.abbreviations.map((item) => item.expanded ? `${item.abbreviation}（${item.expanded}）` : item.abbreviation).join("、")}`
      : null,
    `上下文 v${context.schemaVersion}${context.contextHash ? ` · ${context.contextHash}` : ""}`,
  ];
  return messages.filter((item): item is string => item !== null);
}

function appendUniqueEntry(
  logs: PdfJobLogEntry[],
  kind: PdfJobLogKind,
  level: PdfJobLogLevel,
  message: string,
): PdfJobLogEntry[] {
  const normalizedMessage = message.trim();
  if (!normalizedMessage) return logs;
  if (logs.some((entry) => entry.kind === kind && entry.level === level && entry.message === normalizedMessage)) {
    return logs;
  }
  return appendEntry(logs, kind, level, normalizedMessage);
}

function appendPreflightWarnings(logs: PdfJobLogEntry[], warnings: string[]): PdfJobLogEntry[] {
  return warnings.reduce(
    (result, warning) => appendUniqueEntry(result, "warning", "warning", warning),
    logs,
  );
}

function appendPreflightContext(logs: PdfJobLogEntry[], context: DocumentContext | null): PdfJobLogEntry[] {
  if (!context) return logs;
  return formatContextSummary(context).reduce(
    (result, message) => appendUniqueEntry(result, "preflight", "info", message),
    logs,
  );
}

function formatProgress(progress: PdfJobProgress): string {
  const label = stageLabel(progress.stage);
  const count = progress.current !== null && progress.total !== null
    ? ` · ${progress.current}/${progress.total}`
    : "";
  return `${label}${count}${progress.message ? ` · ${progress.message}` : ""}`;
}

function appendProgressLog(
  logs: PdfJobLogEntry[],
  progress: PdfJobProgress,
): PdfJobLogEntry[] {
  const message = formatProgress(progress);
  const previous = logs.at(-1);
  const label = stageLabel(progress.stage);
  if (
    previous?.kind === "progress" &&
    previous.level === "info" &&
    (previous.message === label || previous.message.startsWith(`${label} · `))
  ) {
    return [
      ...logs.slice(0, -1),
      { ...previous, message },
    ];
  }
  return appendEntry(logs, "progress", "info", message);
}

function appendDiagnostic(
  logs: PdfJobLogEntry[],
  diagnostic: PdfQualityDiagnostic,
): PdfJobLogEntry[] {
  const location = [
    diagnostic.pageNumber === null ? null : `第 ${diagnostic.pageNumber} 页`,
    diagnostic.segmentId ? `段落 ${diagnostic.segmentId}` : null,
  ].filter((item): item is string => item !== null);
  const details = [
    diagnostic.severity === "error" ? "错误" : diagnostic.severity === "warning" ? "警告" : "提示",
    diagnostic.ruleId,
    diagnostic.message,
    ...location,
  ].filter((item): item is string => item !== null && item.length > 0);
  return appendEntry(
    logs,
    "quality",
    diagnostic.severity === "error" ? "error" : diagnostic.severity === "warning" ? "warning" : "info",
    details.join(" · "),
  );
}

function appendEventDiagnostics(logs: PdfJobLogEntry[], event: PdfJobEvent): PdfJobLogEntry[] {
  if (!("diagnostics" in event) || !event.diagnostics) return logs;
  return event.diagnostics.reduce(appendDiagnostic, logs);
}

export function reducePdfJobLog(logs: PdfJobLogEntry[], event: PdfJobEvent): PdfJobLogEntry[] {
  let next = logs;
  switch (event.type) {
    case "started":
      next = appendEntry(
        next,
        "system",
        "info",
        event.workerVersion
          ? `翻译任务已启动 · 状态：翻译进行中 · Worker ${event.workerVersion}`
          : "翻译任务已启动 · 状态：翻译进行中",
      );
      break;
    case "stage":
      next = appendEntry(next, "stage", "info", `阶段：${stageLabel(event.stage)}`);
      break;
    case "progress":
      next = appendProgressLog(next, event.progress);
      break;
    case "warning":
      next = appendEntry(next, "warning", "warning", event.message);
      break;
    case "diagnostic":
      next = appendDiagnostic(next, event.diagnostic);
      break;
    case "preflightStarted":
      next = appendEntry(next, "preflight", "info", "文档预检已启动");
      break;
    case "preflightActivity":
      next = appendEntry(next, "preflight", "info", `文档预检${event.phase === "thinking" ? "分析中" : "生成中"}`);
      break;
    case "preflightCompleted":
      next = appendEntry(
        next,
        "preflight",
        "info",
        event.preflight.applied ? "文档预检已完成 · 上下文已应用" : "文档预检已完成 · 未应用上下文",
      );
      next = appendPreflightWarnings(next, event.preflight.warnings);
      next = appendPreflightContext(next, event.preflight.context);
      if (!event.preflight.context) {
        next = appendEntry(next, "preflight", "info", "未返回文档上下文，继续使用普通 PDF 翻译");
      }
      break;
    case "preflightDegraded":
      next = appendEntry(next, "preflight", "warning", `文档预检已降级 · ${event.preflight.message ?? "继续翻译"}`);
      next = appendPreflightWarnings(next, event.preflight.warnings);
      next = appendPreflightContext(next, event.preflight.context);
      break;
    case "preflightFailed":
      next = appendEntry(next, "preflight", "error", `文档预检失败 · ${event.preflight.message ?? "继续翻译"}`);
      next = appendPreflightWarnings(next, event.preflight.warnings);
      next = appendPreflightContext(next, event.preflight.context);
      break;
    case "finished":
      next = event.warnings.reduce(
        (result, warning) => appendUniqueEntry(result, "warning", "warning", warning),
        next,
      );
      next = appendEntry(
        next,
        "result",
        "info",
        event.pageCount ? `翻译完成 · ${event.pageCount} 页` : "翻译完成",
      );
      break;
    case "cancelled":
      next = appendEntry(next, "result", "warning", event.reason ? `任务已取消 · ${event.reason}` : "任务已取消");
      break;
    case "failed":
      next = appendEntry(
        next,
        "result",
        "error",
        `翻译失败 · ${event.message} · 错误代码：${event.code}`,
      );
      break;
    case "tokenUsage":
      break;
  }
  return appendEventDiagnostics(next, event);
}
