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

export type PdfEngineStatusKind = "missing" | "preparing" | "ready" | "invalid";

export interface PdfEngineStatus {
  status: PdfEngineStatusKind;
  engineVersion: string | null;
  target: string | null;
  pythonVersion: string | null;
  babeldocVersion: string | null;
  error: string | null;
}

export interface PdfEngineProgress {
  operationId: string | null;
  stage: string;
  current: number | null;
  total: number | null;
  fraction: number | null;
  message: string | null;
}

export type PdfEngineEvent =
  | {
    type: "status";
    status: PdfEngineStatus;
  }
  | {
    type: "prepareStarted";
    operationId: string | null;
  }
  | {
    type: "prepareProgress";
    progress: PdfEngineProgress;
  }
  | {
    type: "prepareCompleted";
    operationId: string | null;
    status: PdfEngineStatus | null;
  }
  | {
    type: "prepareFailed";
    operationId: string | null;
    message: string;
  };

export interface PdfTranslationStarted {
  taskId: string;
}

export interface PdfJobProgress {
  stage: string;
  current: number | null;
  total: number | null;
  fraction: number | null;
  message: string | null;
}

export interface PdfJobTokenUsage {
  promptTokens: number | null;
  completionTokens: number | null;
  totalTokens: number | null;
}

export type PdfJobStatus = "idle" | "starting" | "running" | "cancelling" | "completed" | "cancelled" | "failed";

export interface PdfJobUiState {
  taskId: string | null;
  status: PdfJobStatus;
  stage: string | null;
  progress: PdfJobProgress | null;
  workerVersion: string | null;
  outputPdf: string | null;
  outputMode: string | null;
  pageCount: number | null;
  warnings: string[];
  tokenUsage: PdfJobTokenUsage | null;
  code: string | null;
  message: string | null;
}

export type PdfJobEvent =
  | {
    type: "started";
    taskId: string;
    workerVersion: string | null;
  }
  | {
    type: "stage";
    taskId: string;
    stage: string;
  }
  | {
    type: "progress";
    taskId: string;
    progress: PdfJobProgress;
  }
  | {
    type: "tokenUsage";
    taskId: string;
    translationRequestId: string | null;
    usage: PdfJobTokenUsage;
  }
  | {
    type: "warning";
    taskId: string;
    code: string;
    message: string;
  }
  | {
    type: "finished";
    taskId: string;
    outputPdf: string;
    outputMode: string | null;
    pageCount: number | null;
    warnings: string[];
  }
  | {
    type: "cancelled";
    taskId: string;
    reason: string | null;
  }
  | {
    type: "failed";
    taskId: string;
    code: string;
    message: string;
  };

export const PDF_ENGINE_EVENT_NAMES = [
  "pdf_engine_status_changed",
  "pdf_engine_prepare_started",
  "pdf_engine_prepare_progress",
  "pdf_engine_prepare_completed",
  "pdf_engine_prepare_failed",
] as const;

export const PDF_JOB_EVENT_NAMES = [
  "pdf_translation_started",
  "pdf_translation_stage",
  "pdf_translation_progress",
  "pdf_translation_token_usage",
  "pdf_translation_warning",
  "pdf_translation_finished",
  "pdf_translation_cancelled",
  "pdf_translation_failed",
] as const;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function stringValue(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function nonEmptyString(value: unknown): string | null {
  const result = stringValue(value);
  return result === null || result.trim().length === 0 ? null : result;
}

function readField(record: Record<string, unknown>, ...names: string[]): unknown {
  for (const name of names) {
    if (name in record) return record[name];
  }
  return undefined;
}

function optionalStringField(record: Record<string, unknown>, ...names: string[]): { valid: boolean; value: string | null } {
  const raw = readField(record, ...names);
  if (raw === undefined || raw === null) return { valid: true, value: null };
  const value = stringValue(raw);
  return value === null ? { valid: false, value: null } : { valid: true, value };
}

function optionalNonNegativeIntegerField(record: Record<string, unknown>, ...names: string[]): { valid: boolean; value: number | null } {
  const raw = readField(record, ...names);
  if (raw === undefined || raw === null) return { valid: true, value: null };
  return typeof raw === "number" && Number.isInteger(raw) && Number.isFinite(raw) && raw >= 0
    ? { valid: true, value: raw }
    : { valid: false, value: null };
}

function optionalFractionField(record: Record<string, unknown>, ...names: string[]): { valid: boolean; value: number | null } {
  const raw = readField(record, ...names);
  if (raw === undefined || raw === null) return { valid: true, value: null };
  if (typeof raw !== "number" || !Number.isFinite(raw) || raw < 0 || raw > 100) {
    return { valid: false, value: null };
  }
  return { valid: true, value: raw > 1 ? raw / 100 : raw };
}

function optionalOperationId(record: Record<string, unknown>): { valid: boolean; value: string | null } {
  return optionalStringField(record, "operationId", "operation_id");
}

function decodePdfEngineStatusKind(value: unknown): PdfEngineStatusKind | null {
  if (value === "missing" || value === "not_installed" || value === "unavailable") return "missing";
  if (value === "preparing" || value === "installing" || value === "updating") return "preparing";
  if (value === "ready" || value === "available") return "ready";
  if (value === "invalid" || value === "failed" || value === "error") return "invalid";
  return null;
}

function decodePdfEngineStatusPayload(value: unknown): PdfEngineStatus | null {
  if (!isRecord(value)) return null;
  const status = decodePdfEngineStatusKind(readField(value, "status", "state"));
  if (status === null) return null;
  const engineVersion = optionalStringField(value, "engineVersion", "engine_version");
  const target = optionalStringField(value, "target", "architecture");
  const pythonVersion = optionalStringField(value, "pythonVersion", "python_version");
  const babeldocVersion = optionalStringField(value, "babeldocVersion", "babeldoc_version");
  const error = optionalStringField(value, "error", "message");
  if (!engineVersion.valid || !target.valid || !pythonVersion.valid || !babeldocVersion.valid || !error.valid) return null;
  return {
    status,
    engineVersion: engineVersion.value,
    target: target.value,
    pythonVersion: pythonVersion.value,
    babeldocVersion: babeldocVersion.value,
    error: error.value,
  };
}

export function decodePdfEngineStatus(value: unknown): PdfEngineStatus | null {
  if (!isRecord(value)) return null;
  const nested = readField(value, "status");
  if (isRecord(nested)) return decodePdfEngineStatusPayload(nested);
  return decodePdfEngineStatusPayload(value);
}

export function decodePdfEngineEvent(name: string, value: unknown): PdfEngineEvent | null {
  if (!isRecord(value)) return null;

  if (name === "pdf_engine_status_changed") {
    const status = decodePdfEngineStatus(value);
    return status === null ? null : { type: "status", status };
  }

  const operationId = optionalOperationId(value);
  if (!operationId.valid) return null;

  if (name === "pdf_engine_prepare_started") {
    return { type: "prepareStarted", operationId: operationId.value };
  }

  if (name === "pdf_engine_prepare_progress") {
    const stage = optionalStringField(value, "stage");
    const current = optionalNonNegativeIntegerField(value, "current");
    const total = optionalNonNegativeIntegerField(value, "total");
    const fraction = optionalFractionField(value, "fraction", "progress");
    const message = optionalStringField(value, "message");
    if (!stage.valid || !current.valid || !total.valid || !fraction.valid || !message.valid) return null;
    return {
      type: "prepareProgress",
      progress: {
        operationId: operationId.value,
        stage: stage.value ?? "preparing",
        current: current.value,
        total: total.value,
        fraction: fraction.value,
        message: message.value,
      },
    };
  }

  if (name === "pdf_engine_prepare_completed") {
    const statusValue = readField(value, "status");
    const status = statusValue === undefined || statusValue === null
      ? null
      : typeof statusValue === "string"
        ? decodePdfEngineStatus({ status: statusValue })
        : decodePdfEngineStatus(statusValue);
    return statusValue !== undefined && statusValue !== null && status === null
      ? null
      : { type: "prepareCompleted", operationId: operationId.value, status };
  }

  if (name === "pdf_engine_prepare_failed") {
    const message = nonEmptyString(readField(value, "message", "error"));
    return message === null
      ? null
      : { type: "prepareFailed", operationId: operationId.value, message };
  }

  return null;
}

export function decodePdfTranslationStartResult(value: unknown): PdfTranslationStarted | null {
  if (!isRecord(value)) return null;
  const taskId = nonEmptyString(readField(value, "taskId", "task_id"));
  return taskId === null ? null : { taskId };
}

export const decodePdfTranslationStartedResult = decodePdfTranslationStartResult;

export function decodePdfTranslationCancelResult(value: unknown): boolean | null {
  return typeof value === "boolean" ? value : null;
}

function decodePdfJobProgress(value: Record<string, unknown>): PdfJobProgress | null {
  const stage = optionalStringField(value, "stage");
  const current = optionalNonNegativeIntegerField(value, "current");
  const total = optionalNonNegativeIntegerField(value, "total");
  const fraction = optionalFractionField(value, "fraction", "progress");
  const message = optionalStringField(value, "message");
  return !stage.valid || !current.valid || !total.valid || !fraction.valid || !message.valid
    ? null
    : {
      stage: stage.value ?? "engine",
      current: current.value,
      total: total.value,
      fraction: fraction.value,
      message: message.value,
    };
}

function decodePdfJobTokenUsage(value: unknown): PdfJobTokenUsage | null {
  if (!isRecord(value)) return null;
  const promptTokens = optionalNonNegativeIntegerField(value, "promptTokens", "prompt_tokens");
  const completionTokens = optionalNonNegativeIntegerField(value, "completionTokens", "completion_tokens");
  const totalTokens = optionalNonNegativeIntegerField(value, "totalTokens", "total_tokens");
  return !promptTokens.valid || !completionTokens.valid || !totalTokens.valid
    ? null
    : {
      promptTokens: promptTokens.value,
      completionTokens: completionTokens.value,
      totalTokens: totalTokens.value,
    };
}

function decodeStringArrayField(value: unknown): string[] | null {
  if (value === undefined || value === null) return [];
  return Array.isArray(value) && value.every((item) => typeof item === "string")
    ? value
    : null;
}

export function decodePdfJobEvent(name: string, value: unknown): PdfJobEvent | null {
  if (!isRecord(value)) return null;
  const taskId = nonEmptyString(readField(value, "taskId", "task_id"));
  if (taskId === null) return null;

  if (name === "pdf_translation_started") {
    const workerVersion = optionalStringField(value, "workerVersion", "worker_version");
    return workerVersion.valid
      ? { type: "started", taskId, workerVersion: workerVersion.value }
      : null;
  }

  if (name === "pdf_translation_stage") {
    const stage = nonEmptyString(readField(value, "stage"));
    if (stage !== null) return { type: "stage", taskId, stage };
    const workerVersion = optionalStringField(value, "workerVersion", "worker_version");
    return workerVersion.valid
      ? { type: "started", taskId, workerVersion: workerVersion.value }
      : null;
  }

  if (name === "pdf_translation_progress") {
    const progress = decodePdfJobProgress(value);
    return progress === null ? null : { type: "progress", taskId, progress };
  }

  if (name === "pdf_translation_token_usage") {
    const translationRequestId = optionalStringField(value, "translationRequestId", "translation_request_id");
    const usage = decodePdfJobTokenUsage(readField(value, "usage"));
    return !translationRequestId.valid || usage === null
      ? null
      : { type: "tokenUsage", taskId, translationRequestId: translationRequestId.value, usage };
  }

  if (name === "pdf_translation_warning") {
    const code = nonEmptyString(readField(value, "code"));
    const message = nonEmptyString(readField(value, "message"));
    return code === null || message === null ? null : { type: "warning", taskId, code, message };
  }

  if (name === "pdf_translation_finished") {
    const outputPdf = nonEmptyString(readField(value, "outputPdf", "output_pdf"));
    const outputMode = optionalStringField(value, "outputMode", "output_mode");
    const pageCount = optionalNonNegativeIntegerField(value, "pageCount", "page_count");
    const warnings = decodeStringArrayField(readField(value, "warnings"));
    if (outputPdf === null || !outputMode.valid || !pageCount.valid || warnings === null) return null;
    return {
      type: "finished",
      taskId,
      outputPdf,
      outputMode: outputMode.value,
      pageCount: pageCount.value,
      warnings,
    };
  }

  if (name === "pdf_translation_cancelled") {
    const reason = optionalStringField(value, "reason");
    return reason.valid ? { type: "cancelled", taskId, reason: reason.value } : null;
  }

  if (name === "pdf_translation_failed") {
    const code = nonEmptyString(readField(value, "code"));
    const message = nonEmptyString(readField(value, "message"));
    return code === null || message === null ? null : { type: "failed", taskId, code, message };
  }

  return null;
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
