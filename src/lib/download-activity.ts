export const DOWNLOAD_RESOURCES = ["dictionary", "pdf-engine"] as const;

export type DownloadResource = (typeof DOWNLOAD_RESOURCES)[number];
export type DownloadActivityStatus = "running" | "completed" | "failed";

export interface ResourceDownloadPromptRequest {
  resource: DownloadResource;
  title: string;
  description: string;
  startLabel: string;
  onStart: () => void;
}

export interface DownloadActivity {
  key: string;
  resource: DownloadResource;
  operationId: string;
  status: DownloadActivityStatus;
  phase: string | null;
  stagePercent: number | null;
  overallPercent: number | null;
  message: string | null;
  error: string | null;
}

export interface DownloadActivityState {
  byKey: Record<string, DownloadActivity>;
  latestByResource: Partial<Record<DownloadResource, string>>;
}

export type DownloadActivityAction =
  | {
    type: "started";
    resource: DownloadResource;
    operationId: string;
    phase: string;
    stagePercent?: number | null;
    message?: string | null;
  }
  | {
    type: "progress";
    resource: DownloadResource;
    operationId: string;
    phase: string;
    stagePercent: number | null;
    message?: string | null;
  }
  | {
    type: "completed";
    resource: DownloadResource;
    operationId: string;
    message?: string | null;
  }
  | {
    type: "failed";
    resource: DownloadResource;
    operationId: string;
    error: string;
  }
  | {
    type: "removed";
    resource: DownloadResource;
    operationId: string;
  };

export const initialDownloadActivityState: DownloadActivityState = {
  byKey: {},
  latestByResource: {},
};

interface ProgressRange {
  start: number;
  end: number;
}

function normalizedPhase(phase: string): string {
  return phase.trim().toLowerCase().replace(/[\s-]+/g, "_");
}

function progressRange(resource: DownloadResource, phase: string): ProgressRange | null {
  const normalized = normalizedPhase(phase);
  if (resource === "dictionary") {
    if (normalized === "download") return { start: 0, end: 70 };
    if (normalized === "verify" || normalized === "verification") return { start: 70, end: 85 };
    if (normalized === "extract" || normalized === "extraction") return { start: 85, end: 100 };
    return null;
  }

  if (normalized === "index" || normalized === "prepare") return { start: 0, end: 5 };
  if (normalized === "download") return { start: 5, end: 75 };
  if (normalized === "verify" || normalized === "verification" || normalized === "extract" || normalized === "extraction") {
    return { start: 75, end: 100 };
  }
  return null;
}

function clampPercent(value: number): number {
  return Math.max(0, Math.min(100, value));
}

function safeStagePercent(value: number | null | undefined): number | null {
  return value === null || value === undefined || !Number.isFinite(value)
    ? null
    : clampPercent(value);
}

export function normalizeStagePercent(
  current: number | null,
  total: number | null,
  fraction: number | null = null,
): number | null {
  if (fraction !== null && Number.isFinite(fraction)) {
    return clampPercent(fraction <= 1 ? fraction * 100 : fraction);
  }
  if (current === null || total === null || !Number.isFinite(current) || !Number.isFinite(total) || total <= 0) {
    return null;
  }
  return clampPercent((current / total) * 100);
}

export function calculateOverallPercent(
  resource: DownloadResource,
  phase: string,
  stagePercent: number | null,
): number | null {
  const range = progressRange(resource, phase);
  if (!range) return null;
  const normalized = safeStagePercent(stagePercent);
  if (normalized === null) return range.start;
  return Math.round(range.start + ((range.end - range.start) * normalized) / 100);
}

export function downloadActivityKey(resource: DownloadResource, operationId: string): string {
  return `${resource}:${operationId}`;
}

export function formatDownloadResource(resource: DownloadResource): string {
  return resource === "dictionary" ? "词典" : "PDF Engine";
}

export function formatDownloadPhase(resource: DownloadResource, phase: string | null): string {
  if (!phase) return "准备";
  const normalized = normalizedPhase(phase);
  if (resource === "dictionary") {
    if (normalized === "download") return "下载";
    if (normalized === "verify" || normalized === "verification") return "校验";
    if (normalized === "extract" || normalized === "extraction") return "解压";
  } else {
    if (normalized === "index" || normalized === "prepare") return "读取索引";
    if (normalized === "download") return "下载";
    if (normalized === "verify" || normalized === "verification") return "校验";
    if (normalized === "extract" || normalized === "extraction") return "校验与解压";
  }
  return phase.replace(/[_-]+/g, " ");
}

function copyState(state: DownloadActivityState, byKey: Record<string, DownloadActivity>): DownloadActivityState {
  return { ...state, byKey };
}

function isLatestOperation(state: DownloadActivityState, resource: DownloadResource, operationId: string): boolean {
  return state.latestByResource[resource] === operationId;
}

function createActivity(
  resource: DownloadResource,
  operationId: string,
  phase: string,
  stagePercent: number | null,
  message: string | null,
): DownloadActivity {
  return {
    key: downloadActivityKey(resource, operationId),
    resource,
    operationId,
    status: "running",
    phase,
    stagePercent: safeStagePercent(stagePercent),
    overallPercent: calculateOverallPercent(resource, phase, stagePercent),
    message,
    error: null,
  };
}

export function downloadActivityReducer(
  state: DownloadActivityState,
  action: DownloadActivityAction,
): DownloadActivityState {
  if (!action.operationId.trim()) return state;
  const key = downloadActivityKey(action.resource, action.operationId);
  const current = state.byKey[key];

  switch (action.type) {
    case "started": {
      const latestOperationId = state.latestByResource[action.resource];
      const latest = latestOperationId
        ? state.byKey[downloadActivityKey(action.resource, latestOperationId)]
        : undefined;
      if (latest && latestOperationId !== action.operationId && latest.status === "running") return state;
      const next = createActivity(
        action.resource,
        action.operationId,
        action.phase,
        action.stagePercent ?? null,
        action.message ?? null,
      );
      return {
        byKey: { ...state.byKey, [key]: next },
        latestByResource: { ...state.latestByResource, [action.resource]: action.operationId },
      };
    }
    case "progress": {
      if (!current || current.status !== "running" || !isLatestOperation(state, action.resource, action.operationId)) return state;
      const stagePercent = safeStagePercent(action.stagePercent);
      const next = {
        ...current,
        phase: action.phase,
        stagePercent,
        overallPercent: calculateOverallPercent(action.resource, action.phase, stagePercent),
        message: action.message ?? current.message,
      };
      return copyState(state, { ...state.byKey, [key]: next });
    }
    case "completed": {
      if (!current || !isLatestOperation(state, action.resource, action.operationId)) return state;
      const next = {
        ...current,
        status: "completed" as const,
        stagePercent: 100,
        overallPercent: 100,
        message: action.message ?? current.message,
        error: null,
      };
      return copyState(state, { ...state.byKey, [key]: next });
    }
    case "failed": {
      if (!current || !isLatestOperation(state, action.resource, action.operationId)) return state;
      const next = {
        ...current,
        status: "failed" as const,
        error: action.error,
      };
      return copyState(state, { ...state.byKey, [key]: next });
    }
    case "removed": {
      if (!current || !isLatestOperation(state, action.resource, action.operationId)) return state;
      const next = { ...state.byKey };
      delete next[key];
      return copyState(state, next);
    }
  }
}

export function listDownloadActivities(state: DownloadActivityState): DownloadActivity[] {
  return Object.values(state.byKey);
}

export function selectDownloadActivity(
  state: DownloadActivityState,
  resource: DownloadResource,
): DownloadActivity | null {
  const operationId = state.latestByResource[resource];
  return operationId ? state.byKey[downloadActivityKey(resource, operationId)] ?? null : null;
}
