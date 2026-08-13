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
} from "./contracts";

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
