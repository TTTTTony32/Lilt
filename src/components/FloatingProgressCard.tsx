import { AlertCircle, Check, LoaderCircle } from "lucide-react";

export type FloatingProgressCardStatus = "running" | "completed" | "failed";

export interface FloatingProgressCardData {
  status: FloatingProgressCardStatus;
  statusText: string;
  progress: number | null;
  error?: string | null;
}

export interface FloatingProgressCardProps extends FloatingProgressCardData {
  ariaLabel?: string;
  onClick?: () => void;
}

export function FloatingProgressCard({
  status,
  statusText,
  progress,
  error,
  ariaLabel,
  onClick,
}: FloatingProgressCardProps) {
  const progressValue = progress === null ? 0 : Math.max(0, Math.min(100, progress));
  const className = [
    "progress-floating-card",
    `progress-floating-card-${status}`,
    onClick ? "progress-floating-card-clickable" : "",
  ].filter(Boolean).join(" ");
  const content = (
    <>
      <span className="progress-floating-heading">
        <span className="progress-floating-icon" aria-hidden="true">
          {status === "running" && <LoaderCircle className="spin" size={14} />}
          {status === "completed" && <Check size={14} />}
          {status === "failed" && <AlertCircle size={14} />}
        </span>
        <span className="progress-floating-text">{statusText}</span>
      </span>
      <span className="progress-floating-progress" aria-hidden="true">
        <span style={{ width: `${progressValue}%` }} />
      </span>
      {error && <span className="progress-floating-error">{error}</span>}
    </>
  );

  if (onClick) {
    return (
      <button className={className} type="button" onClick={onClick} aria-label={ariaLabel ?? statusText}>
        {content}
      </button>
    );
  }

  return <div className={className}>{content}</div>;
}
