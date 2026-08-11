export type AppTab = "translate" | "glossary" | "history" | "settings";

export type TranslationStatus = "idle" | "streaming" | "cancelling" | "completed" | "failed";

export interface AppSettings {
  historyRetention: number;
  cacheEnabled: boolean;
  cacheMaxBytes: number;
  cacheUsageBytes: number;
}

export interface ProviderConfig {
  id: string;
  name: string;
  baseUrl: string;
  modelId: string;
  promptId: string;
  hasApiKey: boolean;
}

export interface ModelInfo {
  id: string;
  label: string;
  source: "remote" | "manual";
}

export interface Prompt {
  id: string;
  name: string;
  content: string;
  version: number;
  isBuiltin: boolean;
}

export interface GlossaryTerm {
  id: string;
  source: string;
  target: string;
  note: string | null;
}

export interface HistoryEntry {
  id: string;
  createdAt: string;
  sourceText: string;
  translatedText: string;
  sourceLanguage: string;
  targetLanguage: string;
  providerName: string;
  modelId: string;
  cacheHit: boolean;
}

export interface CacheStats {
  usageBytes: number;
  entryCount: number;
  maxBytes: number;
}

export interface AppSnapshot {
  settings: AppSettings;
  provider: ProviderConfig;
  models: ModelInfo[];
  prompts: Prompt[];
  glossaryTerms: GlossaryTerm[];
  history: HistoryEntry[];
  cacheStats: CacheStats;
}

export interface TranslationEventStarted {
  type: "started";
  requestId: string;
}

export interface TranslationEventDelta {
  type: "delta";
  requestId: string;
  content: string;
}

export interface TranslationEventCompleted {
  type: "completed";
  requestId: string;
  content: string;
  cacheHit: boolean;
}

export interface TranslationEventCancelled {
  type: "cancelled";
  requestId: string;
}

export interface TranslationEventFailed {
  type: "failed";
  requestId: string;
  message: string;
}

export type TranslationEvent =
  | TranslationEventStarted
  | TranslationEventDelta
  | TranslationEventCompleted
  | TranslationEventCancelled
  | TranslationEventFailed;

export type TranslationOutcome = "completed" | "cancelled" | "failed";

export interface TranslationCommandResult {
  outcome: TranslationOutcome;
  content: string | null;
  cacheHit: boolean;
  message: string | null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function stringValue(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

export function decodeTranslationEvent(name: string, value: unknown): TranslationEvent | null {
  if (!isRecord(value)) return null;
  const requestId = stringValue(value.requestId);
  if (!requestId) return null;

  if (name === "translation_started") return { type: "started", requestId };
  if (name === "translation_delta") {
    const content = stringValue(value.content);
    return content === null ? null : { type: "delta", requestId, content };
  }
  if (name === "translation_completed") {
    const content = stringValue(value.content);
    if (typeof value.cacheHit !== "boolean") return null;
    return content === null
      ? null
      : { type: "completed", requestId, content, cacheHit: value.cacheHit };
  }
  if (name === "translation_cancelled") return { type: "cancelled", requestId };
  if (name === "translation_failed") {
    const message = stringValue(value.message);
    return message === null ? null : { type: "failed", requestId, message };
  }
  return null;
}

export function decodeTranslationCommandResult(value: unknown): TranslationCommandResult | null {
  if (!isRecord(value)) return null;

  const outcome = value.outcome;
  if (outcome !== "completed" && outcome !== "cancelled" && outcome !== "failed") return null;

  if (typeof value.cacheHit !== "boolean") return null;

  const content = value.content === null || value.content === undefined
    ? null
    : stringValue(value.content);
  if (content === null && value.content !== null && value.content !== undefined) return null;

  const message = value.message === null || value.message === undefined
    ? null
    : stringValue(value.message);
  if (message === null && value.message !== null && value.message !== undefined) return null;

  if (outcome === "completed" && content === null) return null;
  if (outcome === "failed" && message === null) return null;

  return { outcome, content, cacheHit: value.cacheHit, message };
}

export const DEFAULT_SNAPSHOT: AppSnapshot = {
  settings: {
    historyRetention: 50,
    cacheEnabled: true,
    cacheMaxBytes: 268_435_456,
    cacheUsageBytes: 0,
  },
  provider: {
    id: "default",
    name: "OpenAI-compatible",
    baseUrl: "https://api.openai.com/v1",
    modelId: "gpt-4o-mini",
    promptId: "builtin-general",
    hasApiKey: false,
  },
  models: [],
  prompts: [],
  glossaryTerms: [],
  history: [],
  cacheStats: { usageBytes: 0, entryCount: 0, maxBytes: 268_435_456 },
};
