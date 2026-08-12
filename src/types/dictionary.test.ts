import { describe, expect, it } from "vitest";
import {
  decodeDictionaryEntry,
  decodeDictionaryLookupCommandResult,
  decodeDictionaryLookupResult,
  decodeDictionaryState,
  decodeDictionaryUpdateEvent,
  splitMeaningsByPriority,
} from "./dictionary";

const state = {
  status: "ready",
  installedRelease: "v2.0",
  artifactSha256: "a".repeat(64),
  entryCount: 84212,
  distributionSchemaVersion: "distribution_entry_v5",
  sqliteSchemaVersion: "distribution_sqlite_v1",
  installedAt: "2026-08-12T00:00:00Z",
  downloadedBytes: 216962932,
  totalBytes: 216962932,
  databaseBytes: 906096640,
  cacheSizeBytes: 906096640,
  error: null,
};

const entry = {
  schema_version: "distribution_entry_v5",
  entry_id: "fixture-resolve",
  headword: "resolve",
  normalized_headword: "resolve",
  headword_language: { code: "en", name: "English" },
  definition_language: { code: "zh-Hans", name: "Chinese (Simplified)" },
  entry_type: "standard",
  headword_summary: "解决",
  memory_hook: "",
  study_notes: [],
  etymology_note: null,
  etymologies: [],
  pos_groups: [{
    pos: "verb",
    etymology_id: null,
    proper_name: false,
    summary: "解决问题",
    usage_note: null,
    forms: [],
    pronunciations: [],
    relations: [{ type: "synonym", word: "solve", lang_code: "en" }],
    meanings: [{
      sense_id: "s1",
      priority: "core",
      short_gloss: "解决",
      learner_explanation: "使问题得到解决",
      usage_note: null,
      labels: [],
      topics: [],
      examples: [{ text: "Resolve the issue.", translation: "解决这个问题。" }],
    }],
  }],
};

const lookup = { word: "RESOLVE", normalizedWord: "resolve", entry };
const historyEntry = {
  normalizedWord: "resolve",
  displayWord: "RESOLVE",
  lastQueriedAt: "2026-08-12T00:00:00Z",
  queryCount: 2,
};
const history = [historyEntry];

describe("dictionary contracts", () => {
  it("decodes a validated local lookup result", () => {
    expect(decodeDictionaryLookupResult(lookup)).toEqual(lookup);
  });

  it("decodes a combined lookup result and latest history", () => {
    expect(decodeDictionaryLookupCommandResult({ lookup, history })).toEqual({ lookup, history });
  });

  it("rejects missing or malformed lookup and history fields", () => {
    expect(decodeDictionaryLookupCommandResult({ history })).toBeNull();
    expect(decodeDictionaryLookupCommandResult({ lookup: { ...lookup, entry: null }, history })).toBeNull();
    expect(decodeDictionaryLookupCommandResult({ lookup })).toBeNull();
    expect(decodeDictionaryLookupCommandResult({ lookup, history: {} })).toBeNull();
    expect(decodeDictionaryLookupCommandResult({
      lookup,
      history: [{ ...historyEntry, queryCount: "2" }],
    })).toBeNull();
  });

  it("rejects malformed state and update payloads", () => {
    expect(decodeDictionaryState({ ...state, downloadedBytes: "large" })).toBeNull();
    expect(decodeDictionaryUpdateEvent("dictionary_download_progress", {
      operationId: "op-1",
      downloadedBytes: 10,
      totalBytes: "unknown",
    })).toBeNull();
  });

  it("requires nullable fields to be present with a valid string or null", () => {
    const missingNullableField = { ...entry };
    Reflect.deleteProperty(missingNullableField, "etymology_note");
    expect(decodeDictionaryEntry(missingNullableField)).toBeNull();
    expect(decodeDictionaryEntry({ ...entry, etymology_note: 42 })).toBeNull();
    expect(decodeDictionaryEntry({ ...entry, etymology_note: null })).toEqual(entry);
    expect(decodeDictionaryState({ ...state, installedAt: undefined })).toBeNull();
    expect(decodeDictionaryState({ ...state, installedAt: 42 })).toBeNull();
  });

  it("keeps rare-only groups visible after priority filtering", () => {
    const decodedEntry = decodeDictionaryEntry(entry);
    if (!decodedEntry) throw new Error("fixture should satisfy dictionary entry contract");
    const firstGroup = decodedEntry.pos_groups[0];
    if (!firstGroup) throw new Error("fixture should include one part-of-speech group");
    const group = {
      ...firstGroup,
      meanings: firstGroup.meanings.map((meaning) => ({ ...meaning, priority: "rare" as const })),
    };
    expect(splitMeaningsByPriority(group)).toEqual({ visible: group.meanings, hidden: [] });
  });

  it("decodes update progress with its operation id", () => {
    expect(decodeDictionaryUpdateEvent("dictionary_download_progress", {
      operationId: "op-1",
      downloadedBytes: 10,
      totalBytes: 100,
    })).toEqual({
      type: "downloadProgress",
      operationId: "op-1",
      downloadedBytes: 10,
      totalBytes: 100,
    });
  });
});
