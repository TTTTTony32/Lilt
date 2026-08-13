import { useCallback, useEffect, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import { Copy, ExternalLink, LoaderCircle, Square, X } from "lucide-react";
import { getCurrentWindow, PhysicalSize } from "@tauri-apps/api/window";
import { describeError } from "./lib/errors";
import { isDictionarySelection } from "./lib/selection";
import { invokeCommand, listenTo } from "./lib/tauri";
import {
  decodeSelectionNotice,
  decodeSelectionRequest,
  decodeSelectionTriggerNotice,
  decodeSelectionUnavailable,
  decodeTranslationCommandResult,
  decodeTranslationEvent,
  decodeWordExampleCommandResult,
  decodeWordExampleEvent,
  type SelectionNotice,
  type SelectionRequestPayload,
  type SelectionTriggerNotice,
  type TranslationCommandResult,
  type TranslationEvent,
  type WordExampleEvent,
  type WordExampleState,
} from "./types/contracts";
import {
  decodeDictionaryLookupCommandResult,
  type DictionaryLookupCommandResult,
  type DictionaryPosGroup,
} from "./types/dictionary";

const TRANSLATION_EVENTS = [
  "translation_started",
  "translation_delta",
  "translation_completed",
  "translation_cancelled",
  "translation_failed",
] as const;

const WORD_EXAMPLE_EVENTS = [
  "word_example_started",
  "word_example_translation_delta",
  "word_example_pos_delta",
  "word_example_completed",
  "word_example_cancelled",
  "word_example_failed",
] as const;

const EMPTY_WORD_EXAMPLE: WordExampleState = {
  exampleId: null,
  requestId: null,
  translation: "",
  partOfSpeech: "",
  status: "idle",
  cacheHit: false,
  error: null,
};

type ViewStatus = "idle" | "loading" | "streaming" | "cancelling" | "completed" | "failed";
type TriggerStatus = "hidden" | "available" | "reading" | "failed";

export default function SelectionView() {
  const [triggerNotice, setTriggerNotice] = useState<SelectionTriggerNotice | null>(null);
  const [triggerStatus, setTriggerStatus] = useState<TriggerStatus>("hidden");
  const [triggerError, setTriggerError] = useState<string | null>(null);
  const [selection, setSelection] = useState<SelectionRequestPayload | null>(null);
  const [route, setRoute] = useState<"dictionary" | "paragraph" | null>(null);
  const [translation, setTranslation] = useState("");
  const [translationStatus, setTranslationStatus] = useState<ViewStatus>("idle");
  const [translationCacheHit, setTranslationCacheHit] = useState(false);
  const [dictionary, setDictionary] = useState<DictionaryLookupCommandResult | null>(null);
  const [dictionaryLoading, setDictionaryLoading] = useState(false);
  const [wordExample, setWordExample] = useState<WordExampleState>(EMPTY_WORD_EXAMPLE);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const activeTriggerId = useRef<string | null>(null);
  const activeSelectionId = useRef<string | null>(null);
  const activeTranslationId = useRef<string | null>(null);
  const activeWordExampleId = useRef<string | null>(null);
  const selectionScrollRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!triggerNotice || selection) return;
    const window = getCurrentWindow();
    const size = triggerStatus === "failed"
      ? new PhysicalSize(280, 80)
      : triggerStatus === "reading"
        ? new PhysicalSize(440, 320)
        : new PhysicalSize(48, 48);
    void window.setSize(size).catch(() => undefined);
  }, [triggerNotice, triggerStatus, selection]);

  useEffect(() => {
    if (!selection) return;
    const window = getCurrentWindow();
    void window.setSize(new PhysicalSize(440, 320)).catch(() => undefined);
  }, [selection]);

  const handleHeaderMouseDown = useCallback((event: ReactMouseEvent<HTMLElement>) => {
    if (event.button !== 0) return;
    if (event.target instanceof Element && event.target.closest("[data-no-drag], button, a, input, select, textarea")) return;
    event.preventDefault();
    void (async () => {
      try {
        await invokeCommand("begin_selection_drag");
        await getCurrentWindow().startDragging();
      } catch (reason) {
        setError(describeError(reason, "无法拖动划词浮窗"));
      } finally {
        await invokeCommand("end_selection_drag").catch(() => undefined);
      }
    })();
  }, []);

  const cancelCurrent = useCallback(() => {
    const translationId = activeTranslationId.current;
    const wordExampleId = activeWordExampleId.current;
    activeTranslationId.current = null;
    activeWordExampleId.current = null;
    if (translationId) void invokeCommand("cancel_translation", { requestId: translationId });
    if (wordExampleId) void invokeCommand("cancel_word_example", { requestId: wordExampleId });
  }, []);

  const applyTranslationResult = useCallback((requestId: string, result: TranslationCommandResult | null) => {
    if (!result || activeTranslationId.current !== requestId) return;
    activeTranslationId.current = null;
    if (result.outcome === "completed") {
      setTranslation(result.content ?? "");
      setTranslationCacheHit(result.cacheHit);
      setTranslationStatus("completed");
      setError(null);
    } else if (result.outcome === "cancelled") {
      setTranslationStatus("idle");
    } else {
      setTranslationStatus("failed");
      setError(result.message ?? "翻译请求失败");
    }
  }, []);

  const handleTranslationEvent = useCallback((event: TranslationEvent) => {
    if (event.requestId !== activeTranslationId.current) return;
    switch (event.type) {
      case "started":
        setTranslationStatus("streaming");
        break;
      case "delta":
        setTranslation((current) => current + event.content);
        break;
      case "completed":
        applyTranslationResult(event.requestId, {
          outcome: "completed",
          content: event.content,
          cacheHit: event.cacheHit,
          message: null,
        });
        break;
      case "cancelled":
        applyTranslationResult(event.requestId, {
          outcome: "cancelled",
          content: null,
          cacheHit: false,
          message: null,
        });
        break;
      case "failed":
        applyTranslationResult(event.requestId, {
          outcome: "failed",
          content: null,
          cacheHit: false,
          message: event.message,
        });
        break;
    }
  }, [applyTranslationResult]);

  const applyWordExampleResult = useCallback((requestId: string, raw: unknown) => {
    const result = decodeWordExampleCommandResult(raw);
    if (!result || activeWordExampleId.current !== requestId) return;
    activeWordExampleId.current = null;
    if (result.outcome === "completed") {
      setWordExample((current) => ({
        ...current,
        requestId: null,
        translation: result.translation ?? current.translation,
        partOfSpeech: result.partOfSpeech ?? current.partOfSpeech,
        status: "completed",
        cacheHit: result.cacheHit,
        error: null,
      }));
    } else if (result.outcome === "cancelled") {
      setWordExample((current) => ({ ...current, requestId: null, status: "idle" }));
    } else {
      setWordExample((current) => ({
        ...current,
        requestId: null,
        status: "failed",
        error: result.message ?? "例句生成失败",
      }));
    }
  }, []);

  const handleWordExampleEvent = useCallback((event: WordExampleEvent) => {
    if (event.requestId !== activeWordExampleId.current) return;
    switch (event.type) {
      case "started":
        setWordExample((current) => ({ ...current, status: "streaming", error: null }));
        break;
      case "translationDelta":
        setWordExample((current) => ({ ...current, translation: current.translation + event.content }));
        break;
      case "posDelta":
        setWordExample((current) => ({ ...current, partOfSpeech: current.partOfSpeech + event.content }));
        break;
      case "completed":
        applyWordExampleResult(event.requestId, {
          outcome: "completed",
          translation: event.translation,
          partOfSpeech: event.partOfSpeech,
          cacheHit: event.cacheHit,
          message: null,
        });
        break;
      case "cancelled":
        applyWordExampleResult(event.requestId, {
          outcome: "cancelled",
          translation: null,
          partOfSpeech: null,
          cacheHit: false,
          message: null,
        });
        break;
      case "failed":
        applyWordExampleResult(event.requestId, {
          outcome: "failed",
          translation: null,
          partOfSpeech: null,
          cacheHit: false,
          message: event.message,
        });
        break;
    }
  }, [applyWordExampleResult]);

  const startWordExample = useCallback(async (payload: SelectionRequestPayload, lookup: NonNullable<DictionaryLookupCommandResult["lookup"]>, exampleId: number) => {
    const requestId = crypto.randomUUID();
    activeWordExampleId.current = requestId;
    setWordExample({
      exampleId,
      requestId,
      translation: "",
      partOfSpeech: "",
      status: "streaming",
      cacheHit: false,
      error: null,
    });
    try {
      const raw = await invokeCommand<unknown>("generate_word_example", {
        request: {
          requestId,
          exampleId,
          word: lookup.word,
          canonicalWord: lookup.canonicalWord,
          targetLanguage: payload.targetLanguage,
        },
      });
      applyWordExampleResult(requestId, raw);
    } catch (reason) {
      if (activeWordExampleId.current !== requestId) return;
      activeWordExampleId.current = null;
      setWordExample((current) => ({ ...current, requestId: null, status: "failed", error: describeError(reason, "例句生成失败") }));
    }
  }, [applyWordExampleResult]);

  const lookupWord = useCallback(async (payload: SelectionRequestPayload, word: string, canonicalWord?: string) => {
    const previousWordExampleId = activeWordExampleId.current;
    activeWordExampleId.current = null;
    if (previousWordExampleId) {
      void invokeCommand("cancel_word_example", { requestId: previousWordExampleId });
    }
    setDictionaryLoading(true);
    setError(null);
    setNotice(null);
    try {
      const raw = await invokeCommand<unknown>("query_dictionary", { word, canonicalWord: canonicalWord ?? null });
      const result = decodeDictionaryLookupCommandResult(raw);
      if (!result) throw new Error("词典返回了无法识别的结果");
      if (activeSelectionId.current !== payload.requestId) return;
      setDictionary(result);
      setDictionaryLoading(false);
      setTranslationStatus("completed");
      if (result.lookup && result.example) {
        void startWordExample(payload, result.lookup, result.example.exampleId);
      } else {
        setWordExample(EMPTY_WORD_EXAMPLE);
      }
    } catch (reason) {
      if (activeSelectionId.current !== payload.requestId) return;
      setDictionaryLoading(false);
      setTranslationStatus("failed");
      setError(describeError(reason, "词典查询失败"));
    }
  }, [startWordExample]);

  const startTranslation = useCallback(async (payload: SelectionRequestPayload) => {
    const snapshot = await invokeCommand<{
      provider: { modelId: string; promptId: string };
    }>("get_app_snapshot");
    if (activeSelectionId.current !== payload.requestId) return;
    if (!snapshot.provider.modelId || !snapshot.provider.promptId) {
      setTranslationStatus("failed");
      setError("尚未配置 Provider");
      return;
    }
    const requestId = payload.requestId;
    activeTranslationId.current = requestId;
    setTranslationStatus("streaming");
    try {
      const raw = await invokeCommand<unknown>("translate", {
        request: {
          requestId,
          sourceText: payload.sourceText,
          sourceLanguage: payload.sourceLanguage,
          targetLanguage: payload.targetLanguage,
          modelId: snapshot.provider.modelId,
          promptId: snapshot.provider.promptId,
        },
      });
      applyTranslationResult(requestId, decodeTranslationCommandResult(raw));
    } catch (reason) {
      if (activeTranslationId.current !== requestId) return;
      activeTranslationId.current = null;
      setTranslationStatus("failed");
      setError(describeError(reason, "翻译请求失败"));
    }
  }, [applyTranslationResult]);

  const handleNotice = useCallback(async (notice: SelectionNotice) => {
    if (activeTriggerId.current !== notice.triggerId) return;
    cancelCurrent();
    if (activeTriggerId.current !== notice.triggerId) return;
    activeSelectionId.current = notice.requestId;
    activeTriggerId.current = null;
    setTriggerNotice(null);
    setTriggerStatus("hidden");
    setTriggerError(null);
    setSelection(null);
    setRoute(null);
    setDictionary(null);
    setTranslation("");
    setTranslationStatus("loading");
    setTranslationCacheHit(false);
    setWordExample(EMPTY_WORD_EXAMPLE);
    setError(null);
    setNotice(null);
    try {
      const raw = await invokeCommand<unknown>("get_selection_request", { requestId: notice.requestId });
      const payload = decodeSelectionRequest(raw);
      if (!payload || activeSelectionId.current !== notice.requestId) return;
      setSelection(payload);
      if (isDictionarySelection(payload.sourceText)) {
        setRoute("dictionary");
        await lookupWord(payload, payload.sourceText.trim());
      } else {
        setRoute("paragraph");
        await startTranslation(payload);
      }
    } catch (reason) {
      if (activeSelectionId.current !== notice.requestId) return;
      setTranslationStatus("failed");
      setError(describeError(reason, "无法读取选中文本"));
    }
  }, [cancelCurrent, lookupWord, startTranslation]);

  const handleTrigger = useCallback((notice: SelectionTriggerNotice) => {
    if (activeTriggerId.current === notice.triggerId) return;
    cancelCurrent();
    activeTriggerId.current = notice.triggerId;
    activeSelectionId.current = null;
    setTriggerNotice(notice);
    setTriggerStatus("available");
    setTriggerError(null);
    setSelection(null);
    setRoute(null);
    setDictionary(null);
    setTranslation("");
    setTranslationStatus("idle");
    setTranslationCacheHit(false);
    setWordExample(EMPTY_WORD_EXAMPLE);
    setError(null);
    setNotice(null);
  }, [cancelCurrent]);

  const activateTrigger = useCallback(async () => {
    const notice = triggerNotice;
    if (!notice || triggerStatus !== "available") return;
    activeTriggerId.current = notice.triggerId;
    setTriggerStatus("reading");
    setTriggerError(null);
    try {
      await invokeCommand("activate_selection", { triggerId: notice.triggerId });
    } catch (reason) {
      if (activeTriggerId.current !== notice.triggerId) return;
      setTriggerStatus("failed");
      setTriggerError(describeError(reason, "无法读取选中文本"));
    }
  }, [triggerNotice, triggerStatus]);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const setup = async () => {
      const eventNames = ["selection_trigger_available", "selection_available", "selection_unavailable", ...TRANSLATION_EVENTS, ...WORD_EXAMPLE_EVENTS];
      const results = await Promise.allSettled(eventNames.map(async (name) => listenTo<unknown>(name, (payload) => {
        if (disposed) return;
        if (name === "selection_trigger_available") {
          const notice = decodeSelectionTriggerNotice(payload);
          if (notice) handleTrigger(notice);
        } else if (name === "selection_available") {
          const notice = decodeSelectionNotice(payload);
          if (notice) void handleNotice(notice);
        } else if (name === "selection_unavailable") {
          const value = decodeSelectionUnavailable(payload);
          if (value) {
            if (activeTriggerId.current) {
              if (value.triggerId !== activeTriggerId.current) return;
              setTriggerStatus("failed");
              setTriggerError(value.message);
            } else if (!activeSelectionId.current) {
              setTranslationStatus("failed");
              setError(value.message);
            }
          }
        } else if (name.startsWith("translation_")) {
          const event = decodeTranslationEvent(name, payload);
          if (event) handleTranslationEvent(event);
        } else {
          const event = decodeWordExampleEvent(name, payload);
          if (event) handleWordExampleEvent(event);
        }
      })));
      for (const result of results) {
        if (result.status === "fulfilled") {
          if (disposed) result.value();
          else unlisteners.push(result.value);
        }
      }
      if (disposed) return;
      try {
        const pendingRaw = await invokeCommand<unknown>("selection_window_ready");
        const pending = pendingRaw === null ? null : decodeSelectionTriggerNotice(pendingRaw);
        if (pending) handleTrigger(pending);
      } catch (reason) {
        if (!disposed) setError(describeError(reason, "划词浮窗初始化失败"));
      }
    };
    void setup();
    return () => {
      disposed = true;
      unlisteners.splice(0).forEach((unlisten) => unlisten());
      void cancelCurrent();
    };
  }, [cancelCurrent, handleNotice, handleTrigger, handleTranslationEvent, handleWordExampleEvent]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        const requestId = activeSelectionId.current;
        activeTriggerId.current = null;
        activeSelectionId.current = null;
        cancelCurrent();
        setTriggerNotice(null);
        setTriggerStatus("hidden");
        void invokeCommand("dismiss_selection", { requestId });
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [cancelCurrent]);

  useEffect(() => {
    if (selectionScrollRef.current) selectionScrollRef.current.scrollTop = 0;
  }, [selection?.requestId]);

  const cancel = async () => {
    const requestId = activeSelectionId.current;
    setTranslationStatus("cancelling");
    activeTriggerId.current = null;
    activeSelectionId.current = null;
    setTriggerNotice(null);
    setTriggerStatus("hidden");
    cancelCurrent();
    await invokeCommand("dismiss_selection", { requestId });
  };

  const copy = async () => {
    const value = route === "paragraph" ? translation : wordExample.translation;
    if (!value) return;
    try {
      await navigator.clipboard.writeText(value);
      setNotice("已复制");
    } catch (reason) {
      setError(describeError(reason, "复制失败"));
    }
  };

  const openMain = async () => {
    if (!activeSelectionId.current) return;
    try {
      await invokeCommand("open_selection_in_main", { requestId: activeSelectionId.current });
    } catch (reason) {
      setError(describeError(reason, "无法打开主窗口"));
    }
  };

  const closeSelection = () => {
    const requestId = activeSelectionId.current;
    activeTriggerId.current = null;
    activeSelectionId.current = null;
    cancelCurrent();
    setTriggerNotice(null);
    setTriggerStatus("hidden");
    void invokeCommand("dismiss_selection", { requestId });
  };

  const dictionaryGroup = dictionary?.lookup?.entry.pos_groups[0];
  const isBusy = dictionaryLoading || translationStatus === "loading" || translationStatus === "streaming" || translationStatus === "cancelling" || wordExample.status === "streaming";
  const resultText = route === "paragraph" ? translation : wordExample.translation;

  if (triggerNotice && !selection && triggerStatus !== "hidden" && triggerStatus !== "reading") {
    return (
      <main className="selection-trigger-window" role="dialog" aria-label="Lilt 划词操作">
        <button
          className="selection-trigger-button"
          type="button"
          aria-label="读取选中文本"
          disabled={triggerStatus !== "available"}
          onClick={() => void activateTrigger()}
        />
        {triggerStatus === "failed" && triggerError && <span className="selection-trigger-error">{triggerError}</span>}
      </main>
    );
  }

  return (
    <main className="selection-window" role="dialog" aria-label="Lilt 划词翻译">
      <header className="selection-header" onMouseDown={handleHeaderMouseDown}>
        <div><span className="selection-mark">L</span><strong>Lilt</strong></div>
        <button className="selection-close" type="button" data-no-drag onClick={closeSelection} aria-label="关闭"><X size={16} /></button>
      </header>
      <div className="selection-scroll" ref={selectionScrollRef}>
        {selection ? <p className="selection-source">{selection.sourceText}</p> : <p className="selection-placeholder">{triggerStatus === "reading" ? "正在读取选中文本……" : "读取选区中……"}</p>}
        {route === "dictionary" && (
          <section className="selection-result">
            {dictionaryLoading && <div className="selection-loading"><LoaderCircle className="spin" size={16} />正在查询词典</div>}
            {dictionary?.candidates.length ? <div className="selection-candidates"><span>请选择规范词头</span>{dictionary.candidates.map((candidate) => <button key={candidate.normalizedCanonicalWord} type="button" onClick={() => selection && void lookupWord(selection, selection.sourceText, candidate.canonicalWord)}>{candidate.canonicalWord}</button>)}</div> : null}
            {dictionary?.lookup && <DictionarySummary group={dictionaryGroup} summary={dictionary.lookup.entry.headword_summary} canonical={dictionary.lookup.canonicalWord} />}
            {dictionary && !dictionary.lookup && dictionary.candidates.length === 0 && !dictionaryLoading && <p className="selection-muted">词典中未找到这个词。</p>}
            {dictionary?.example && <div className="selection-example"><span>例句</span><p>{dictionary.example.sourceText}</p>{wordExample.status === "failed" && <small>{wordExample.error}</small>}{wordExample.translation && <p className="selection-example-translation">{wordExample.translation}</p>}{wordExample.partOfSpeech && <span className="selection-pos">{wordExample.partOfSpeech}</span>}</div>}
          </section>
        )}
        {route === "paragraph" && <section className="selection-result"><div className="selection-label">译文{translationCacheHit && <span>缓存命中</span>}</div><div className="selection-translation">{translation || (translationStatus === "loading" ? "准备翻译……" : "")}{translationStatus === "streaming" && <span className="stream-caret" />}</div>{translationStatus === "failed" && <p className="selection-muted">翻译失败</p>}</section>}
        {error && <p className="selection-error">{error}</p>}
        {notice && <p className="selection-notice">{notice}</p>}
      </div>
      <footer className="selection-actions">
        <span>{route === "dictionary" ? "词典" : route === "paragraph" ? "段落翻译" : ""}</span>
        <div>
          {isBusy && <button className="selection-action danger" type="button" onClick={() => void cancel()}><Square size={13} fill="currentColor" />取消</button>}
          <button className="selection-action" type="button" onClick={() => void copy()} disabled={!resultText}><Copy size={14} />复制</button>
          <button className="selection-action" type="button" onClick={() => void openMain()}><ExternalLink size={14} />打开主窗口</button>
        </div>
      </footer>
    </main>
  );
}

function DictionarySummary({ group, summary, canonical }: { group: DictionaryPosGroup | undefined; summary: string; canonical: string }) {
  const meaning = group?.meanings[0];
  return <div className="selection-dictionary"><div className="selection-word-heading"><strong>{canonical}</strong>{group?.pos && <span>{group.pos}</span>}</div><p>{summary}</p>{group?.summary && <p className="selection-muted">{group.summary}</p>}{meaning && <p>{meaning.short_gloss || meaning.learner_explanation}</p>}</div>;
}
