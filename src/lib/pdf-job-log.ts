import type {
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
  return appendEntry(
    logs,
    "quality",
    diagnostic.severity === "error" ? "error" : diagnostic.severity === "warning" ? "warning" : "info",
    `质检：${diagnostic.message}`,
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
        event.workerVersion ? `翻译任务已启动 · Worker ${event.workerVersion}` : "翻译任务已启动",
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
      next = appendEntry(next, "preflight", "info", "文档预检已完成");
      break;
    case "preflightDegraded":
      next = appendEntry(next, "preflight", "warning", event.preflight.message ?? "文档预检降级，继续翻译");
      break;
    case "preflightFailed":
      next = appendEntry(next, "preflight", "error", event.preflight.message ?? "文档预检失败，继续翻译");
      break;
    case "finished":
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
      next = appendEntry(next, "result", "error", event.message);
      break;
    case "tokenUsage":
      break;
  }
  return appendEventDiagnostics(next, event);
}
