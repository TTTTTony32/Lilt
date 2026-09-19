// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DownloadActivityStack } from "./DownloadActivityStack";

const reactGlobal = globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean };
reactGlobal.IS_REACT_ACT_ENVIRONMENT = true;

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
});

describe("DownloadActivityStack", () => {
  it("renders PDF translation and resource activities as clickable floating cards", () => {
    const onPdfTranslationClick = vi.fn();
    const onResourceClick = vi.fn();

    act(() => {
      root.render(createElement(DownloadActivityStack, {
        pdfTranslation: {
          status: "running",
          statusText: "PDF 全文翻译 · 分段翻译中 · 1/3",
          progress: 35,
        },
        onPdfTranslationClick,
        onResourceClick,
        activities: [
          {
            key: "dictionary:dictionary-1",
            resource: "dictionary",
            operationId: "dictionary-1",
            status: "running",
            phase: "download",
            stagePercent: 20,
            overallPercent: 14,
            message: null,
            error: null,
          },
          {
            key: "pdf-engine:pdf-engine-1",
            resource: "pdf-engine",
            operationId: "pdf-engine-1",
            status: "failed",
            phase: "verify",
            stagePercent: 70,
            overallPercent: 86,
            message: null,
            error: "引擎准备失败",
          },
        ],
      }));
    });

    const cards = container.querySelectorAll<HTMLButtonElement>("button.progress-floating-card");
    expect(cards).toHaveLength(3);
    const pdfCard = cards[0];
    const dictionaryCard = cards[1];
    const pdfEngineCard = cards[2];
    if (!pdfCard || !dictionaryCard || !pdfEngineCard) throw new Error("悬浮卡片未完整渲染");
    expect(pdfCard.getAttribute("aria-label")).toBe("打开 PDF 翻译任务详情");
    expect(dictionaryCard.getAttribute("aria-label")).toBe("打开词典下载设置");
    expect(pdfEngineCard.getAttribute("aria-label")).toBe("打开 PDF Engine 准备设置");
    expect(pdfCard.querySelector(".progress-floating-progress span")?.getAttribute("style")).toContain("35%");
    expect(pdfEngineCard.textContent).toContain("引擎准备失败");

    act(() => pdfCard.click());
    act(() => dictionaryCard.click());
    act(() => pdfEngineCard.click());

    expect(onPdfTranslationClick).toHaveBeenCalledOnce();
    expect(onResourceClick).toHaveBeenNthCalledWith(1, "dictionary");
    expect(onResourceClick).toHaveBeenNthCalledWith(2, "pdf-engine");
  });
});
