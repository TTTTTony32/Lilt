export type DictionaryStatus = "notInstalled" | "ready" | "updating" | "failed";

export type MeaningPriority = "core" | "common" | "rare";

export interface DictionaryLanguage {
  code: string;
  name: string;
}

export interface DictionaryExample {
  text: string;
  translation: string;
}

export interface DictionaryMeaning {
  sense_id: string;
  priority: MeaningPriority;
  short_gloss: string | null;
  learner_explanation: string;
  usage_note: string | null;
  labels: string[];
  topics: string[];
  examples: DictionaryExample[];
}

export interface DictionaryForm {
  text: string;
  tags: string[];
  roman: string | null;
}

export interface DictionaryPronunciation {
  ipa: string | null;
  text: string | null;
  tags: string[];
}

export interface DictionaryRelation {
  type: string;
  word: string;
  lang_code: string | null;
}

export interface DictionaryPosGroup {
  pos: string;
  etymology_id: string | null;
  proper_name: boolean;
  summary: string;
  usage_note: string | null;
  forms: DictionaryForm[];
  pronunciations: DictionaryPronunciation[];
  relations: DictionaryRelation[];
  meanings: DictionaryMeaning[];
}

export interface DictionaryEtymology {
  etymology_id: string;
  text: string | null;
  pos_members: string[];
}

export interface DictionaryEntry {
  schema_version: string;
  entry_id: string;
  headword: string;
  normalized_headword: string;
  headword_language: DictionaryLanguage;
  definition_language: DictionaryLanguage;
  entry_type: string;
  headword_summary: string;
  memory_hook: string;
  study_notes: string[];
  etymology_note: string | null;
  etymologies: DictionaryEtymology[];
  pos_groups: DictionaryPosGroup[];
}

export interface DictionaryLookupResult {
  word: string;
  normalizedWord: string;
  canonicalWord: string;
  matchType: "exact" | "form";
  entry: DictionaryEntry;
}

export interface DictionaryLookupCandidate {
  canonicalWord: string;
  normalizedCanonicalWord: string;
}

export interface DictionarySuggestion {
  word: string;
  normalizedWord: string;
}

export interface ParagraphExample {
  exampleId: number;
  sourceText: string;
  createdAt: string;
}

export interface DictionaryHistoryEntry {
  normalizedWord: string;
  displayWord: string;
  lastQueriedAt: string;
  queryCount: number;
}

export interface DictionaryLookupCommandResult {
  lookup: DictionaryLookupResult | null;
  candidates: DictionaryLookupCandidate[];
  example: ParagraphExample | null;
  history: DictionaryHistoryEntry[];
}

export interface DictionaryState {
  status: DictionaryStatus;
  installedRelease: string | null;
  artifactSha256: string | null;
  entryCount: number | null;
  distributionSchemaVersion: string | null;
  sqliteSchemaVersion: string | null;
  installedAt: string | null;
  downloadedBytes: number;
  totalBytes: number;
  databaseBytes: number;
  cacheSizeBytes: number;
  error: string | null;
}

export interface DictionaryUpdateCommandResult {
  operationId: string;
  state: DictionaryState;
}

export interface DictionaryUpdateStarted {
  type: "started";
  operationId: string;
  state: DictionaryState;
}

export interface DictionaryDownloadProgress {
  type: "downloadProgress";
  operationId: string;
  downloadedBytes: number;
  totalBytes: number;
}

export interface DictionaryVerifyProgress {
  type: "verifyProgress";
  operationId: string;
  current: number;
  total: number;
}

export interface DictionaryExtractProgress {
  type: "extractProgress";
  operationId: string;
  current: number;
  total: number;
}

export interface DictionaryUpdateCompleted {
  type: "completed";
  operationId: string;
  state: DictionaryState;
}

export interface DictionaryUpdateFailed {
  type: "failed";
  operationId: string;
  message: string;
}

export type DictionaryUpdateEvent =
  | DictionaryUpdateStarted
  | DictionaryDownloadProgress
  | DictionaryVerifyProgress
  | DictionaryExtractProgress
  | DictionaryUpdateCompleted
  | DictionaryUpdateFailed;

export const DICTIONARY_EVENT_NAMES = [
  "dictionary_update_started",
  "dictionary_download_progress",
  "dictionary_verify_progress",
  "dictionary_extract_progress",
  "dictionary_update_completed",
  "dictionary_update_failed",
] as const;

export const DEFAULT_DICTIONARY_STATE: DictionaryState = {
  status: "notInstalled",
  installedRelease: null,
  artifactSha256: null,
  entryCount: null,
  distributionSchemaVersion: null,
  sqliteSchemaVersion: null,
  installedAt: null,
  downloadedBytes: 0,
  totalBytes: 0,
  databaseBytes: 0,
  cacheSizeBytes: 0,
  error: null,
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function stringValue(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function nullableStringValue(value: unknown): string | null | undefined {
  if (value === null) return null;
  return typeof value === "string" ? value : undefined;
}

function nonNegativeNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : null;
}

function stringArray(value: unknown): string[] | null {
  if (!Array.isArray(value) || !value.every((item) => typeof item === "string")) return null;
  return value;
}

function language(value: unknown): DictionaryLanguage | null {
  if (!isRecord(value)) return null;
  const code = stringValue(value.code);
  const name = stringValue(value.name);
  return code === null || name === null ? null : { code, name };
}

function example(value: unknown): DictionaryExample | null {
  if (!isRecord(value)) return null;
  const text = stringValue(value.text);
  const translation = stringValue(value.translation);
  return text === null || translation === null ? null : { text, translation };
}

function meaning(value: unknown): DictionaryMeaning | null {
  if (!isRecord(value)) return null;
  const senseId = stringValue(value.sense_id);
  const priority = value.priority;
  const shortGloss = nullableStringValue(value.short_gloss);
  const explanation = stringValue(value.learner_explanation);
  const usageNote = nullableStringValue(value.usage_note);
  const labels = stringArray(value.labels);
  const topics = stringArray(value.topics);
  const examples = Array.isArray(value.examples) ? value.examples.map(example) : null;
  if (
    senseId === null ||
    (priority !== "core" && priority !== "common" && priority !== "rare") ||
    shortGloss === undefined ||
    explanation === null ||
    usageNote === undefined ||
    labels === null ||
    topics === null ||
    examples === null ||
    examples.some((item) => item === null)
  ) {
    return null;
  }
  return {
    sense_id: senseId,
    priority,
    short_gloss: shortGloss,
    learner_explanation: explanation,
    usage_note: usageNote,
    labels,
    topics,
    examples: examples as DictionaryExample[],
  };
}

function pronunciation(value: unknown): DictionaryPronunciation | null {
  if (!isRecord(value)) return null;
  const ipa = nullableStringValue(value.ipa);
  const text = nullableStringValue(value.text);
  const tags = stringArray(value.tags);
  return ipa === undefined || text === undefined || tags === null ? null : { ipa, text, tags };
}

function form(value: unknown): DictionaryForm | null {
  if (!isRecord(value)) return null;
  const text = stringValue(value.text);
  const tags = stringArray(value.tags);
  const roman = nullableStringValue(value.roman);
  return text === null || tags === null || roman === undefined ? null : { text, tags, roman };
}

function relation(value: unknown): DictionaryRelation | null {
  if (!isRecord(value)) return null;
  const type = stringValue(value.type);
  const word = stringValue(value.word);
  const languageCode = nullableStringValue(value.lang_code);
  return type === null || word === null || languageCode === undefined
    ? null
    : { type, word, lang_code: languageCode };
}

function posGroup(value: unknown): DictionaryPosGroup | null {
  if (!isRecord(value)) return null;
  const pos = stringValue(value.pos);
  const etymologyId = nullableStringValue(value.etymology_id);
  const properName = value.proper_name;
  const summary = stringValue(value.summary);
  const usageNote = nullableStringValue(value.usage_note);
  const forms = Array.isArray(value.forms) ? value.forms.map(form) : null;
  const pronunciations = Array.isArray(value.pronunciations) ? value.pronunciations.map(pronunciation) : null;
  const relations = Array.isArray(value.relations) ? value.relations.map(relation) : null;
  const meanings = Array.isArray(value.meanings) ? value.meanings.map(meaning) : null;
  if (
    pos === null ||
    etymologyId === undefined ||
    typeof properName !== "boolean" ||
    summary === null ||
    usageNote === undefined ||
    forms === null ||
    forms.some((item) => item === null) ||
    pronunciations === null ||
    pronunciations.some((item) => item === null) ||
    relations === null ||
    relations.some((item) => item === null) ||
    meanings === null ||
    meanings.some((item) => item === null)
  ) {
    return null;
  }
  return {
    pos,
    etymology_id: etymologyId,
    proper_name: properName,
    summary,
    usage_note: usageNote,
    forms: forms as DictionaryForm[],
    pronunciations: pronunciations as DictionaryPronunciation[],
    relations: relations as DictionaryRelation[],
    meanings: meanings as DictionaryMeaning[],
  };
}

function etymology(value: unknown): DictionaryEtymology | null {
  if (!isRecord(value)) return null;
  const etymologyId = stringValue(value.etymology_id);
  const text = nullableStringValue(value.text);
  const posMembers = stringArray(value.pos_members);
  return etymologyId === null || text === undefined || posMembers === null
    ? null
    : { etymology_id: etymologyId, text, pos_members: posMembers };
}

export function decodeDictionaryState(value: unknown): DictionaryState | null {
  if (!isRecord(value)) return null;
  const status = value.status;
  const installedRelease = nullableStringValue(value.installedRelease);
  const artifactSha256 = nullableStringValue(value.artifactSha256);
  const entryCount = value.entryCount === null
    ? null
    : nonNegativeNumber(value.entryCount);
  const distributionSchemaVersion = nullableStringValue(value.distributionSchemaVersion);
  const sqliteSchemaVersion = nullableStringValue(value.sqliteSchemaVersion);
  const installedAt = nullableStringValue(value.installedAt);
  const downloadedBytes = nonNegativeNumber(value.downloadedBytes);
  const totalBytes = nonNegativeNumber(value.totalBytes);
  const databaseBytes = nonNegativeNumber(value.databaseBytes);
  const cacheSizeBytes = nonNegativeNumber(value.cacheSizeBytes);
  const error = nullableStringValue(value.error);
  if (
    (status !== "notInstalled" && status !== "ready" && status !== "updating" && status !== "failed") ||
    installedRelease === undefined ||
    artifactSha256 === undefined ||
    (entryCount === null && value.entryCount !== null) ||
    distributionSchemaVersion === undefined ||
    sqliteSchemaVersion === undefined ||
    installedAt === undefined ||
    downloadedBytes === null ||
    totalBytes === null ||
    databaseBytes === null ||
    cacheSizeBytes === null ||
    error === undefined
  ) {
    return null;
  }
  return {
    status,
    installedRelease,
    artifactSha256,
    entryCount,
    distributionSchemaVersion,
    sqliteSchemaVersion,
    installedAt,
    downloadedBytes,
    totalBytes,
    databaseBytes,
    cacheSizeBytes,
    error,
  };
}

export function decodeDictionaryHistoryEntry(value: unknown): DictionaryHistoryEntry | null {
  if (!isRecord(value)) return null;
  const normalizedWord = stringValue(value.normalizedWord);
  const displayWord = stringValue(value.displayWord);
  const lastQueriedAt = stringValue(value.lastQueriedAt);
  const queryCount = nonNegativeNumber(value.queryCount);
  return normalizedWord === null || displayWord === null || lastQueriedAt === null || queryCount === null
    ? null
    : { normalizedWord, displayWord, lastQueriedAt, queryCount };
}

export function decodeDictionaryEntry(value: unknown): DictionaryEntry | null {
  if (!isRecord(value)) return null;
  const schemaVersion = stringValue(value.schema_version);
  const entryId = stringValue(value.entry_id);
  const headword = stringValue(value.headword);
  const normalizedHeadword = stringValue(value.normalized_headword);
  const headwordLanguage = language(value.headword_language);
  const definitionLanguage = language(value.definition_language);
  const entryType = stringValue(value.entry_type);
  const headwordSummary = stringValue(value.headword_summary);
  const memoryHook = stringValue(value.memory_hook);
  const studyNotes = stringArray(value.study_notes);
  const etymologyNote = nullableStringValue(value.etymology_note);
  const etymologies = Array.isArray(value.etymologies) ? value.etymologies.map(etymology) : null;
  const posGroups = Array.isArray(value.pos_groups) ? value.pos_groups.map(posGroup) : null;
  if (
    schemaVersion === null ||
    entryId === null ||
    headword === null ||
    normalizedHeadword === null ||
    headwordLanguage === null ||
    definitionLanguage === null ||
    entryType === null ||
    headwordSummary === null ||
    memoryHook === null ||
    studyNotes === null ||
    etymologyNote === undefined ||
    etymologies === null ||
    etymologies.some((item) => item === null) ||
    posGroups === null ||
    posGroups.some((item) => item === null)
  ) {
    return null;
  }
  return {
    schema_version: schemaVersion,
    entry_id: entryId,
    headword,
    normalized_headword: normalizedHeadword,
    headword_language: headwordLanguage,
    definition_language: definitionLanguage,
    entry_type: entryType,
    headword_summary: headwordSummary,
    memory_hook: memoryHook,
    study_notes: studyNotes,
    etymology_note: etymologyNote,
    etymologies: etymologies as DictionaryEtymology[],
    pos_groups: posGroups as DictionaryPosGroup[],
  };
}

export function decodeDictionaryLookupResult(value: unknown): DictionaryLookupResult | null {
  if (!isRecord(value)) return null;
  const word = stringValue(value.word);
  const normalizedWord = stringValue(value.normalizedWord);
  const canonicalWord = stringValue(value.canonicalWord);
  const matchType = value.matchType;
  const entry = decodeDictionaryEntry(value.entry);
  return word === null || normalizedWord === null || canonicalWord === null ||
    (matchType !== "exact" && matchType !== "form") || entry === null
    ? null
    : { word, normalizedWord, canonicalWord, matchType, entry };
}

export function decodeDictionaryLookupCommandResult(value: unknown): DictionaryLookupCommandResult | null {
  if (!isRecord(value) || !Array.isArray(value.history) || !Array.isArray(value.candidates)) return null;
  const lookup = decodeDictionaryLookupResult(value.lookup);
  const candidates = value.candidates.map(decodeDictionaryLookupCandidate);
  const example = decodeParagraphExample(value.example);
  const history = value.history.map(decodeDictionaryHistoryEntry);
  if (
    history.some((item) => item === null) ||
    candidates.some((item) => item === null) ||
    (example === null && value.example !== null)
  ) return null;
  return {
    lookup,
    candidates: candidates as DictionaryLookupCandidate[],
    example,
    history: history as DictionaryHistoryEntry[],
  };
}

function decodeDictionaryLookupCandidate(value: unknown): DictionaryLookupCandidate | null {
  if (!isRecord(value)) return null;
  const canonicalWord = stringValue(value.canonicalWord);
  const normalizedCanonicalWord = stringValue(value.normalizedCanonicalWord);
  return canonicalWord === null || normalizedCanonicalWord === null
    ? null
    : { canonicalWord, normalizedCanonicalWord };
}

export function decodeDictionarySuggestions(value: unknown): DictionarySuggestion[] | null {
  if (!Array.isArray(value)) return null;
  const suggestions = value.map((item) => {
    if (!isRecord(item)) return null;
    const word = stringValue(item.word);
    const normalizedWord = stringValue(item.normalizedWord);
    return word === null || normalizedWord === null ? null : { word, normalizedWord };
  });
  return suggestions.some((item) => item === null) ? null : suggestions as DictionarySuggestion[];
}

function decodeParagraphExample(value: unknown): ParagraphExample | null {
  if (value === null || value === undefined) return null;
  if (!isRecord(value)) return null;
  const exampleId = nonNegativeNumber(value.exampleId);
  const sourceText = stringValue(value.sourceText);
  const createdAt = stringValue(value.createdAt);
  return exampleId === null || sourceText === null || createdAt === null
    ? null
    : { exampleId, sourceText, createdAt };
}

export function decodeDictionaryUpdateEvent(name: string, value: unknown): DictionaryUpdateEvent | null {
  if (!isRecord(value)) return null;
  const operationId = stringValue(value.operationId);
  if (operationId === null) return null;
  if (name === "dictionary_update_started") {
    const state = decodeDictionaryState(value.state);
    return state === null ? null : { type: "started", operationId, state };
  }
  if (name === "dictionary_download_progress") {
    const downloadedBytes = nonNegativeNumber(value.downloadedBytes);
    const totalBytes = nonNegativeNumber(value.totalBytes);
    return downloadedBytes === null || totalBytes === null
      ? null
      : { type: "downloadProgress", operationId, downloadedBytes, totalBytes };
  }
  if (name === "dictionary_verify_progress" || name === "dictionary_extract_progress") {
    const current = nonNegativeNumber(value.current);
    const total = nonNegativeNumber(value.total);
    if (current === null || total === null) return null;
    return name === "dictionary_verify_progress"
      ? { type: "verifyProgress", operationId, current, total }
      : { type: "extractProgress", operationId, current, total };
  }
  if (name === "dictionary_update_completed") {
    const state = decodeDictionaryState(value.state);
    return state === null ? null : { type: "completed", operationId, state };
  }
  if (name === "dictionary_update_failed") {
    const message = stringValue(value.message);
    return message === null ? null : { type: "failed", operationId, message };
  }
  return null;
}

export function decodeDictionaryCommandResult(value: unknown): DictionaryUpdateCommandResult | null {
  if (!isRecord(value)) return null;
  const operationId = stringValue(value.operationId);
  const state = decodeDictionaryState(value.state);
  return operationId === null || state === null ? null : { operationId, state };
}

export function splitMeaningsByPriority(group: DictionaryPosGroup): {
  visible: DictionaryMeaning[];
  hidden: DictionaryMeaning[];
} {
  const visible = group.meanings.filter((meaning) => meaning.priority !== "rare");
  if (visible.length === 0) return { visible: group.meanings, hidden: [] };
  return {
    visible,
    hidden: group.meanings.filter((meaning) => meaning.priority === "rare"),
  };
}

export function collectPronunciations(entry: DictionaryEntry): DictionaryPronunciation[] {
  const seen = new Set<string>();
  const result: DictionaryPronunciation[] = [];
  for (const group of entry.pos_groups) {
    for (const pronunciation of group.pronunciations) {
      const rendered = pronunciation.ipa ?? pronunciation.text;
      if (!rendered) continue;
      const key = `${pronunciation.tags.join("+")}|${rendered}`;
      if (seen.has(key)) continue;
      seen.add(key);
      result.push(pronunciation);
    }
  }
  return result;
}

export function groupRelationsByType(group: DictionaryPosGroup): Map<string, string[]> {
  const result = new Map<string, string[]>();
  for (const relation of group.relations) {
    const word = relation.word.trim();
    if (!word) continue;
    const words = result.get(relation.type);
    if (words) {
      if (!words.includes(word)) words.push(word);
    } else {
      result.set(relation.type, [word]);
    }
  }
  return result;
}

const POS_LABELS_ZH: Record<string, string> = {
  noun: "名词",
  verb: "动词",
  adjective: "形容词",
  adj: "形容词",
  adverb: "副词",
  adv: "副词",
  pronoun: "代词",
  pron: "代词",
  preposition: "介词",
  prep: "介词",
  conjunction: "连词",
  conj: "连词",
  interjection: "感叹词",
  intj: "感叹词",
  determiner: "限定词",
  det: "限定词",
  article: "冠词",
  numeral: "数词",
  num: "数词",
  particle: "助词",
  name: "专有名词",
  phrase: "短语",
  proverb: "谚语",
  prefix: "前缀",
  suffix: "后缀",
  affix: "词缀",
  abbreviation: "缩写",
  symbol: "符号",
  character: "字符",
};

export function posLabelZh(pos: string): string | null {
  return POS_LABELS_ZH[pos.toLowerCase()] ?? null;
}
