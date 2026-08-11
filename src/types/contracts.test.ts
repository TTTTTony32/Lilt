import { describe, expect, it } from "vitest";
import { decodeTranslationEvent } from "./contracts";

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
});
