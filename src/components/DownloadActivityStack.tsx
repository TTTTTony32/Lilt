import {
  formatDownloadPhase,
  formatDownloadResource,
  type DownloadActivity,
  type DownloadResource,
} from "../lib/download-activity";
import { FloatingProgressCard, type FloatingProgressCardData } from "./FloatingProgressCard";

interface DownloadActivityStackProps {
  activities: DownloadActivity[];
  pdfTranslation: FloatingProgressCardData | null;
  onPdfTranslationClick: () => void;
  onResourceClick: (resource: DownloadResource) => void;
}

export function DownloadActivityStack({
  activities,
  pdfTranslation,
  onPdfTranslationClick,
  onResourceClick,
}: DownloadActivityStackProps) {
  if (activities.length === 0 && !pdfTranslation) return null;
  return (
    <aside className="progress-floating-stack" aria-live="polite" aria-label="后台任务">
      {pdfTranslation && (
        <FloatingProgressCard
          key="pdf-translation"
          {...pdfTranslation}
          ariaLabel="打开 PDF 翻译任务详情"
          onClick={onPdfTranslationClick}
        />
      )}
      {activities.map((activity) => {
        const overallPercent = activity.overallPercent === null ? null : Math.round(activity.overallPercent);
        const stagePercent = activity.stagePercent === null ? null : Math.round(activity.stagePercent);
        const statusText = activity.status === "running"
          ? `${formatDownloadResource(activity.resource)}${formatDownloadPhase(activity.resource, activity.phase)}中 · ${stagePercent === null ? "准备中" : `${stagePercent}%`}`
          : activity.status === "completed"
            ? `${formatDownloadResource(activity.resource)}下载完成`
            : `${formatDownloadResource(activity.resource)}下载失败`;
        return (
          <FloatingProgressCard
            key={activity.key}
            status={activity.status}
            statusText={statusText}
            progress={overallPercent}
            error={activity.error}
            ariaLabel={activity.resource === "pdf-engine" ? "打开 PDF Engine 准备设置" : "打开词典下载设置"}
            onClick={() => onResourceClick(activity.resource)}
          />
        );
      })}
    </aside>
  );
}
