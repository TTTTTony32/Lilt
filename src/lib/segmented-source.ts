import type { ParagraphLearningResult, ParagraphLearningSegment } from "../types/contracts";

export interface SegmentedSourcePartPlain {
  type: "plain";
  text: string;
}

export interface SegmentedSourcePartSegment {
  type: "segment";
  segment: ParagraphLearningSegment;
}

export type SegmentedSourcePart = SegmentedSourcePartPlain | SegmentedSourcePartSegment;

export function buildSegmentedSourceParts(
  sourceText: string,
  learning: ParagraphLearningResult | null,
): SegmentedSourcePart[] {
  if (!learning || learning.segments.length === 0) return [{ type: "plain", text: sourceText }];

  const normalizedSource = sourceText.trim();
  if (!normalizedSource) return [{ type: "plain", text: sourceText }];
  const leadingLength = sourceText.length - sourceText.trimStart().length;
  const ordered = [...learning.segments].sort(
    (left, right) => left.sourceStart - right.sourceStart || left.sourceEnd - right.sourceEnd,
  );
  const parts: SegmentedSourcePart[] = [];
  let cursor = 0;

  if (leadingLength > 0) parts.push({ type: "plain", text: sourceText.slice(0, leadingLength) });
  for (const segment of ordered) {
    if (
      segment.sourceStart !== cursor ||
      segment.sourceEnd > normalizedSource.length ||
      segment.sourceEnd <= segment.sourceStart ||
      normalizedSource.slice(segment.sourceStart, segment.sourceEnd) !== segment.source
    ) {
      return [{ type: "plain", text: sourceText }];
    }
    if (segment.sourceStart > cursor) {
      parts.push({ type: "plain", text: normalizedSource.slice(cursor, segment.sourceStart) });
    }
    parts.push({ type: "segment", segment });
    cursor = segment.sourceEnd;
  }
  if (cursor !== normalizedSource.length) return [{ type: "plain", text: sourceText }];

  const trailingStart = leadingLength + normalizedSource.length;
  if (trailingStart < sourceText.length) {
    parts.push({ type: "plain", text: sourceText.slice(trailingStart) });
  }
  return parts;
}

export interface SegmentInteractionState {
  hoveredSegmentId: string | null;
  focusedSegmentId: string | null;
}

export const EMPTY_SEGMENT_INTERACTION_STATE: SegmentInteractionState = {
  hoveredSegmentId: null,
  focusedSegmentId: null,
};

export type SegmentInteractionAction =
  | { type: "pointerEnter"; segmentId: string }
  | { type: "pointerLeave"; segmentId: string }
  | { type: "focus"; segmentId: string }
  | { type: "blur"; segmentId: string }
  | { type: "escape" }
  | { type: "reset" };

export function getActiveSegmentId(state: SegmentInteractionState): string | null {
  return state.hoveredSegmentId ?? state.focusedSegmentId;
}

export function reduceSegmentInteraction(
  state: SegmentInteractionState,
  action: SegmentInteractionAction,
): SegmentInteractionState {
  switch (action.type) {
    case "pointerEnter":
      return state.hoveredSegmentId === action.segmentId
        ? state
        : { ...state, hoveredSegmentId: action.segmentId };
    case "pointerLeave":
      return state.hoveredSegmentId === action.segmentId
        ? { ...state, hoveredSegmentId: null }
        : state;
    case "focus":
      return state.focusedSegmentId === action.segmentId
        ? state
        : { ...state, focusedSegmentId: action.segmentId };
    case "blur":
      return state.focusedSegmentId === action.segmentId
        ? { ...state, focusedSegmentId: null }
        : state;
    case "escape":
    case "reset":
      return state.hoveredSegmentId === null && state.focusedSegmentId === null
        ? state
        : EMPTY_SEGMENT_INTERACTION_STATE;
  }
}
