// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import SegmentedSourceEditor from "./SegmentedSourceEditor";
import {
  buildSegmentedSourceParts,
  EMPTY_SEGMENT_INTERACTION_STATE,
  getActiveSegmentId,
  reduceSegmentInteraction,
} from "../lib/segmented-source";
import { PARAGRAPH_LEARNING_PROTOCOL_VERSION, type ParagraphLearningResult } from "../types/contracts";

const reactGlobal = globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean };
reactGlobal.IS_REACT_ACT_ENVIRONMENT = true;

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

const source = "  前😀后文  ";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  document.querySelectorAll(".segment-explanation-tooltip").forEach((tooltip) => tooltip.remove());
});

function renderEditor(
  value = source,
  learningResult: ParagraphLearningResult | null = learning,
  onChange = vi.fn(),
) {
  act(() => {
    root.render(createElement(SegmentedSourceEditor, {
      value,
      learning: learningResult,
      onChange,
    }));
  });
  return onChange;
}

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

describe("segmented source editor component", () => {
  it("renders one read-only segmented source and avoids contentEditable duplication", () => {
    renderEditor();
    const surface = container.querySelector<HTMLElement>(".segmented-source-editor-surface");
    expect(surface).not.toBeNull();
    expect(surface?.textContent).toBe(source);
    expect(surface?.getAttribute("contenteditable")).toBeNull();
    expect(surface?.getAttribute("aria-readonly")).toBe("true");
    expect(surface?.querySelectorAll("[data-segment-id]")).toHaveLength(2);

    act(() => {
      root.render(createElement(SegmentedSourceEditor, {
        value: source,
        learning: { ...learning, segments: [...learning.segments] },
        onChange: vi.fn(),
      }));
    });

    const rerenderedSurface = container.querySelector<HTMLElement>(".segmented-source-editor-surface");
    expect(rerenderedSurface?.textContent).toBe(source);
    expect(rerenderedSurface?.querySelectorAll("[data-segment-id]")).toHaveLength(2);
  });

  it("enters a focused controlled textarea from the source area and reports input", () => {
    const onChange = renderEditor();
    const surface = container.querySelector<HTMLElement>(".segmented-source-editor-surface");
    expect(surface).not.toBeNull();

    act(() => {
      surface?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    const editor = container.querySelector<HTMLTextAreaElement>(".segmented-source-editor-input");
    expect(editor).not.toBeNull();
    expect(editor?.value).toBe(source);
    expect(document.activeElement).toBe(editor);
    expect(container.querySelectorAll("[data-segment-id]")).toHaveLength(0);

    act(() => {
      if (!editor) return;
      const setNativeValue = Object.getOwnPropertyDescriptor(
        HTMLTextAreaElement.prototype,
        "value",
      )?.set;
      setNativeValue?.call(editor, "改写后的\n段落");
      editor.dispatchEvent(new Event("input", { bubbles: true }));
    });

    expect(onChange).toHaveBeenCalledWith("改写后的\n段落");
  });

  it("clears hover and tooltip when a segment enters editing, then restores current segments on blur", () => {
    renderEditor();
    const segment = container.querySelector<HTMLElement>("[data-segment-id='segment-1']");
    expect(segment).not.toBeNull();

    act(() => {
      segment?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    });
    expect(segment?.classList.contains("is-active")).toBe(true);
    expect(document.querySelector(".segment-explanation-tooltip")?.textContent).toBe("第一个解释");

    act(() => {
      segment?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(container.querySelector(".segmented-source-editor-input")).not.toBeNull();
    expect(container.querySelectorAll("[data-segment-id]")).toHaveLength(0);
    expect(document.querySelector(".segment-explanation-tooltip")).toBeNull();

    act(() => {
      (document.activeElement as HTMLTextAreaElement | null)?.blur();
    });
    expect(container.querySelector(".segmented-source-editor-surface")).not.toBeNull();
    expect(container.querySelectorAll("[data-segment-id]")).toHaveLength(2);
  });

  it("clears focused segment highlighting and its explanation on Escape", () => {
    renderEditor();
    const segment = container.querySelector<HTMLElement>("[data-segment-id='segment-2']");
    expect(segment).not.toBeNull();

    act(() => {
      segment?.focus();
    });
    expect(segment?.classList.contains("is-active")).toBe(true);
    expect(document.querySelector(".segment-explanation-tooltip")?.textContent).toBe("第二个解释");

    act(() => {
      segment?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    expect(segment?.classList.contains("is-active")).toBe(false);
    expect(document.querySelector(".segment-explanation-tooltip")).toBeNull();
  });
});
