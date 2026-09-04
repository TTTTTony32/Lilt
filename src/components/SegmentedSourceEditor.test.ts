import { describe, expect, it } from "vitest";
import {
  buildSegmentedSourceParts,
  EMPTY_SEGMENT_INTERACTION_STATE,
  getActiveSegmentId,
  reduceSegmentInteraction,
} from "../lib/segmented-source";
import { PARAGRAPH_LEARNING_PROTOCOL_VERSION, type ParagraphLearningResult } from "../types/contracts";

const learning: ParagraphLearningResult = {
  protocolVersion: PARAGRAPH_LEARNING_PROTOCOL_VERSION,
  segments: [
    {
      id: "segment-1",
      source: "前😀",
      translation: "译文一",
      explanation: "第一个解释",
      sourceStart: 0,
      sourceEnd: 3,
    },
    {
      id: "segment-2",
      source: "后文",
      translation: "译文二",
      explanation: "第二个解释",
      sourceStart: 3,
      sourceEnd: 5,
    },
  ],
};

describe("segmented source editor pure helpers", () => {
  it("converts UTF-16 ranges into ordered interactive parts", () => {
    expect(buildSegmentedSourceParts("前😀后文", learning)).toEqual([
      { type: "segment", segment: learning.segments[0] },
      { type: "segment", segment: learning.segments[1] },
    ]);
  });

  it("falls back to plain text when a result no longer matches the source", () => {
    expect(buildSegmentedSourceParts("前😀旧文", learning)).toEqual([{ type: "plain", text: "前😀旧文" }]);
    expect(buildSegmentedSourceParts("原文", null)).toEqual([{ type: "plain", text: "原文" }]);
  });

  it("keeps whitespace outside the normalized translation source interactive surface", () => {
    expect(buildSegmentedSourceParts("  前😀后文  ", learning)).toEqual([
      { type: "plain", text: "  " },
      { type: "segment", segment: learning.segments[0] },
      { type: "segment", segment: learning.segments[1] },
      { type: "plain", text: "  " },
    ]);
  });

  it("keeps keyboard focus active while allowing hover to temporarily take precedence", () => {
    const focused = reduceSegmentInteraction(EMPTY_SEGMENT_INTERACTION_STATE, { type: "focus", segmentId: "segment-1" });
    const hovered = reduceSegmentInteraction(focused, { type: "pointerEnter", segmentId: "segment-2" });
    expect(getActiveSegmentId(hovered)).toBe("segment-2");
    const afterLeave = reduceSegmentInteraction(hovered, { type: "pointerLeave", segmentId: "segment-2" });
    expect(getActiveSegmentId(afterLeave)).toBe("segment-1");
    const afterBlur = reduceSegmentInteraction(afterLeave, { type: "blur", segmentId: "segment-1" });
    expect(getActiveSegmentId(afterBlur)).toBeNull();
  });

  it("clears both temporary interaction sources on Escape", () => {
    const active = reduceSegmentInteraction(
      { hoveredSegmentId: "segment-1", focusedSegmentId: "segment-2" },
      { type: "escape" },
    );
    expect(active).toEqual(EMPTY_SEGMENT_INTERACTION_STATE);
  });
});
