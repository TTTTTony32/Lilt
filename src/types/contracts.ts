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
  distributionVersion: string | null;
  resourceSizeBytes: number | null;
  updating: boolean;
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
    status: PdfEngineStatus | null;
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

export interface DocumentTerm {
  source: string;
  target: string | null;
  sourceKind: string | null;
  confidence: number | null;
  note: string | null;
}

export type PdfTaskTerm = DocumentTerm;

export interface Abbreviation {
  abbreviation: string;
  expanded: string | null;
  target: string | null;
  confidence: number | null;
}

export type PdfTaskAbbreviation = Abbreviation;

export interface DocumentContext {
  schemaVersion: number;
  title: string | null;
  abstract: string | null;
  documentType: string | null;
  domain: string | null;
  headings: string[];
  keyTerms: DocumentTerm[];
  abbreviations: Abbreviation[];
  translationNotes: string[];
  contextHash: string | null;
}

export type PdfDocumentContext = DocumentContext;

export type PdfPreflightStatus = "idle" | "running" | "completed" | "degraded" | "failed";

export type PdfPreflightResponsePhase = "waiting" | "thinking" | "streaming";

export interface PdfPreflightState {
  requestId: string | null;
  responsePhase: PdfPreflightResponsePhase | null;
  status: PdfPreflightStatus;
  schemaVersion: number | null;
  context: DocumentContext | null;
  contextHash: string | null;
  warnings: string[];
  applied: boolean;
  message: string | null;
}

export type PdfPreflightUiState = PdfPreflightState;

export type PdfQualityDiagnosticSeverity = "info" | "warning" | "error";

export interface PdfQualityDiagnostic {
  severity: PdfQualityDiagnosticSeverity;
  ruleId: string | null;
  message: string;
  taskId: string | null;
  translationRequestId: string | null;
  segmentId: string | null;
  pageNumber: number | null;
}

export type QualityDiagnostic = PdfQualityDiagnostic;

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
  preflight?: PdfPreflightState;
  documentContext?: DocumentContext | null;
  diagnostics?: PdfQualityDiagnostic[];
}

export interface PdfJobContextMetadata {
  preflight?: PdfPreflightState;
  documentContext?: DocumentContext | null;
  diagnostics?: PdfQualityDiagnostic[];
}

export type PdfJobEvent =
  | {
    type: "started";
    taskId: string;
    workerVersion: string | null;
  } & PdfJobContextMetadata
  | {
    type: "stage";
    taskId: string;
    stage: string;
  } & PdfJobContextMetadata
  | {
    type: "progress";
    taskId: string;
    progress: PdfJobProgress;
  } & PdfJobContextMetadata
  | {
    type: "tokenUsage";
    taskId: string;
    translationRequestId: string | null;
    usage: PdfJobTokenUsage;
  } & PdfJobContextMetadata
  | {
    type: "warning";
    taskId: string;
    code: string;
    message: string;
    preflightRequestId?: string | null;
    diagnostic?: PdfQualityDiagnostic;
  } & PdfJobContextMetadata
  | {
    type: "finished";
    taskId: string;
    outputPdf: string;
    outputMode: string | null;
    pageCount: number | null;
    warnings: string[];
  } & PdfJobContextMetadata
  | {
    type: "cancelled";
    taskId: string;
    reason: string | null;
  } & PdfJobContextMetadata
  | {
    type: "failed";
    taskId: string;
    code: string;
    message: string;
  } & PdfJobContextMetadata
  | PdfPreflightEvent
  | {
    type: "diagnostic";
    taskId: string;
    diagnostic: PdfQualityDiagnostic;
  } & PdfJobContextMetadata;

export type PdfPreflightEvent =
  | {
    type: "preflightStarted";
    taskId: string;
    preflightRequestId: string | null;
    preflight: PdfPreflightState;
    diagnostics?: PdfQualityDiagnostic[];
  }
  | {
    type: "preflightActivity";
    taskId: string;
    preflightRequestId: string | null;
    phase: Exclude<PdfPreflightResponsePhase, "waiting">;
    preflight: PdfPreflightState;
    diagnostics?: PdfQualityDiagnostic[];
  }
  | {
    type: "preflightCompleted";
    taskId: string;
    preflightRequestId: string | null;
    preflight: PdfPreflightState;
    diagnostics?: PdfQualityDiagnostic[];
  }
  | {
    type: "preflightDegraded";
    taskId: string;
    preflightRequestId: string | null;
    preflight: PdfPreflightState;
    diagnostics?: PdfQualityDiagnostic[];
  }
  | {
    type: "preflightFailed";
    taskId: string;
    preflightRequestId: string | null;
    preflight: PdfPreflightState;
    diagnostics?: PdfQualityDiagnostic[];
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
  "pdf_translation_preflight_started",
  "pdf_translation_preflight_activity",
  "pdf_translation_preflight_completed",
  "pdf_translation_preflight_degraded",
  "pdf_translation_preflight_warning",
  "pdf_translation_preflight_failed",
  "pdf_translation_diagnostic",
  "pdf_translation_quality_diagnostic",
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

function optionalNonEmptyStringField(record: Record<string, unknown>, ...names: string[]): { valid: boolean; value: string | null } {
  const result = optionalStringField(record, ...names);
  return result.value !== null && result.value.trim().length === 0
    ? { valid: false, value: null }
    : result;
}

function optionalNonNegativeIntegerField(record: Record<string, unknown>, ...names: string[]): { valid: boolean; value: number | null } {
  const raw = readField(record, ...names);
  if (raw === undefined || raw === null) return { valid: true, value: null };
  return typeof raw === "number" && Number.isInteger(raw) && Number.isFinite(raw) && raw >= 0
    ? { valid: true, value: raw }
    : { valid: false, value: null };
}

function optionalBooleanField(record: Record<string, unknown>, ...names: string[]): { valid: boolean; value: boolean } {
  const raw = readField(record, ...names);
  if (raw === undefined || raw === null) return { valid: true, value: false };
  return typeof raw === "boolean" ? { valid: true, value: raw } : { valid: false, value: false };
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
  const distributionVersion = optionalStringField(value, "distributionVersion", "distribution_version");
  const resourceSizeBytes = optionalNonNegativeIntegerField(value, "resourceSizeBytes", "resource_size_bytes", "resourceSize");
  const updating = optionalBooleanField(value, "updating");
  const error = optionalStringField(value, "error", "message");
  if (!engineVersion.valid || !target.valid || !pythonVersion.valid || !babeldocVersion.valid
    || !distributionVersion.valid || !resourceSizeBytes.valid || !updating.valid || !error.valid) return null;
  return {
    status,
    engineVersion: engineVersion.value,
    target: target.value,
    pythonVersion: pythonVersion.value,
    babeldocVersion: babeldocVersion.value,
    distributionVersion: distributionVersion.value,
    resourceSizeBytes: resourceSizeBytes.value,
    updating: updating.value,
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
    const statusValue = readField(value, "status");
    const status = statusValue === undefined || statusValue === null
      ? null
      : typeof statusValue === "string"
        ? decodePdfEngineStatus({ status: statusValue })
        : decodePdfEngineStatus(statusValue);
    return message === null
      ? null
      : statusValue !== undefined && statusValue !== null && status === null
        ? null
        : { type: "prepareFailed", operationId: operationId.value, message, status };
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

function decodeStringList(value: unknown, allowScalar = false): string[] | null {
  if (value === undefined || value === null) return [];
  if (Array.isArray(value)) return value.every((item) => typeof item === "string") ? value : null;
  return allowScalar && typeof value === "string" ? [value] : null;
}

function optionalConfidenceField(record: Record<string, unknown>, ...names: string[]): { valid: boolean; value: number | null } {
  const raw = readField(record, ...names);
  if (raw === undefined || raw === null) return { valid: true, value: null };
  return typeof raw === "number" && Number.isFinite(raw) && raw >= 0 && raw <= 1
    ? { valid: true, value: raw }
    : { valid: false, value: null };
}

function hasField(record: Record<string, unknown>, ...names: string[]): boolean {
  return names.some((name) => name in record);
}

function decodeDocumentTerm(value: unknown): DocumentTerm | null {
  if (!isRecord(value)) return null;
  const source = nonEmptyString(readField(value, "source", "original", "term", "text"));
  const target = optionalStringField(value, "target", "translation", "suggestedTranslation", "suggested_translation");
  const sourceKind = optionalStringField(value, "sourceKind", "source_kind", "origin", "sourceType", "source_type");
  const confidence = optionalConfidenceField(value, "confidence", "score");
  const note = optionalStringField(value, "note", "reason");
  return source === null || !target.valid || !sourceKind.valid || !confidence.valid || !note.valid
    ? null
    : { source, target: target.value, sourceKind: sourceKind.value, confidence: confidence.value, note: note.value };
}

function decodeAbbreviation(value: unknown): Abbreviation | null {
  if (!isRecord(value)) return null;
  const abbreviation = nonEmptyString(readField(value, "abbreviation", "short", "shortForm", "short_form"));
  const expanded = optionalStringField(value, "expanded", "expansion", "expandedForm", "expanded_form");
  const target = optionalStringField(value, "target", "translation", "suggestedTranslation", "suggested_translation");
  const confidence = optionalConfidenceField(value, "confidence", "score");
  return abbreviation === null || !expanded.valid || !target.valid || !confidence.valid
    ? null
    : {
      abbreviation,
      expanded: expanded.value,
      target: target.value,
      confidence: confidence.value,
    };
}

export function decodeDocumentContext(value: unknown): DocumentContext | null {
  if (!isRecord(value)) return null;

  const rawSchemaVersion = readField(value, "schemaVersion", "schema_version", "version");
  const schemaVersion = rawSchemaVersion === undefined || rawSchemaVersion === null
    ? 1
    : typeof rawSchemaVersion === "number" && Number.isInteger(rawSchemaVersion) && rawSchemaVersion >= 1
      ? rawSchemaVersion
      : null;
  const title = optionalStringField(value, "title");
  const abstract = optionalStringField(value, "abstract", "summary");
  const documentType = optionalStringField(value, "documentType", "document_type", "type");
  const domain = optionalStringField(value, "domain", "field");
  const headings = decodeStringList(readField(value, "headings", "headingLevels", "heading_levels"));
  const rawTerms = readField(value, "keyTerms", "key_terms", "terms");
  const keyTerms = rawTerms === undefined || rawTerms === null
    ? []
    : Array.isArray(rawTerms) ? rawTerms.map(decodeDocumentTerm) : null;
  const rawAbbreviations = readField(value, "abbreviations", "abbreviationList", "abbreviation_list");
  const abbreviations = rawAbbreviations === undefined || rawAbbreviations === null
    ? []
    : Array.isArray(rawAbbreviations) ? rawAbbreviations.map(decodeAbbreviation) : null;
  const translationNotes = decodeStringList(readField(value, "translationNotes", "translation_notes", "notes"), true);
  const contextHash = optionalStringField(value, "contextHash", "context_hash", "hash");

  if (
    schemaVersion === null ||
    !title.valid ||
    !abstract.valid ||
    !documentType.valid ||
    !domain.valid ||
    headings === null ||
    keyTerms === null ||
    keyTerms.some((term) => term === null) ||
    abbreviations === null ||
    abbreviations.some((item) => item === null) ||
    translationNotes === null ||
    !contextHash.valid
  ) {
    return null;
  }

  return {
    schemaVersion,
    title: title.value,
    abstract: abstract.value,
    documentType: documentType.value,
    domain: domain.value,
    headings,
    keyTerms: keyTerms as DocumentTerm[],
    abbreviations: abbreviations as Abbreviation[],
    translationNotes,
    contextHash: contextHash.value,
  };
}

function decodePdfPreflightStatus(value: unknown, fallback: PdfPreflightStatus): PdfPreflightStatus {
  const normalized = typeof value === "string" ? value.toLowerCase() : value;
  if (normalized === "idle") return "idle";
  if (normalized === "running" || normalized === "pending" || normalized === "started" || normalized === "in_progress") return "running";
  if (normalized === "completed" || normalized === "complete" || normalized === "success" || normalized === "applied") return "completed";
  if (normalized === "degraded" || normalized === "fallback" || normalized === "warning" || normalized === "partial") return "degraded";
  if (normalized === "failed" || normalized === "error") return "failed";
  return fallback;
}

function decodePdfPreflightResponsePhase(value: unknown): PdfPreflightResponsePhase | null {
  if (typeof value !== "string") return null;
  const normalized = value.toLowerCase();
  if (normalized === "waiting" || normalized === "pending") return "waiting";
  if (normalized === "thinking" || normalized === "reasoning") return "thinking";
  if (normalized === "streaming" || normalized === "content" || normalized === "generating") return "streaming";
  return null;
}

function decodePdfContextFromPayload(value: Record<string, unknown>): DocumentContext | null {
  const rawContext = readField(value, "documentContext", "document_context", "context");
  if (rawContext !== undefined && rawContext !== null) return decodeDocumentContext(rawContext);
  if (rawContext === null) return null;
  if (!hasField(value, "title", "abstract", "summary", "documentType", "document_type", "domain", "headings", "keyTerms", "key_terms", "abbreviations", "translationNotes", "translation_notes", "contextHash", "context_hash")) {
    return null;
  }
  return decodeDocumentContext(value);
}

export function decodePdfPreflightState(value: unknown, fallbackStatus: PdfPreflightStatus = "running"): PdfPreflightState | null {
  if (!isRecord(value)) return null;
  const nested = readField(value, "preflight", "preflightState", "preflight_state");
  const payload = isRecord(nested) ? nested : value;
  const status = decodePdfPreflightStatus(
    readField(payload, "status", "state", "phase", "outcome", "preflightStatus", "preflight_status"),
    fallbackStatus,
  );
  const context = decodePdfContextFromPayload(payload);
  const contextWasProvided = hasField(payload, "documentContext", "document_context", "context", "title", "abstract", "summary", "documentType", "document_type", "domain", "headings", "keyTerms", "key_terms", "abbreviations", "translationNotes", "translation_notes", "contextHash", "context_hash");
  if (contextWasProvided && context === null && readField(payload, "context", "documentContext", "document_context") !== null) return null;

  const requestIdRaw = readField(payload, "preflightRequestId", "preflight_request_id", "requestId", "request_id")
    ?? (payload === value ? undefined : readField(value, "preflightRequestId", "preflight_request_id", "requestId", "request_id"));
  const requestId = requestIdRaw === undefined || requestIdRaw === null
    ? { valid: true, value: null }
    : optionalNonEmptyStringField({ requestId: requestIdRaw }, "requestId");
  const responsePhaseRaw = readField(payload, "responsePhase", "response_phase", "activityPhase", "activity_phase");
  const responsePhase = responsePhaseRaw === undefined || responsePhaseRaw === null
    ? null
    : decodePdfPreflightResponsePhase(responsePhaseRaw);

  const schemaVersionRaw = readField(payload, "schemaVersion", "schema_version");
  const schemaVersion = schemaVersionRaw === undefined || schemaVersionRaw === null
    ? context?.schemaVersion ?? null
    : typeof schemaVersionRaw === "number" && Number.isInteger(schemaVersionRaw) && schemaVersionRaw >= 1
      ? schemaVersionRaw
      : null;
  const contextHash = optionalStringField(payload, "contextHash", "context_hash", "hash");
  const warnings = decodeStringList(readField(payload, "warnings", "warning"), true);
  const message = optionalStringField(payload, "message", "error", "reason");
  const applied = optionalBooleanField(payload, "applied", "autoApplied", "auto_applied");
  const degraded = readField(payload, "degraded", "fallback");
  if (
    (schemaVersionRaw !== undefined && schemaVersion === null) ||
    !requestId.valid ||
    (responsePhaseRaw !== undefined && responsePhaseRaw !== null && responsePhase === null) ||
    !contextHash.valid ||
    warnings === null ||
    !message.valid ||
    !applied.valid ||
    (degraded !== undefined && degraded !== null && typeof degraded !== "boolean")
  ) return null;

  const normalizedContext = context && context.contextHash === null && contextHash.value !== null
    ? { ...context, contextHash: contextHash.value }
    : context;
  const isDegraded = degraded === true || status === "degraded";
  return {
    requestId: requestId.value,
    responsePhase,
    status: isDegraded ? "degraded" : status,
    schemaVersion,
    context: normalizedContext,
    contextHash: contextHash.value ?? normalizedContext?.contextHash ?? null,
    warnings,
    applied: applied.value || status === "completed" && !isDegraded && normalizedContext !== null,
    message: message.value,
  };
}

function decodePdfJobContextMetadata(value: Record<string, unknown>): PdfJobContextMetadata | null {
  const hasPreflight = hasField(value, "preflight", "preflightState", "preflight_state")
    || hasField(value, "documentContext", "document_context", "context")
    || hasField(value, "preflightStatus", "preflight_status", "preflightState", "preflight_state");
  const preflight = hasPreflight ? decodePdfPreflightState(value) : null;
  if (hasPreflight && preflight === null) return null;

  const hasContext = hasField(value, "documentContext", "document_context", "context");
  const documentContext = hasContext ? decodePdfContextFromPayload(value) : null;
  if (hasContext && documentContext === null && readField(value, "documentContext", "document_context", "context") !== null) return null;

  const hasDiagnostics = hasField(value, "diagnostics", "qualityDiagnostics", "quality_diagnostics");
  const rawDiagnostics = readField(value, "diagnostics", "qualityDiagnostics", "quality_diagnostics");
  const diagnostics = hasDiagnostics
    ? (rawDiagnostics === null || rawDiagnostics === undefined
      ? []
      : Array.isArray(rawDiagnostics) ? rawDiagnostics.map(decodePdfQualityDiagnostic) : null)
    : [];
  if (diagnostics === null || diagnostics.some((diagnostic) => diagnostic === null)) return null;

  const metadata: PdfJobContextMetadata = {};
  if (preflight) metadata.preflight = preflight;
  if (documentContext || (hasContext && readField(value, "documentContext", "document_context", "context") === null)) metadata.documentContext = documentContext;
  if (hasDiagnostics) metadata.diagnostics = diagnostics as PdfQualityDiagnostic[];
  return metadata;
}

function decodePdfDiagnosticSeverity(value: unknown): PdfQualityDiagnosticSeverity | null {
  if (value === "info" || value === "notice") return "info";
  if (value === "warning" || value === "warn") return "warning";
  if (value === "error" || value === "critical" || value === "fatal") return "error";
  return null;
}

export function decodePdfQualityDiagnostic(value: unknown): PdfQualityDiagnostic | null {
  if (!isRecord(value)) return null;
  const nested = readField(value, "diagnostic", "qualityDiagnostic", "quality_diagnostic");
  const payload = isRecord(nested) ? nested : value;
  const severity = decodePdfDiagnosticSeverity(readField(payload, "severity", "level"));
  const ruleId = optionalStringField(payload, "ruleId", "rule_id", "code", "kind");
  const message = nonEmptyString(readField(payload, "message", "description", "detail", "reason"));
  const taskId = optionalStringField(payload, "taskId", "task_id");
  const translationRequestId = optionalStringField(payload, "translationRequestId", "translation_request_id");
  const segmentId = optionalStringField(payload, "segmentId", "segment_id");
  const pageNumber = optionalNonNegativeIntegerField(payload, "pageNumber", "page_number", "page");
  return severity === null || message === null || !ruleId.valid || !taskId.valid || !translationRequestId.valid || !segmentId.valid || !pageNumber.valid
    ? null
    : {
      severity,
      ruleId: ruleId.value,
      message,
      taskId: taskId.value,
      translationRequestId: translationRequestId.value,
      segmentId: segmentId.value,
      pageNumber: pageNumber.value,
    };
}

export function decodePdfPreflightEvent(name: string, value: unknown): PdfPreflightEvent | null {
  if (!isRecord(value)) return null;
  const taskId = nonEmptyString(readField(value, "taskId", "task_id"));
  if (taskId === null) return null;

  const requestIdField = optionalNonEmptyStringField(value, "preflightRequestId", "preflight_request_id", "requestId", "request_id");
  if (!requestIdField.valid) return null;

  const eventType = name.includes("activity")
    ? "preflightActivity"
    : name.includes("started")
    ? "preflightStarted"
    : name.includes("completed")
      ? "preflightCompleted"
      : name.includes("degraded") || name.includes("warning")
        ? "preflightDegraded"
        : name.includes("failed")
          ? "preflightFailed"
          : null;
  if (eventType === null) return null;
  const fallbackStatus = eventType === "preflightStarted"
    ? "running"
    : eventType === "preflightActivity"
      ? "running"
    : eventType === "preflightCompleted"
      ? "completed"
      : eventType === "preflightDegraded"
        ? "degraded"
        : "failed";
  const preflight = decodePdfPreflightState(value, fallbackStatus);
  if (preflight === null) return null;
  const preflightRequestId = requestIdField.value ?? preflight.requestId;
  if (requestIdField.value !== null && preflight.requestId !== null && requestIdField.value !== preflight.requestId) return null;

  let normalizedPreflight: PdfPreflightState = {
    ...preflight,
    requestId: preflightRequestId,
    responsePhase: null,
  };
  if (eventType === "preflightStarted") {
    normalizedPreflight = { ...normalizedPreflight, status: "running", responsePhase: "waiting" };
  } else if (eventType === "preflightActivity") {
    const phase = decodePdfPreflightResponsePhase(readField(value, "phase", "responsePhase", "response_phase"));
    if (phase === null || phase === "waiting") return null;
    normalizedPreflight = { ...normalizedPreflight, status: "running", responsePhase: phase };
  } else {
    normalizedPreflight = {
      ...normalizedPreflight,
      status: eventType === "preflightCompleted"
        ? normalizedPreflight.status === "degraded" ? "degraded" : "completed"
        : eventType === "preflightDegraded" ? "degraded" : "failed",
      responsePhase: null,
    };
  }
  const rawDiagnostics = readField(value, "diagnostics", "qualityDiagnostics", "quality_diagnostics");
  const diagnostics = rawDiagnostics === undefined || rawDiagnostics === null
    ? undefined
    : Array.isArray(rawDiagnostics) ? rawDiagnostics.map(decodePdfQualityDiagnostic) : null;
  if (diagnostics === null || diagnostics?.some((diagnostic) => diagnostic === null)) return null;
  const normalizedEventType: PdfPreflightEvent["type"] = eventType === "preflightCompleted" && normalizedPreflight.status === "degraded"
    ? "preflightDegraded"
    : eventType;
  const base = {
    taskId,
    preflightRequestId,
    preflight: normalizedPreflight,
  } as const;
  if (normalizedEventType === "preflightActivity") {
    return diagnostics === undefined
      ? { ...base, type: "preflightActivity", phase: normalizedPreflight.responsePhase as Exclude<PdfPreflightResponsePhase, "waiting"> }
      : { ...base, type: "preflightActivity", phase: normalizedPreflight.responsePhase as Exclude<PdfPreflightResponsePhase, "waiting">, diagnostics: diagnostics as PdfQualityDiagnostic[] };
  }
  if (normalizedEventType === "preflightStarted") {
    return diagnostics === undefined
      ? { ...base, type: "preflightStarted" }
      : { ...base, type: "preflightStarted", diagnostics: diagnostics as PdfQualityDiagnostic[] };
  }
  if (normalizedEventType === "preflightCompleted") {
    return diagnostics === undefined
      ? { ...base, type: "preflightCompleted" }
      : { ...base, type: "preflightCompleted", diagnostics: diagnostics as PdfQualityDiagnostic[] };
  }
  if (normalizedEventType === "preflightDegraded") {
    return diagnostics === undefined
      ? { ...base, type: "preflightDegraded" }
      : { ...base, type: "preflightDegraded", diagnostics: diagnostics as PdfQualityDiagnostic[] };
  }
  return diagnostics === undefined
    ? { ...base, type: "preflightFailed" }
    : { ...base, type: "preflightFailed", diagnostics: diagnostics as PdfQualityDiagnostic[] };
}

export function decodePdfJobEvent(name: string, value: unknown): PdfJobEvent | null {
  if (!isRecord(value)) return null;
  const taskId = nonEmptyString(readField(value, "taskId", "task_id"));
  if (taskId === null) return null;

  if (name.includes("preflight")) return decodePdfPreflightEvent(name, value);

  const metadata = decodePdfJobContextMetadata(value);
  if (metadata === null) return null;

  if (name === "pdf_translation_started") {
    const workerVersion = optionalStringField(value, "workerVersion", "worker_version");
    return workerVersion.valid
      ? { type: "started", taskId, workerVersion: workerVersion.value, ...metadata }
      : null;
  }

  if (name === "pdf_translation_stage") {
    const stage = nonEmptyString(readField(value, "stage"));
    if (stage !== null) return { type: "stage", taskId, stage, ...metadata };
    const workerVersion = optionalStringField(value, "workerVersion", "worker_version");
    return workerVersion.valid
      ? { type: "started", taskId, workerVersion: workerVersion.value, ...metadata }
      : null;
  }

  if (name === "pdf_translation_progress") {
    const progress = decodePdfJobProgress(value);
    return progress === null ? null : { type: "progress", taskId, progress, ...metadata };
  }

  if (name === "pdf_translation_token_usage") {
    const translationRequestId = optionalStringField(value, "translationRequestId", "translation_request_id");
    const usage = decodePdfJobTokenUsage(readField(value, "usage"));
    return !translationRequestId.valid || usage === null
      ? null
      : { type: "tokenUsage", taskId, translationRequestId: translationRequestId.value, usage, ...metadata };
  }

  if (name === "pdf_translation_warning") {
    const code = nonEmptyString(readField(value, "code"));
    const message = nonEmptyString(readField(value, "message"));
    if (code === null || message === null) return null;
    const preflightRequestIdField = optionalNonEmptyStringField(value, "preflightRequestId", "preflight_request_id");
    if (!preflightRequestIdField.valid) return null;
    const preflightRequestMetadata = hasField(value, "preflightRequestId", "preflight_request_id")
      ? { preflightRequestId: preflightRequestIdField.value }
      : {};
    const rawDiagnostic = readField(value, "diagnostic", "qualityDiagnostic", "quality_diagnostic");
    const diagnostic = rawDiagnostic === undefined
      ? hasField(value, "severity", "level") ? decodePdfQualityDiagnostic(value) : undefined
      : decodePdfQualityDiagnostic(rawDiagnostic);
    return rawDiagnostic !== undefined && diagnostic === null || diagnostic === null
      ? null
      : diagnostic
        ? { type: "warning", taskId, code, message, diagnostic, ...preflightRequestMetadata, ...metadata }
        : { type: "warning", taskId, code, message, ...preflightRequestMetadata, ...metadata };
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
      ...metadata,
    };
  }

  if (name === "pdf_translation_cancelled") {
    const reason = optionalStringField(value, "reason");
    return reason.valid ? { type: "cancelled", taskId, reason: reason.value, ...metadata } : null;
  }

  if (name === "pdf_translation_failed") {
    const code = nonEmptyString(readField(value, "code"));
    const message = nonEmptyString(readField(value, "message"));
    return code === null || message === null ? null : { type: "failed", taskId, code, message, ...metadata };
  }

  if (name === "pdf_translation_diagnostic" || name === "pdf_translation_quality_diagnostic") {
    const rawDiagnostic = readField(value, "diagnostic", "qualityDiagnostic", "quality_diagnostic");
    const diagnostic = rawDiagnostic === undefined ? decodePdfQualityDiagnostic(value) : decodePdfQualityDiagnostic(rawDiagnostic);
    return diagnostic === null ? null : { type: "diagnostic", taskId, diagnostic, ...metadata };
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
