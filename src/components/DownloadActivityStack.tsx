import { AlertCircle, Check, LoaderCircle } from "lucide-react";
import {
  formatDownloadPhase,
  formatDownloadResource,
  type DownloadActivity,
} from "../lib/download-activity";

export function DownloadActivityStack({ activities }: { activities: DownloadActivity[] }) {
  if (activities.length === 0) return null;
  return (
    <aside className="download-activity-stack" aria-live="polite" aria-label="资源准备任务">
      {activities.map((activity) => {
        const overallPercent = activity.overallPercent === null ? null : Math.round(activity.overallPercent);
        const stagePercent = activity.stagePercent === null ? null : Math.round(activity.stagePercent);
        const statusText = activity.status === "running"
          ? `${formatDownloadResource(activity.resource)}${formatDownloadPhase(activity.resource, activity.phase)}中 · ${stagePercent === null ? "准备中" : `${stagePercent}%`}`
          : activity.status === "completed"
            ? `${formatDownloadResource(activity.resource)}下载完成`
            : `${formatDownloadResource(activity.resource)}下载失败`;
        return (
          <div className={`download-activity-item download-activity-item-${activity.status}`} key={activity.key}>
            <div className="download-activity-heading">
              <span className="download-activity-icon" aria-hidden="true">
                {activity.status === "running" && <LoaderCircle className="spin" size={14} />}
                {activity.status === "completed" && <Check size={14} />}
                {activity.status === "failed" && <AlertCircle size={14} />}
              </span>
              <span className="download-activity-text">{statusText}</span>
            </div>
            <div className="download-activity-progress" aria-hidden="true">
              <span style={{ width: `${overallPercent ?? 0}%` }} />
            </div>
            {activity.status === "failed" && activity.error && <span className="download-activity-error">{activity.error}</span>}
          </div>
        );
      })}
    </aside>
  );
}
