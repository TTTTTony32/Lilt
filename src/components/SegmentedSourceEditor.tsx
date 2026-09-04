import { useCallback, useEffect, useId, useLayoutEffect, useReducer, useRef, useState, type ChangeEvent, type CSSProperties, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { createPortal } from "react-dom";
import type { ParagraphLearningResult } from "../types/contracts";
import {
  buildSegmentedSourceParts,
  EMPTY_SEGMENT_INTERACTION_STATE,
  getActiveSegmentId,
  reduceSegmentInteraction,
} from "../lib/segmented-source";

export interface SegmentedSourceEditorProps {
  value: string;
  learning: ParagraphLearningResult | null;
  onChange: (value: string) => void;
  placeholder?: string;
}

interface TooltipPosition {
  top: number;
  left: number;
  maxWidth: number;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(Math.max(value, minimum), Math.max(minimum, maximum));
}

export default function SegmentedSourceEditor({
  value,
  learning,
  onChange,
  placeholder = "粘贴需要翻译的英文段落……",
}: SegmentedSourceEditorProps) {
  const [interaction, dispatch] = useReducer(
    reduceSegmentInteraction,
    EMPTY_SEGMENT_INTERACTION_STATE,
  );
  const [isEditing, setIsEditing] = useState(false);
  const [tooltipPosition, setTooltipPosition] = useState<TooltipPosition | null>(null);
  const surfaceRef = useRef<HTMLDivElement | null>(null);
  const editorRef = useRef<HTMLTextAreaElement | null>(null);
  const tooltipRef = useRef<HTMLDivElement | null>(null);
  const segmentRefs = useRef<Record<string, HTMLSpanElement | null>>({});
  const tooltipId = useId();
  const parts = buildSegmentedSourceParts(value, learning);
  const activeSegmentId = getActiveSegmentId(interaction);
  const activeSegment = learning?.segments.find((segment) => segment.id === activeSegmentId) ?? null;

  const clearSegmentInteraction = useCallback(() => {
    dispatch({ type: "reset" });
    setTooltipPosition(null);
  }, []);

  useEffect(() => {
    clearSegmentInteraction();
  }, [clearSegmentInteraction, learning, value]);

  useLayoutEffect(() => {
    if (!isEditing) return;
    editorRef.current?.focus();
  }, [isEditing]);

  const updateTooltipPosition = useCallback(() => {
    if (!activeSegmentId || !activeSegment) {
      setTooltipPosition(null);
      return;
    }
    const anchor = segmentRefs.current[activeSegmentId];
    if (!anchor) return;

    const anchorRect = anchor.getBoundingClientRect();
    const viewportPadding = 12;
    const preferredMaxWidth = Math.min(320, Math.max(180, window.innerWidth - viewportPadding * 2));
    const tooltip = tooltipRef.current;
    const tooltipWidth = tooltip?.offsetWidth || preferredMaxWidth;
    const tooltipHeight = tooltip?.offsetHeight || 76;
    const left = clamp(
      anchorRect.left,
      viewportPadding,
      window.innerWidth - tooltipWidth - viewportPadding,
    );
    let top = anchorRect.bottom + 8;
    if (top + tooltipHeight > window.innerHeight - viewportPadding) {
      top = anchorRect.top - tooltipHeight - 8;
    }
    if (top < viewportPadding) {
      top = Math.min(
        anchorRect.bottom + 8,
        Math.max(viewportPadding, window.innerHeight - tooltipHeight - viewportPadding),
      );
    }
    setTooltipPosition({ top, left, maxWidth: preferredMaxWidth });
  }, [activeSegment, activeSegmentId]);

  useLayoutEffect(() => {
    updateTooltipPosition();
  }, [updateTooltipPosition]);

  useEffect(() => {
    if (isEditing || !activeSegment) return undefined;
    const anchor = activeSegmentId ? segmentRefs.current[activeSegmentId] : null;
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(updateTooltipPosition);
    if (surfaceRef.current) observer?.observe(surfaceRef.current);
    if (anchor) observer?.observe(anchor);
    if (tooltipRef.current) observer?.observe(tooltipRef.current);
    window.addEventListener("resize", updateTooltipPosition);
    document.addEventListener("scroll", updateTooltipPosition, true);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", updateTooltipPosition);
      document.removeEventListener("scroll", updateTooltipPosition, true);
    };
  }, [activeSegment, activeSegmentId, isEditing, updateTooltipPosition]);

  const handleEditorChange = (event: ChangeEvent<HTMLTextAreaElement>) => {
    onChange(event.currentTarget.value.replace(/\r\n/g, "\n"));
  };

  const handleEnterEditing = () => {
    clearSegmentInteraction();
    setIsEditing(true);
  };

  const handleEditorBlur = () => {
    clearSegmentInteraction();
    setIsEditing(false);
  };

  const handleSegmentKeyDown = (event: ReactKeyboardEvent<HTMLSpanElement>) => {
    if (event.key !== "Escape") return;
    event.preventDefault();
    clearSegmentInteraction();
  };

  return (
    <>
      <div className="segmented-source-editor">
        {isEditing ? (
          <textarea
            ref={editorRef}
            className="segmented-source-editor-input"
            value={value}
            aria-label="原文"
            aria-multiline="true"
            placeholder={placeholder}
            spellCheck={false}
            onChange={handleEditorChange}
            onBlur={handleEditorBlur}
          />
        ) : (
          <div
            ref={surfaceRef}
            className="segmented-source-editor-surface"
            role="textbox"
            aria-label="原文"
            aria-multiline="true"
            aria-readonly="true"
            aria-placeholder={placeholder}
            data-empty={value.length === 0 ? "true" : "false"}
            tabIndex={0}
            onClick={handleEnterEditing}
          >
            {parts.map((part, index) => {
              if (part.type === "plain") return part.text ? <span key={`plain-${index}`}>{part.text}</span> : null;
              const { segment } = part;
              const isActive = segment.id === activeSegmentId;
              return (
                <span
                  key={segment.id}
                  ref={(element) => { segmentRefs.current[segment.id] = element; }}
                  className={`source-segment ${isActive ? "is-active" : ""}`}
                  data-segment-id={segment.id}
                  tabIndex={0}
                  aria-describedby={isActive ? tooltipId : undefined}
                  onMouseEnter={() => dispatch({ type: "pointerEnter", segmentId: segment.id })}
                  onMouseLeave={() => dispatch({ type: "pointerLeave", segmentId: segment.id })}
                  onFocus={() => dispatch({ type: "focus", segmentId: segment.id })}
                  onBlur={() => dispatch({ type: "blur", segmentId: segment.id })}
                  onKeyDown={handleSegmentKeyDown}
                >
                  {segment.source}
                </span>
              );
            })}
          </div>
        )}
      </div>
      {!isEditing && activeSegment && createPortal(
        <div
          ref={tooltipRef}
          id={tooltipId}
          className="segment-explanation-tooltip"
          role="tooltip"
          style={{
            top: tooltipPosition?.top ?? 0,
            left: tooltipPosition?.left ?? 0,
            maxWidth: tooltipPosition?.maxWidth ?? Math.min(320, Math.max(180, window.innerWidth - 24)),
            visibility: tooltipPosition ? "visible" : "hidden",
          } satisfies CSSProperties}
        >
          {activeSegment.explanation}
        </div>,
        document.body,
      )}
    </>
  );
}
