import { DEFAULT_DICTIONARY_STATE, type DictionaryHistoryEntry, type DictionaryState } from "./dictionary";

export type AppTab = "translate" | "dictionary" | "pdf" | "personal" | "glossary";

export type TranslationStatus = "idle" | "streaming" | "cancelling" | "completed" | "failed";

export interface AppSettings {
  historyRetention: number;
  cacheEnabled: boolean;
  cacheMaxBytes: number;
  cacheUsageBytes: number;
  wordAiCacheEnabled: boolean;
  paragraphExampleLookupEnabled: boolean;
  selectionMode: "shortcut" | "automatic";
  selectionShortcut: string;
  selectionWindowWidth: number;
  selectionWindowHeight: number;
  closeBehavior: CloseBehavior;
}

export type SelectionMode = "shortcut" | "automatic";
export type SelectionTrigger = "shortcut" | "automatic";
export type CloseBehavior = "ask" | "exit" | "tray";

export interface SelectionAnchor {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface SelectionTriggerNotice {
  triggerId: string;
  trigger: SelectionTrigger;
  anchor: SelectionAnchor | null;
}

export interface SelectionNotice {
  requestId: string;
  triggerId: string;
  trigger: SelectionTrigger;
  anchor: SelectionAnchor | null;
}

export interface SelectionUnavailable {
  requestId: string | null;
  triggerId: string;
  trigger: SelectionTrigger;
  code: string;
  message: string;
}

export interface SelectionRuntimeStatus {
  mode: SelectionMode;
  shortcut: string;
  shortcutRegistered: boolean;
  uiAutomationReady: boolean;
  message: string | null;
}

export interface SelectionRequestPayload {
  requestId: string;
  sourceText: string;
  sourceLanguage: string;
  targetLanguage: string;
  trigger: SelectionTrigger;
  anchor: SelectionAnchor | null;
}

export interface ProviderConfig {
  id: string;
  name: string;
  baseUrl: string;
  modelId: string;
  promptId: string;
  thinkingEffort: ThinkingEffort;
  hasApiKey: boolean;
}

export type ThinkingEffort = "none" | "low" | "medium" | "high";

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

export interface PersonalDictionaryEntry {
  normalizedCanonicalWord: string;
  canonicalWord: string;
  lookupWord: string;
  savedAt: string;
}

export interface PersonalDictionaryExportResult {
  entryCount: number;
  fileName: string;
}

export interface GlossaryExportResult {
  entryCount: number;
  fileName: string;
}

export interface GlossaryTerm {
  id: string;
  source: string;
  target: string;
  note: string | null;
}

export interface GlossaryImportSkippedRow {
  line: number;
  reason: string;
}

export interface GlossaryImportResult {
  addedCount: number;
  updatedCount: number;
  skippedCount: number;
  skippedRows: GlossaryImportSkippedRow[];
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
  dictionary: DictionaryState;
  dictionaryHistory: DictionaryHistoryEntry[];
  personalDictionary: PersonalDictionaryEntry[];
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

export type WordExampleStatus = "idle" | "streaming" | "completed" | "cancelling" | "failed";

export interface WordExampleState {
  exampleId: number | null;
  requestId: string | null;
  translation: string;
  partOfSpeech: string;
  status: WordExampleStatus;
  cacheHit: boolean;
  error: string | null;
}

export interface WordExampleEventStarted {
  type: "started";
  requestId: string;
}

export interface WordExampleEventTranslationDelta {
  type: "translationDelta";
  requestId: string;
  content: string;
}

export interface WordExampleEventPosDelta {
  type: "posDelta";
  requestId: string;
  content: string;
}

export interface WordExampleEventCompleted {
  type: "completed";
  requestId: string;
  translation: string;
  partOfSpeech: string;
  cacheHit: boolean;
}

export interface WordExampleEventCancelled {
  type: "cancelled";
  requestId: string;
}

export interface WordExampleEventFailed {
  type: "failed";
  requestId: string;
  message: string;
}

export type WordExampleEvent =
  | WordExampleEventStarted
  | WordExampleEventTranslationDelta
  | WordExampleEventPosDelta
  | WordExampleEventCompleted
  | WordExampleEventCancelled
  | WordExampleEventFailed;

export type WordExampleOutcome = "completed" | "cancelled" | "failed";

export interface WordExampleCommandResult {
  outcome: WordExampleOutcome;
  translation: string | null;
  partOfSpeech: string | null;
  cacheHit: boolean;
  message: string | null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function stringValue(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function selectionAnchor(value: unknown): SelectionAnchor | null | undefined {
  if (value === null) return null;
  if (!isRecord(value)) return undefined;
  const x = value.x;
  const y = value.y;
  const width = value.width;
  const height = value.height;
  return typeof x === "number" && Number.isFinite(x) && typeof y === "number" && Number.isFinite(y)
    && typeof width === "number" && Number.isFinite(width) && typeof height === "number" && Number.isFinite(height)
    ? { x, y, width, height }
    : undefined;
}

function selectionTrigger(value: unknown): SelectionTrigger | null {
  return value === "shortcut" || value === "automatic" ? value : null;
}

export function decodeSelectionTriggerNotice(value: unknown): SelectionTriggerNotice | null {
  if (!isRecord(value)) return null;
  const triggerId = stringValue(value.triggerId);
  const trigger = selectionTrigger(value.trigger);
  const anchor = selectionAnchor(value.anchor);
  return triggerId === null || trigger === null || anchor === undefined
    ? null
    : { triggerId, trigger, anchor };
}

export function decodeSelectionNotice(value: unknown): SelectionNotice | null {
  if (!isRecord(value)) return null;
  const requestId = stringValue(value.requestId);
  const triggerId = stringValue(value.triggerId);
  const trigger = selectionTrigger(value.trigger);
  const anchor = selectionAnchor(value.anchor);
  return requestId === null || triggerId === null || trigger === null || anchor === undefined
    ? null
    : { requestId, triggerId, trigger, anchor };
}

export function decodeSelectionUnavailable(value: unknown): SelectionUnavailable | null {
  if (!isRecord(value)) return null;
  const requestId = value.requestId === null || value.requestId === undefined ? null : stringValue(value.requestId);
  const triggerId = stringValue(value.triggerId);
  const trigger = selectionTrigger(value.trigger);
  const code = stringValue(value.code);
  const message = stringValue(value.message);
  return triggerId === null || trigger === null || code === null || message === null || (requestId === null && value.requestId !== null && value.requestId !== undefined)
    ? null
    : { requestId, triggerId, trigger, code, message };
}

export function decodeSelectionStatus(value: unknown): SelectionRuntimeStatus | null {
  if (!isRecord(value)) return null;
  const mode = value.mode === "shortcut" || value.mode === "automatic" ? value.mode : null;
  const shortcut = stringValue(value.shortcut);
  const message = value.message === null || value.message === undefined ? null : stringValue(value.message);
  return mode === null || shortcut === null || typeof value.shortcutRegistered !== "boolean"
    || typeof value.uiAutomationReady !== "boolean" || (message === null && value.message !== null && value.message !== undefined)
    ? null
    : { mode, shortcut, shortcutRegistered: value.shortcutRegistered, uiAutomationReady: value.uiAutomationReady, message };
}

export function decodeSelectionRequest(value: unknown): SelectionRequestPayload | null {
  if (!isRecord(value)) return null;
  const requestId = stringValue(value.requestId);
  const sourceText = stringValue(value.sourceText);
  const sourceLanguage = stringValue(value.sourceLanguage);
  const targetLanguage = stringValue(value.targetLanguage);
  const trigger = selectionTrigger(value.trigger);
  const anchor = selectionAnchor(value.anchor);
  return requestId === null || sourceText === null || sourceLanguage === null || targetLanguage === null
    || trigger === null || anchor === undefined
    ? null
    : { requestId, sourceText, sourceLanguage, targetLanguage, trigger, anchor };
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

export function decodeWordExampleEvent(name: string, value: unknown): WordExampleEvent | null {
  if (!isRecord(value)) return null;
  const requestId = stringValue(value.requestId);
  if (!requestId) return null;

  if (name === "word_example_started") return { type: "started", requestId };
  if (name === "word_example_translation_delta") {
    const content = stringValue(value.content);
    return content === null ? null : { type: "translationDelta", requestId, content };
  }
  if (name === "word_example_pos_delta") {
    const content = stringValue(value.content);
    return content === null ? null : { type: "posDelta", requestId, content };
  }
  if (name === "word_example_completed") {
    const translation = stringValue(value.translation);
    const partOfSpeech = stringValue(value.partOfSpeech);
    return translation === null || partOfSpeech === null || typeof value.cacheHit !== "boolean"
      ? null
      : { type: "completed", requestId, translation, partOfSpeech, cacheHit: value.cacheHit };
  }
  if (name === "word_example_cancelled") return { type: "cancelled", requestId };
  if (name === "word_example_failed") {
    const message = stringValue(value.message);
    return message === null ? null : { type: "failed", requestId, message };
  }
  return null;
}

export function decodeWordExampleCommandResult(value: unknown): WordExampleCommandResult | null {
  if (!isRecord(value)) return null;
  const outcome = value.outcome;
  if (outcome !== "completed" && outcome !== "cancelled" && outcome !== "failed") return null;
  if (typeof value.cacheHit !== "boolean") return null;
  const translation = value.translation === null || value.translation === undefined
    ? null
    : stringValue(value.translation);
  const partOfSpeech = value.partOfSpeech === null || value.partOfSpeech === undefined
    ? null
    : stringValue(value.partOfSpeech);
  const message = value.message === null || value.message === undefined
    ? null
    : stringValue(value.message);
  if (
    (translation === null && value.translation !== null && value.translation !== undefined) ||
    (partOfSpeech === null && value.partOfSpeech !== null && value.partOfSpeech !== undefined) ||
    (message === null && value.message !== null && value.message !== undefined) ||
    (outcome === "completed" && (translation === null || partOfSpeech === null)) ||
    (outcome === "failed" && message === null)
  ) {
    return null;
  }
  return { outcome, translation, partOfSpeech, cacheHit: value.cacheHit, message };
}

export function decodePrompt(value: unknown): Prompt | null {
  if (!isRecord(value)) return null;
  const id = stringValue(value.id);
  const name = stringValue(value.name);
  const content = stringValue(value.content);
  const version = value.version;
  if (id === null || name === null || content === null || typeof version !== "number" || !Number.isInteger(version) || version < 1 || typeof value.isBuiltin !== "boolean") {
    return null;
  }
  return { id, name, content, version, isBuiltin: value.isBuiltin };
}

function nonNegativeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= 0;
}

export function decodePersonalDictionaryExportResult(value: unknown): PersonalDictionaryExportResult | null {
  if (!isRecord(value) || !nonNegativeInteger(value.entryCount)) return null;
  const fileName = stringValue(value.fileName);
  return fileName === null || fileName.trim().length === 0
    ? null
    : { entryCount: value.entryCount, fileName };
}

export function decodeGlossaryExportResult(value: unknown): GlossaryExportResult | null {
  if (!isRecord(value) || !nonNegativeInteger(value.entryCount)) return null;
  const fileName = stringValue(value.fileName);
  return fileName === null || fileName.trim().length === 0
    ? null
    : { entryCount: value.entryCount, fileName };
}

export function decodeGlossaryImportResult(value: unknown): GlossaryImportResult | null {
  if (
    !isRecord(value)
    || !nonNegativeInteger(value.addedCount)
    || !nonNegativeInteger(value.updatedCount)
    || !nonNegativeInteger(value.skippedCount)
    || !Array.isArray(value.skippedRows)
  ) {
    return null;
  }
  const skippedRows: GlossaryImportSkippedRow[] = [];
  for (const row of value.skippedRows) {
    if (!isRecord(row) || !nonNegativeInteger(row.line) || row.line < 1) return null;
    const reason = stringValue(row.reason);
    if (reason === null || reason.trim().length === 0) return null;
    skippedRows.push({ line: row.line, reason });
  }
  if (value.skippedCount !== skippedRows.length) return null;
  return {
    addedCount: value.addedCount,
    updatedCount: value.updatedCount,
    skippedCount: value.skippedCount,
    skippedRows,
  };
}

export const DEFAULT_SNAPSHOT: AppSnapshot = {
  settings: {
    historyRetention: 50,
    cacheEnabled: true,
    cacheMaxBytes: 268_435_456,
    cacheUsageBytes: 0,
    wordAiCacheEnabled: true,
    paragraphExampleLookupEnabled: true,
    selectionMode: "shortcut",
    selectionShortcut: "Ctrl+Shift+L",
    selectionWindowWidth: 560,
    selectionWindowHeight: 320,
    closeBehavior: "ask",
  },
  provider: {
    id: "default",
    name: "OpenAI-compatible",
    baseUrl: "https://api.openai.com/v1",
    modelId: "gpt-4o-mini",
    promptId: "builtin-general",
    thinkingEffort: "none",
    hasApiKey: false,
  },
  models: [],
  prompts: [],
  glossaryTerms: [],
  history: [],
  cacheStats: { usageBytes: 0, entryCount: 0, maxBytes: 268_435_456 },
  dictionary: DEFAULT_DICTIONARY_STATE,
  dictionaryHistory: [],
  personalDictionary: [],
};
