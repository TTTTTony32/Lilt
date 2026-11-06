// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import PdfView from "./PdfView";
import type { PdfEngineRuntime } from "./lib/usePdfEngineRuntime";

const { invokeCommand } = vi.hoisted(() => ({ invokeCommand: vi.fn() }));
vi.mock("./lib/tauri", () => ({ invokeCommand, listenTo: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }) }));
vi.mock("./components/usePrefersReducedMotion", () => ({ usePrefersReducedMotion: () => true }));

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

describe("PDF history actions", () => {
  afterEach(() => {
    window.localStorage.clear();
    document.body.replaceChildren();
    invokeCommand.mockReset();
  });

  it("confirms deletion, remembers only a successful choice, and allows restoring the prompt", async () => {
    const entries = ["first", "second"].map((id) => ({
      taskId: id, sourceFileName: `${id}.pdf`, outputFileName: `${id}.pdf`,
      outputPdf: `C:/${id}.pdf`, completedAtUnixMs: 1, outputMode: "bilingual", pageCount: 1, legacy: false,
    }));
    let failFirstDeletion = true;
    invokeCommand.mockImplementation(async (command: string, args?: { taskId?: string }) => {
      if (command === "list_pdf_translation_history") return [...entries];
      if (command === "delete_pdf_translation_history") {
        if (failFirstDeletion) {
          failFirstDeletion = false;
          throw new Error("文件正被占用");
        }
        const index = entries.findIndex((item) => item.taskId === args?.taskId);
        if (index >= 0) entries.splice(index, 1);
      }
      return null;
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const engine: PdfEngineRuntime = { status: null, statusLoading: true, preparing: false, progress: null, error: null, eventsError: null, prepare: async () => {} };
    await act(async () => {
      root.render(<PdfView active visible pdfEngine={engine} pdfPreflightEnabled={false} pdfPreflightPageLimit={1} pdfNetworkConcurrency={2} pdfPreflightSaving={false} onPdfPreflightEnabledChange={() => {}} onResourceDownloadPrompt={() => {}} onOpenPdfEngineSettings={() => {}} onPdfTranslationFloatingChange={() => {}} />);
    });

    const card = container.querySelector(".pdf-history-item") as HTMLElement;
    expect(card.tagName).toBe("DIV");
    await act(async () => { card.click(); });
    expect(invokeCommand).not.toHaveBeenCalledWith("reveal_pdf_file", expect.anything());
    const folder = card.querySelector<HTMLButtonElement>('button[title="打开文件夹"]')!;
    await act(async () => { folder.click(); });
    expect(invokeCommand).toHaveBeenCalledWith("reveal_pdf_file", { filePath: "C:/first.pdf" });

    await act(async () => { card.querySelector<HTMLButtonElement>('button[title="删除任务"]')!.click(); });
    expect(document.querySelector('[role="dialog"]')?.textContent).toContain("本地译文和任务文件将一并删除");
    const checkbox = document.querySelector<HTMLInputElement>('.pdf-delete-remember input')!;
    await act(async () => { checkbox.click(); });
    const cancel = [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find((button) => button.textContent === "取消")!;
    await act(async () => { cancel.click(); });
    expect(invokeCommand).not.toHaveBeenCalledWith("delete_pdf_translation_history", expect.anything());
    expect(window.localStorage.length).toBe(0);

    await act(async () => { card.querySelector<HTMLButtonElement>('button[title="删除任务"]')!.click(); });
    await act(async () => { document.querySelector<HTMLInputElement>('.pdf-delete-remember input')!.click(); });
    await act(async () => { document.querySelector<HTMLButtonElement>('.pdf-delete-confirm')!.click(); });
    expect(document.querySelector('[role="dialog"]')?.textContent).toContain("文件正被占用");
    expect(window.localStorage.length).toBe(0);
    await act(async () => { document.querySelector<HTMLButtonElement>('.pdf-delete-confirm')!.click(); });
    expect(invokeCommand).toHaveBeenCalledWith("delete_pdf_translation_history", { taskId: "first" });
    expect(window.localStorage.length).toBe(1);
    expect(container.textContent).toContain("恢复删除确认");

    await act(async () => { container.querySelector<HTMLButtonElement>('button[title="删除任务"]')!.click(); });
    expect(invokeCommand).toHaveBeenCalledWith("delete_pdf_translation_history", { taskId: "second" });
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    await act(async () => { container.querySelector<HTMLButtonElement>('.pdf-history-restore-confirm')!.click(); });
    expect(window.localStorage.length).toBe(0);
    await act(async () => { root.unmount(); });
  });
});
