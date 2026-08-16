import type { PdfEngineProgress, PdfEngineStatus } from "./types/contracts";

interface PdfEnginePanelProps {
  engineStatus: PdfEngineStatus | null;
  engineStatusLoading: boolean;
  enginePreparing: boolean;
  engineProgress: PdfEngineProgress | null;
  engineError: string | null;
  onPrepareEngine: () => void;
}

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

function progressPercent(progress: PdfEngineProgress | null): number | null {
  if (!progress) return null;
  if (progress.fraction !== null) return Math.max(0, Math.min(100, Math.round(progress.fraction * 100)));
  if (progress.current !== null && progress.total !== null && progress.total > 0) {
    return Math.max(0, Math.min(100, Math.round((progress.current / progress.total) * 100)));
  }
  return null;
}

function formatBytes(value: number | null): string | null {
  if (value === null || !Number.isFinite(value) || value < 0) return null;
  if (value < 1024) return `${value} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let size = value;
  let unit = -1;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${size >= 10 ? size.toFixed(0) : size.toFixed(1)} ${units[unit]}`;
}

function engineStatusLabel(status: PdfEngineStatus | null, loading: boolean, preparing: boolean): string {
  if (preparing || status?.status === "preparing" || status?.updating) return status?.updating ? "更新中" : "准备中";
  if (loading) return "检查中";
  if (status?.status === "missing") return "未准备";
  if (status?.status === "ready") return "可用";
  if (status?.status === "invalid") return "失败";
  return "未准备";
}

export function PdfEnginePanel({
  engineStatus,
  engineStatusLoading,
  enginePreparing,
  engineProgress,
  engineError,
  onPrepareEngine,
}: PdfEnginePanelProps) {
  const engineProgressValue = progressPercent(engineProgress);
  const engineDetails = [
    engineStatus?.engineVersion ? `Engine ${engineStatus.engineVersion}` : null,
    engineStatus?.babeldocVersion ? `BabelDOC ${engineStatus.babeldocVersion}` : null,
    engineStatus?.pythonVersion ? `Python ${engineStatus.pythonVersion}` : null,
    engineStatus?.distributionVersion ? `资源 ${engineStatus.distributionVersion}` : null,
    formatBytes(engineStatus?.resourceSizeBytes ?? null),
    engineStatus?.target ?? null,
  ].filter((detail): detail is string => detail !== null);

  return (
    <div className="pdf-engine-panel">
      <div className="pdf-task-panel-heading">
        <div>
          <span className="pdf-task-panel-kicker">PDF ENGINE</span>
          <strong>{engineStatusLabel(engineStatus, engineStatusLoading, enginePreparing)}</strong>
        </div>
        <button
          className="secondary-button small-button"
          type="button"
          onClick={onPrepareEngine}
          disabled={engineStatusLoading || enginePreparing}
        >
          {enginePreparing ? "准备中" : engineStatus?.status === "ready" ? "重新准备" : "准备 Engine"}
        </button>
      </div>
      {engineDetails.length > 0 && <p className="pdf-task-panel-meta">{engineDetails.join(" · ")}</p>}
      {(enginePreparing || engineProgress) && (
        <div className="pdf-task-progress-block">
          <div className="pdf-task-progress-label">
            <span>{engineProgress?.message ?? (engineProgress ? formatPdfStage(engineProgress.stage) : "正在准备运行环境")}</span>
            {engineProgressValue !== null && <span>{engineProgressValue}%</span>}
          </div>
          <div className="pdf-task-progress-track" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={engineProgressValue ?? undefined}>
            {engineProgressValue !== null && <span style={{ width: `${engineProgressValue}%` }} />}
          </div>
        </div>
      )}
      {engineError && <p className="pdf-task-error" role="alert">{engineError}</p>}
    </div>
  );
}
