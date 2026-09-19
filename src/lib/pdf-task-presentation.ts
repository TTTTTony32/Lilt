import type { PdfPreflightState, PdfJobUiState } from "../types/contracts";

function formatStageName(stage: string): string {
  return stage
    .replace(/[_-]+/g, " ")
    .replace(/\b\w/g, (character) => character.toUpperCase());
}

function formatPdfStage(stage: string | null): string {
  if (!stage) return "等待 Worker";
  const normalized = stage.toLowerCase().replace(/[-\s]+/g, "_");
  const label = formatStageName(stage);
  if (normalized.includes("babeldoc") || /parse|layout|render|typeset|output|finish/.test(normalized)) {
    return `BabelDOC · ${label}`;
  }
  return `Worker · ${label}`;
}

export function progressPercent(progress: { fraction: number | null; current: number | null; total: number | null } | null): number | null {
  if (!progress) return null;
  if (progress.fraction !== null) return Math.max(0, Math.min(100, Math.round(progress.fraction * 100)));
  if (progress.current !== null && progress.total !== null && progress.total > 0) {
    return Math.max(0, Math.min(100, Math.round((progress.current / progress.total) * 100)));
  }
  return null;
}

export function formatPdfTaskProgressStage(stage: string | null): string {
  if (!stage) return "等待翻译";
  const normalized = stage.toLowerCase().replace(/[-\s]+/g, "_");
  if (normalized.includes("preflight")) return "预检";
  if (/(translate|translation|segment)/.test(normalized)) return "分段翻译中";
  if (/(assemble|render|output|finish)/.test(normalized)) return "生成 PDF";
  return formatPdfStage(stage);
}

export function formatPdfTaskProgressDetail(
  job: PdfJobUiState,
  preflight: PdfPreflightState,
  preflightEnabled: boolean,
): string {
  if (preflightEnabled && preflight.status === "running") {
    const phase = preflight.responsePhase === "waiting"
      ? "等待模型响应"
      : preflight.responsePhase === "thinking"
        ? "模型思考中"
        : preflight.responsePhase === "streaming"
          ? "生成预检结果"
          : "分析文档";
    return `预检 · ${phase}`;
  }
  if (job.progress) {
    const stage = formatPdfTaskProgressStage(job.progress.stage);
    if (job.progress.current !== null && job.progress.total !== null) {
      return `${stage} · ${job.progress.current}/${job.progress.total}`;
    }
    return job.progress.message ? `${stage} · ${job.progress.message}` : stage;
  }
  return jobStatusLabel(job.status);
}

export function jobStatusLabel(status: PdfJobUiState["status"]): string {
  switch (status) {
    case "starting": return "正在启动 Worker";
    case "running": return "翻译进行中";
    case "cancelling": return "正在取消";
    case "completed": return "翻译完成";
    case "cancelled": return "已取消";
    case "failed": return "翻译失败";
    default: return "等待翻译";
  }
}

export function isPdfJobBusy(status: PdfJobUiState["status"]): boolean {
  return status === "starting" || status === "running" || status === "cancelling";
}

export function canStartPdfTranslation(
  translationEnabled: boolean,
  readerReady: boolean,
  jobBusy: boolean,
  preflightSaving: boolean,
): boolean {
  return translationEnabled && readerReady && !jobBusy && !preflightSaving;
}
