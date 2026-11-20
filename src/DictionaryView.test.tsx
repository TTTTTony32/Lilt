// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import DictionaryView from "./DictionaryView";
import { DEFAULT_DICTIONARY_STATE } from "./types/dictionary";

const { invokeCommand } = vi.hoisted(() => ({ invokeCommand: vi.fn() }));
vi.mock("./lib/tauri", () => ({ invokeCommand }));
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

it("waits for the dictionary before enabling search and opening a saved word", async () => {
  invokeCommand.mockResolvedValue({ lookup: null, candidates: [], example: null, history: [], invalidWord: false, invalidInsight: null });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const onOpenRequestHandled = vi.fn();
  const props = {
    state: { ...DEFAULT_DICTIONARY_STATE, status: "preparing" as const, installedRelease: "v1" },
    history: [], progress: null, snapshotReady: true, targetLanguage: "zh",
    wordExample: { exampleId: null, requestId: null, translation: "", partOfSpeech: "", status: "idle" as const, error: null, cacheHit: false },
    onUpdate: vi.fn(async () => {}), onResourceDownloadPrompt: vi.fn(), onHistoryChanged: vi.fn(),
    onSnapshotChanged: vi.fn(async () => {}), onWordExampleRequested: vi.fn(), onWordExampleCancelled: vi.fn(),
    personalDictionary: [], openRequest: { requestId: "1", lookupWord: "hello", canonicalWord: "hello" },
    onOpenRequestHandled, onPersonalDictionaryChanged: vi.fn(async () => {}),
    onOpenPersonalDictionary: vi.fn(), onOpenDictionaryAbout: vi.fn(),
  };
  await act(async () => { root.render(<DictionaryView {...props} />); });
  expect(container.querySelector<HTMLInputElement>('input[aria-label="词典查询"]')?.disabled).toBe(true);
  expect(container.querySelector<HTMLButtonElement>('button[type="submit"]')?.disabled).toBe(true);
  expect(container.querySelector('.dictionary-status-dot')?.getAttribute('title')).toBe("词典正在准备");
  expect(container.textContent).toContain("词典正在准备");
  expect(container.textContent).not.toContain("正在请求 AI");
  expect(invokeCommand).not.toHaveBeenCalled();

  await act(async () => { root.render(<DictionaryView {...props} state={{ ...props.state, status: "ready" }} />); });
  expect(container.querySelector<HTMLInputElement>('input[aria-label="词典查询"]')?.disabled).toBe(false);
  expect(container.querySelector('.dictionary-status-dot')?.getAttribute('title')).toBe("词典准备完成");
  expect(invokeCommand).toHaveBeenCalledWith("query_dictionary", { word: "hello", canonicalWord: "hello" });
  expect(onOpenRequestHandled).toHaveBeenCalledOnce();
  await act(async () => { root.unmount(); });
  container.remove();
});
