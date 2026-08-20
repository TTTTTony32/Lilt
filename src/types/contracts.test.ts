import { describe, expect, it } from "vitest";
import {
  decodeSelectionTriggerNotice,
  decodeSelectionNotice,
  decodeSelectionRequest,
  decodeSelectionStatus,
  decodeSelectionUnavailable,
  decodeTranslationCommandResult,
  decodeTranslationEvent,
  decodePrompt,
  decodePersonalDictionaryExportResult,
  decodeGlossaryExportResult,
  decodeGlossaryImportResult,
  decodeDocumentContext,
  decodePdfJobEvent,
  decodePdfPreflightEvent,
  decodePdfQualityDiagnostic,
} from "./contracts";
import {
  createEmptyPdfPreflightState,
  reducePdfPreflightEvent,
  reducePdfPreflightWarning,
} from "../lib/pdf-preflight";

describe("selection contract", () => {
  it("decodes notices, requests, and runtime status", () => {
    expect(decodeSelectionTriggerNotice({
      triggerId: "trigger-1",
      trigger: "shortcut",
      anchor: null,
    })).toEqual({
      triggerId: "trigger-1",
      trigger: "shortcut",
      anchor: null,
    });
    expect(decodeSelectionNotice({
      requestId: "selection-1",
      triggerId: "trigger-1",
      trigger: "shortcut",
      anchor: { x: 10, y: 20, width: 30, height: 12 },
    })).toEqual({
      requestId: "selection-1",
      triggerId: "trigger-1",
      trigger: "shortcut",
      anchor: { x: 10, y: 20, width: 30, height: 12 },
    });
    expect(decodeSelectionRequest({
      requestId: "selection-1",
      sourceText: "hello",
      sourceLanguage: "en",
      targetLanguage: "zh-CN",
      trigger: "shortcut",
      anchor: null,
    })?.sourceText).toBe("hello");
    expect(decodeSelectionStatus({
      mode: "automatic",
      shortcut: "Ctrl+Shift+L",
      shortcutRegistered: false,
      uiAutomationReady: true,
      message: null,
    })?.uiAutomationReady).toBe(true);
  });

  it("rejects malformed selection payloads and preserves unavailable errors", () => {
    expect(decodeSelectionTriggerNotice({ triggerId: "trigger-1", trigger: "shortcut", anchor: { x: 1 } })).toBeNull();
    expect(decodeSelectionNotice({ requestId: "selection-1", trigger: "shortcut", anchor: { x: "10" } })).toBeNull();
    expect(decodeSelectionNotice({ requestId: "selection-1", trigger: "shortcut", anchor: null })).toBeNull();
    expect(decodeSelectionRequest({ requestId: "selection-1", sourceText: "hello" })).toBeNull();
    expect(decodeSelectionUnavailable({
      requestId: null,
      triggerId: "trigger-1",
      trigger: "automatic",
      code: "unsupported_control",
      message: "不支持读取",
    })).toEqual({
      requestId: null,
      triggerId: "trigger-1",
      trigger: "automatic",
      code: "unsupported_control",
      message: "不支持读取",
    });
    expect(decodeSelectionUnavailable({
      requestId: null,
      trigger: "automatic",
      code: "unsupported_control",
      message: "不支持读取",
    })).toBeNull();
    expect(decodeSelectionStatus({ mode: "automatic", shortcut: "Ctrl+Shift+L" })).toBeNull();
  });
});

describe("translation event contract", () => {
  it("decodes a delta event at the boundary", () => {
    expect(decodeTranslationEvent("translation_delta", { requestId: "req-1", content: "译文" })).toEqual({
      type: "delta",
      requestId: "req-1",
      content: "译文",
    });
  });

  it("rejects payloads that do not satisfy the event contract", () => {
    expect(decodeTranslationEvent("translation_delta", { requestId: "req-1", content: 42 })).toBeNull();
    expect(decodeTranslationEvent("translation_completed", { content: "译文" })).toBeNull();
    expect(decodeTranslationEvent("unknown_event", { requestId: "req-1" })).toBeNull();
  });

  it("preserves cache-hit state on completion", () => {
    expect(decodeTranslationEvent("translation_completed", {
      requestId: "req-2",
      content: "缓存译文",
      cacheHit: true,
    })).toEqual({
      type: "completed",
      requestId: "req-2",
      content: "缓存译文",
      cacheHit: true,
    });
  });

  it("requires the completed event cache flag to be boolean", () => {
    expect(decodeTranslationEvent("translation_completed", {
      requestId: "req-3",
      content: "译文",
      cacheHit: "true",
    })).toBeNull();
  });
});

describe("translation command result contract", () => {
  it("decodes completed, cancelled, and failed results", () => {
    expect(decodeTranslationCommandResult({
      outcome: "completed",
      content: "完整译文",
      cacheHit: true,
      message: null,
    })).toEqual({
      outcome: "completed",
      content: "完整译文",
      cacheHit: true,
      message: null,
    });
    expect(decodeTranslationCommandResult({
      outcome: "cancelled",
      content: null,
      cacheHit: false,
      message: null,
    })).toEqual({
      outcome: "cancelled",
      content: null,
      cacheHit: false,
      message: null,
    });
    expect(decodeTranslationCommandResult({
      outcome: "failed",
      content: null,
      cacheHit: false,
      message: "Provider 请求失败",
    })).toEqual({
      outcome: "failed",
      content: null,
      cacheHit: false,
      message: "Provider 请求失败",
    });
  });

  it("rejects incomplete or invalid terminal results", () => {
    expect(decodeTranslationCommandResult({
      content: "译文",
      cacheHit: false,
      message: null,
    })).toBeNull();
    expect(decodeTranslationCommandResult({
      outcome: "completed",
      content: 42,
      cacheHit: false,
      message: null,
    })).toBeNull();
    expect(decodeTranslationCommandResult({
      outcome: "failed",
      content: null,
      cacheHit: false,
      message: 42,
    })).toBeNull();
    expect(decodeTranslationCommandResult({
      outcome: "unknown",
      content: null,
      cacheHit: false,
      message: null,
    })).toBeNull();
  });
});

describe("prompt contract", () => {
  it("decodes editable and builtin prompts", () => {
    expect(decodePrompt({
      id: "custom-1",
      name: "技术翻译",
      content: "只输出译文",
      version: 2,
      isBuiltin: false,
    })).toEqual({
      id: "custom-1",
      name: "技术翻译",
      content: "只输出译文",
      version: 2,
      isBuiltin: false,
    });
  });

  it("rejects malformed prompt results", () => {
    expect(decodePrompt({ id: "p", name: "缺正文", version: 1, isBuiltin: true })).toBeNull();
    expect(decodePrompt({ id: "p", name: "错误版本", content: "正文", version: 1.5, isBuiltin: true })).toBeNull();
  });
});

describe("dictionary and glossary transfer contracts", () => {
  it("decodes successful personal dictionary exports", () => {
    expect(decodePersonalDictionaryExportResult({
      entryCount: 2,
      fileName: "lilt-personal-dictionary.txt",
    })).toEqual({
      entryCount: 2,
      fileName: "lilt-personal-dictionary.txt",
    });
    expect(decodePersonalDictionaryExportResult({ entryCount: -1, fileName: "words.txt" })).toBeNull();
  });

  it("decodes successful glossary exports", () => {
    expect(decodeGlossaryExportResult({
      entryCount: 3,
      fileName: "lilt-glossary.csv",
    })).toEqual({
      entryCount: 3,
      fileName: "lilt-glossary.csv",
    });
    expect(decodeGlossaryExportResult({ entryCount: 1, fileName: "" })).toBeNull();
  });

  it("decodes glossary import counts and skipped rows", () => {
    expect(decodeGlossaryImportResult({
      addedCount: 2,
      updatedCount: 1,
      skippedCount: 1,
      skippedRows: [{ line: 4, reason: "译文不能为空" }],
    })).toEqual({
      addedCount: 2,
      updatedCount: 1,
      skippedCount: 1,
      skippedRows: [{ line: 4, reason: "译文不能为空" }],
    });
    expect(decodeGlossaryImportResult({
      addedCount: 2,
      updatedCount: 1,
      skippedCount: 2,
      skippedRows: [{ line: 4, reason: "译文不能为空" }],
    })).toBeNull();
  });
});

describe("PDF document context and quality contracts", () => {
  it("decodes a versioned document context with snake_case worker fields", () => {
    expect(decodeDocumentContext({
      schema_version: 1,
      title: "A PDF context",
      abstract: "摘要",
      document_type: "academic_paper",
      domain: "machine_learning",
      headings: ["Introduction", "Method"],
      key_terms: [{
        source: "federated learning",
        target: "联邦学习",
        source_kind: "preflight",
        confidence: 0.92,
        note: "保留术语译法",
      }],
      abbreviations: [{
        abbreviation: "FL",
        expanded: "federated learning",
        target: "联邦学习",
        confidence: 0.88,
      }],
      translation_notes: ["保留模型名称"],
      context_hash: "ctx-1",
    })).toEqual({
      schemaVersion: 1,
      title: "A PDF context",
      abstract: "摘要",
      documentType: "academic_paper",
      domain: "machine_learning",
      headings: ["Introduction", "Method"],
      keyTerms: [{
        source: "federated learning",
        target: "联邦学习",
        sourceKind: "preflight",
        confidence: 0.92,
        note: "保留术语译法",
      }],
      abbreviations: [{
        abbreviation: "FL",
        expanded: "federated learning",
        target: "联邦学习",
        confidence: 0.88,
      }],
      translationNotes: ["保留模型名称"],
      contextHash: "ctx-1",
    });
  });

  it("decodes preflight completion and keeps degraded results non-blocking", () => {
    expect(decodePdfPreflightEvent("pdf_translation_preflight_completed", {
      task_id: "task-1",
      preflight_request_id: "preflight-1",
      schema_version: 1,
      document_context: { title: "标题", key_terms: [], abbreviations: [] },
      context_hash: "ctx-2",
      applied: true,
    })).toEqual({
      type: "preflightCompleted",
      taskId: "task-1",
      preflightRequestId: "preflight-1",
      preflight: {
        requestId: "preflight-1",
        responsePhase: null,
        status: "completed",
        schemaVersion: 1,
        context: {
          schemaVersion: 1,
          title: "标题",
          abstract: null,
          documentType: null,
          domain: null,
          headings: [],
          keyTerms: [],
          abbreviations: [],
          translationNotes: [],
          contextHash: "ctx-2",
        },
        contextHash: "ctx-2",
        warnings: [],
        applied: true,
        message: null,
      },
    });
    expect(decodePdfPreflightEvent("pdf_translation_preflight_warning", {
      taskId: "task-1",
      warnings: ["预检输出被截断"],
      degraded: true,
    })?.type).toBe("preflightDegraded");
    const degraded = decodePdfPreflightEvent("pdf_translation_preflight_completed", {
      taskId: "task-1",
      preflightRequestId: "preflight-1",
      context: { title: "标题", key_terms: [], abbreviations: [] },
      context_hash: "ctx-3",
      warnings: ["Provider 未返回完整结构"],
      degraded: true,
    });
    expect(degraded?.preflight.status).toBe("degraded");
    expect(degraded?.preflight.applied).toBe(false);
  });

  it("decodes preflight activity phases and protects terminal state from late events", () => {
    const started = decodePdfPreflightEvent("pdf_translation_preflight_started", {
      taskId: "task-1",
      preflightRequestId: "preflight-1",
    });
    const activity = decodePdfPreflightEvent("pdf_translation_preflight_activity", {
      task_id: "task-1",
      preflight_request_id: "preflight-1",
      phase: "thinking",
    });
    const degraded = decodePdfPreflightEvent("pdf_translation_preflight_degraded", {
      taskId: "task-1",
      preflightRequestId: "preflight-1",
      warnings: ["未收到完整预检结果"],
      degraded: true,
    });
    const lateCompleted = decodePdfPreflightEvent("pdf_translation_preflight_completed", {
      taskId: "task-1",
      preflightRequestId: "preflight-1",
      documentContext: { title: "迟到上下文", key_terms: [], abbreviations: [] },
      applied: true,
    });
    expect(started?.preflight.responsePhase).toBe("waiting");
    expect(activity).toMatchObject({
      type: "preflightActivity",
      preflightRequestId: "preflight-1",
      phase: "thinking",
      preflight: { requestId: "preflight-1", status: "running", responsePhase: "thinking" },
    });
    expect(degraded).not.toBeNull();
    expect(lateCompleted).not.toBeNull();

    const initialJob = {
      taskId: "task-1",
      status: "running" as const,
      stage: null,
      progress: null,
      workerVersion: null,
      outputPdf: null,
      outputMode: null,
      pageCount: null,
      warnings: [],
      tokenUsage: null,
      code: null,
      message: null,
      preflight: createEmptyPdfPreflightState(),
      documentContext: null,
      diagnostics: [],
    };
    const runningJob = reducePdfPreflightEvent(initialJob, started!);
    const thinkingJob = reducePdfPreflightEvent(runningJob, activity!);
    const degradedJob = reducePdfPreflightEvent(thinkingJob, degraded!);
    expect(thinkingJob.preflight?.responsePhase).toBe("thinking");
    expect(degradedJob.preflight?.status).toBe("degraded");
    expect(reducePdfPreflightWarning(thinkingJob, {
      type: "warning",
      taskId: "task-1",
      code: "document_preflight_warning",
      message: "旧协议警告",
    })).toBe(thinkingJob);
    expect(reducePdfPreflightEvent(degradedJob, lateCompleted!)).toBe(degradedJob);
  });

  it("decodes diagnostics and preserves old PDF events without new fields", () => {
    expect(decodePdfQualityDiagnostic({
      level: "warning",
      rule_id: "placeholder_missing",
      description: "占位符数量不一致",
      page_number: 3,
      segment_id: "p3-s2",
    })).toEqual({
      severity: "warning",
      ruleId: "placeholder_missing",
      message: "占位符数量不一致",
      taskId: null,
      translationRequestId: null,
      segmentId: "p3-s2",
      pageNumber: 3,
    });
    expect(decodePdfJobEvent("pdf_translation_started", {
      taskId: "task-1",
      workerVersion: "worker-1",
    })).toEqual({
      type: "started",
      taskId: "task-1",
      workerVersion: "worker-1",
    });
    expect(decodePdfJobEvent("pdf_translation_diagnostic", {
      task_id: "task-1",
      severity: "error",
      rule_id: "empty_translation",
      message: "译文为空",
    })?.type).toBe("diagnostic");
  });

  it("rejects malformed context entries instead of applying partial constraints", () => {
    expect(decodeDocumentContext({ key_terms: [{ source: "term", confidence: "high" }] })).toBeNull();
    expect(decodePdfQualityDiagnostic({ severity: "unknown", message: "诊断" })).toBeNull();
  });
});
