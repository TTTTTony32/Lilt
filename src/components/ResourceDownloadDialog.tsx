import { Check, Download, LoaderCircle, X } from "lucide-react";
import { useEffect, useRef } from "react";
import { AnimatedOverlay } from "./AnimatedOverlay";
import { formatDownloadPhase, type DownloadResource } from "../lib/download-activity";

export type ResourceDownloadDialogStatus = "missing" | "running" | "completed" | "failed";

export interface ResourceDownloadDialogProps {
  open: boolean;
  resource: DownloadResource;
  title: string;
  description: string;
  status: ResourceDownloadDialogStatus;
  phase: string | null;
  stagePercent: number | null;
  overallPercent: number | null;
  message: string | null;
  error: string | null;
  startLabel: string;
  onStart: () => void;
  onRequestClose: () => void;
  onClosed: () => void;
}

function statusDescription(status: ResourceDownloadDialogStatus): string {
  if (status === "running") return "资源准备正在后台执行，关闭提示不会中断任务。";
  if (status === "completed") return "资源已经准备完成，当前功能可以继续使用。";
  if (status === "failed") return "资源准备失败，可以重试；页面原有入口仍然可用。";
  return "当前功能依赖本地资源，首次使用前需要完成准备。";
}

export function ResourceDownloadDialog({
  open,
  resource,
  title,
  description,
  status,
  phase,
  stagePercent,
  overallPercent,
  message,
  error,
  startLabel,
  onStart,
  onRequestClose,
  onClosed,
}: ResourceDownloadDialogProps) {
  const dialogRef = useRef<HTMLDivElement | null>(null);
  const titleId = `${resource}-resource-download-dialog-title`;
  const running = status === "running";
  const canStart = status === "missing" || status === "failed";
  const progressValue = overallPercent === null ? undefined : Math.round(Math.max(0, Math.min(100, overallPercent)));

  useEffect(() => {
    if (!open) return;
    const frame = window.requestAnimationFrame(() => dialogRef.current?.focus());
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      onRequestClose();
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      window.cancelAnimationFrame(frame);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [onRequestClose, open]);

  return (
    <AnimatedOverlay
      className="modal-backdrop resource-download-backdrop"
      open={open}
      onClosed={onClosed}
      onBackdropClick={(event) => {
        if (event.target === event.currentTarget) onRequestClose();
      }}
    >
      <div
        className="modal-card resource-download-card"
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
      >
        <div className="modal-heading">
          <div>
            <strong id={titleId}>{title}</strong>
            <span>{description}</span>
          </div>
          <button className="icon-button" type="button" onClick={onRequestClose} aria-label="关闭资源提示" title="关闭资源提示"><X size={17} /></button>
        </div>

        <div className="resource-download-body" aria-live="polite">
          <p className={`resource-download-status resource-download-status-${status}`}>
            {status === "running" && <LoaderCircle className="spin" size={16} />}
            {status === "completed" && <Check size={16} />}
            {statusDescription(status)}
          </p>
          {status !== "missing" && (phase || progressValue !== undefined) && (
            <div className="resource-download-progress">
              <div className="resource-download-progress-label">
                <span>{formatDownloadPhase(resource, phase)}{message ? ` · ${message}` : ""}</span>
                <span>{progressValue === undefined ? "" : `${progressValue}%`}</span>
              </div>
              <div className="resource-download-progress-track" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={progressValue}>
                <span style={{ width: `${progressValue ?? 0}%` }} />
              </div>
              {stagePercent !== null && <span className="resource-download-stage-percent">当前阶段 {Math.round(stagePercent)}%</span>}
            </div>
          )}
          {error && <p className="error-message resource-download-error" role="alert">{error}</p>}
        </div>

        <div className="form-actions modal-actions">
          {canStart && <button className="primary-button" type="button" onClick={onStart}><Download size={15} />{status === "failed" ? "重试" : startLabel}</button>}
          {running && <span className="resource-download-running-hint">可以关闭窗口，任务会继续。</span>}
          <button className="secondary-button" type="button" onClick={onRequestClose}>{status === "completed" ? "完成" : "关闭"}</button>
        </div>
      </div>
    </AnimatedOverlay>
  );
}
