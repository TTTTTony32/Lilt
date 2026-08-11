import { describe, expect, it } from "vitest";
import { decodeTranslationCommandResult, decodeTranslationEvent } from "./contracts";

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
