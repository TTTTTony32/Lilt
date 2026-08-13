import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Copy, FileText, History, Languages, LoaderCircle, Settings, Square, WandSparkles, BookOpen } from "lucide-react";
import { describeError } from "./lib/errors";
import { invokeCommand, listenTo } from "./lib/tauri";
import DictionaryView, { type DictionaryProgress, type WordExampleRequestInput } from "./DictionaryView";
import {
  DEFAULT_SNAPSHOT,
  type AppSettings,
  type AppSnapshot,
  type AppTab,
  type GlossaryTerm,
  type HistoryEntry,
  type ModelInfo,
  type TranslationCommandResult,
  type TranslationEvent,
  type TranslationStatus,
  type WordExampleCommandResult,
  type WordExampleEvent,
  type WordExampleState,
  type SelectionRuntimeStatus,
  decodeSelectionRequest,
  decodeSelectionStatus,
  decodeTranslationCommandResult,
  decodeTranslationEvent,
  decodeWordExampleCommandResult,
  decodeWordExampleEvent,
} from "./types/contracts";
import {
  decodeDictionaryCommandResult,
  decodeDictionaryUpdateEvent,
  type DictionaryHistoryEntry,
  DICTIONARY_EVENT_NAMES,
  type DictionaryUpdateEvent,
} from "./types/dictionary";

const EVENT_NAMES = [
  "translation_started",
  "translation_delta",
  "translation_completed",
  "translation_cancelled",
  "translation_failed",
] as const;

const WORD_EXAMPLE_EVENT_NAMES = [
  "word_example_started",
  "word_example_translation_delta",
  "word_example_pos_delta",
  "word_example_completed",
  "word_example_cancelled",
  "word_example_failed",
] as const;

const DEFAULT_WORD_EXAMPLE_STATE: WordExampleState = {
  exampleId: null,
  requestId: null,
  translation: "",
  partOfSpeech: "",
  status: "idle",
  cacheHit: false,
  error: null,
};

const LANGUAGE_OPTIONS = [
  ["英语", "en"],
  ["简体中文", "zh-CN"],
  ["繁体中文", "zh-TW"],
  ["日语", "ja"],
  ["韩语", "ko"],
] as const;

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function formatDate(value: string): string {
  return new Intl.DateTimeFormat("zh-CN", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
}

function App() {
  const [snapshot, setSnapshot] = useState<AppSnapshot>(DEFAULT_SNAPSHOT);
  const [tab, setTab] = useState<AppTab>("translate");
  const [sourceText, setSourceText] = useState("");
  const [translatedText, setTranslatedText] = useState("");
  const [sourceLanguage, setSourceLanguage] = useState("en");
  const [targetLanguage, setTargetLanguage] = useState("zh-CN");
  const [status, setStatus] = useState<TranslationStatus>("idle");
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [lastCacheHit, setLastCacheHit] = useState(false);
  const [translationEventsReady, setTranslationEventsReady] = useState(false);
  const [translationEventsError, setTranslationEventsError] = useState<string | null>(null);
  const [dictionaryProgress, setDictionaryProgress] = useState<DictionaryProgress | null>(null);
  const [dictionaryEventsError, setDictionaryEventsError] = useState<string | null>(null);
  const [wordExample, setWordExample] = useState<WordExampleState>(DEFAULT_WORD_EXAMPLE_STATE);
  const activeRequestId = useRef<string | null>(null);
  const activeDictionaryOperationId = useRef<string | null>(null);
  const activeWordExampleRequestId = useRef<string | null>(null);

  const refreshSnapshot = useCallback(async () => {
    try {
      const next = await invokeCommand<AppSnapshot>("get_app_snapshot");
      setSnapshot(next);
    } catch (reason) {
      setError(describeError(reason, "无法读取应用配置"));
    }
  }, []);

  const handleDictionaryHistoryChanged = useCallback((history: DictionaryHistoryEntry[]) => {
    setSnapshot((current) => ({ ...current, dictionaryHistory: history }));
  }, []);

  useEffect(() => {
    void refreshSnapshot();
  }, [refreshSnapshot]);

  useEffect(() => {
    void invokeCommand("set_selection_language", {
      sourceLanguage,
      targetLanguage,
    }).catch(() => {
      // 主窗口启动时后端可能尚未完成初始化，划词功能会在下一次配置或选区事件中恢复。
    });
  }, [sourceLanguage, targetLanguage]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    const initialise = async () => {
      try {
        const next = await listenTo<unknown>("selection_open_main", async (payload) => {
          if (disposed || typeof payload !== "string") return;
          try {
            const raw = await invokeCommand<unknown>("get_selection_request", { requestId: payload });
            const request = decodeSelectionRequest(raw);
            if (!request || disposed) return;
            setSourceText(request.sourceText);
            setSourceLanguage(request.sourceLanguage);
            setTargetLanguage(request.targetLanguage);
            setTab("translate");
            setError(null);
            setNotice("已将选中文本载入段落翻译");
          } catch (reason) {
            if (!disposed) setError(describeError(reason, "无法读取选中文本"));
          }
        });
        if (disposed) next();
        else unlisten = next;
      } catch (reason) {
        if (!disposed) setError(describeError(reason, "划词翻译事件监听初始化失败"));
      }
    };
    void initialise();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const applyTranslationResult = useCallback((requestId: string, result: TranslationCommandResult) => {
    if (requestId !== activeRequestId.current) return;
    activeRequestId.current = null;
    switch (result.outcome) {
      case "completed":
        setTranslatedText(result.content ?? "");
        setLastCacheHit(result.cacheHit);
        setError(null);
        setStatus("completed");
        void refreshSnapshot();
        break;
      case "cancelled":
        setError(null);
        setStatus("idle");
        break;
      case "failed":
        setError(result.message ?? "翻译请求失败");
        setStatus("failed");
        break;
    }
  }, [refreshSnapshot]);

  const applyWordExampleResult = useCallback((requestId: string, result: WordExampleCommandResult) => {
    if (requestId !== activeWordExampleRequestId.current) return;
    activeWordExampleRequestId.current = null;
    switch (result.outcome) {
      case "completed":
        setWordExample((current) => ({
          ...current,
          requestId: null,
          translation: result.translation ?? current.translation,
          partOfSpeech: result.partOfSpeech ?? current.partOfSpeech,
          status: "completed",
          cacheHit: result.cacheHit,
          error: null,
        }));
        break;
      case "cancelled":
        setWordExample((current) => ({ ...current, requestId: null, status: "idle" }));
        break;
      case "failed":
        setWordExample((current) => ({
          ...current,
          requestId: null,
          status: "failed",
          error: result.message ?? "单词例句生成失败",
        }));
        break;
    }
  }, []);

  const handleWordExampleEvent = useCallback((event: WordExampleEvent) => {
    if (event.requestId !== activeWordExampleRequestId.current) return;
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

  const handleEvent = useCallback((event: TranslationEvent) => {
    if (event.requestId !== activeRequestId.current) return;
    switch (event.type) {
      case "started":
        setStatus("streaming");
        break;
      case "delta":
        setTranslatedText((current) => current + event.content);
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

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    setTranslationEventsReady(false);
    setTranslationEventsError(null);

    const initialiseListeners = async () => {
      const results = await Promise.allSettled(EVENT_NAMES.map(async (name) => {
        return listenTo<unknown>(name, (payload) => {
          if (disposed) return;
          const event = decodeTranslationEvent(name, payload);
          if (event) handleEvent(event);
        });
      }));

      for (const result of results) {
        if (result.status !== "fulfilled") continue;
        if (disposed) result.value();
        else unlisteners.push(result.value);
      }
      if (disposed) return;

      const failure = results.find((result) => result.status === "rejected");
      if (failure?.status === "rejected") {
        unlisteners.splice(0).forEach((unlisten) => unlisten());
        setTranslationEventsError(describeError(failure.reason, "翻译事件监听初始化失败，请重试。"));
        return;
      }
      setTranslationEventsReady(true);
    };

    void initialiseListeners();
    return () => {
      disposed = true;
      unlisteners.splice(0).forEach((unlisten) => unlisten());
    };
  }, [handleEvent]);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const initialiseWordExampleListeners = async () => {
      const results = await Promise.allSettled(WORD_EXAMPLE_EVENT_NAMES.map(async (name) => {
        return listenTo<unknown>(name, (payload) => {
          if (disposed) return;
          const event = decodeWordExampleEvent(name, payload);
          if (event) handleWordExampleEvent(event);
        });
      }));
      for (const result of results) {
        if (result.status !== "fulfilled") continue;
        if (disposed) result.value();
        else unlisteners.push(result.value);
      }
      if (disposed) return;
      const failure = results.find((result) => result.status === "rejected");
      if (failure?.status === "rejected") {
        unlisteners.splice(0).forEach((unlisten) => unlisten());
        setWordExample((current) => ({
          ...current,
          status: "failed",
          error: describeError(failure.reason, "单词例句事件监听初始化失败"),
        }));
      }
    };
    void initialiseWordExampleListeners();
    return () => {
      disposed = true;
      unlisteners.splice(0).forEach((unlisten) => unlisten());
    };
  }, [handleWordExampleEvent]);

  const handleDictionaryEvent = useCallback((event: DictionaryUpdateEvent) => {
    if (event.type === "started") {
      activeDictionaryOperationId.current = event.operationId;
      setDictionaryProgress(null);
      setSnapshot((current) => ({ ...current, dictionary: event.state }));
      return;
    }
    if (event.operationId !== activeDictionaryOperationId.current) return;

    switch (event.type) {
      case "downloadProgress":
        setDictionaryProgress({
          operationId: event.operationId,
          phase: "download",
          current: event.downloadedBytes,
          total: event.totalBytes,
        });
        setSnapshot((current) => ({
          ...current,
          dictionary: {
            ...current.dictionary,
            status: "updating",
            downloadedBytes: event.downloadedBytes,
            totalBytes: event.totalBytes,
            error: null,
          },
        }));
        break;
      case "verifyProgress":
        setDictionaryProgress({
          operationId: event.operationId,
          phase: "verify",
          current: event.current,
          total: event.total,
        });
        break;
      case "extractProgress":
        setDictionaryProgress({
          operationId: event.operationId,
          phase: "extract",
          current: event.current,
          total: event.total,
        });
        break;
      case "completed":
        activeDictionaryOperationId.current = null;
        setDictionaryProgress(null);
        setSnapshot((current) => ({ ...current, dictionary: event.state }));
        void refreshSnapshot();
        break;
      case "failed":
        activeDictionaryOperationId.current = null;
        setDictionaryProgress(null);
        setSnapshot((current) => ({
          ...current,
          dictionary: { ...current.dictionary, status: "failed", error: event.message },
        }));
        break;
    }
  }, [refreshSnapshot]);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    setDictionaryEventsError(null);

    const initialiseDictionaryListeners = async () => {
      const results = await Promise.allSettled(DICTIONARY_EVENT_NAMES.map(async (name) => {
        return listenTo<unknown>(name, (payload) => {
          if (disposed) return;
          const event = decodeDictionaryUpdateEvent(name, payload);
          if (event) handleDictionaryEvent(event);
        });
      }));
      for (const result of results) {
        if (result.status !== "fulfilled") continue;
        if (disposed) result.value();
        else unlisteners.push(result.value);
      }
      if (disposed) return;
      const failure = results.find((result) => result.status === "rejected");
      if (failure?.status === "rejected") {
        unlisteners.splice(0).forEach((unlisten) => unlisten());
        setDictionaryEventsError(describeError(failure.reason, "词典更新事件监听初始化失败。"));
      }
    };

    void initialiseDictionaryListeners();
    return () => {
      disposed = true;
      unlisteners.splice(0).forEach((unlisten) => unlisten());
    };
  }, [handleDictionaryEvent]);

  const handleDictionaryUpdate = useCallback(async () => {
    setDictionaryEventsError(null);
    setSnapshot((current) => ({
      ...current,
      dictionary: { ...current.dictionary, status: "updating", error: null },
    }));
    try {
      const rawResult = await invokeCommand<unknown>("update_dictionary");
      const result = decodeDictionaryCommandResult(rawResult);
      if (!result) throw new Error("词典更新命令返回了无法识别的结果。");
      activeDictionaryOperationId.current = result.operationId;
      setDictionaryProgress(null);
      setSnapshot((current) => ({ ...current, dictionary: result.state }));
      await refreshSnapshot();
    } catch (reason) {
      const message = describeError(reason, "词典更新失败");
      setSnapshot((current) => ({
        ...current,
        dictionary: { ...current.dictionary, status: "failed", error: message },
      }));
      setDictionaryProgress(null);
    }
  }, [refreshSnapshot]);

  const cancelWordExampleRequest = useCallback(async () => {
    const requestId = activeWordExampleRequestId.current;
    if (!requestId) {
      setWordExample((current) => ({ ...current, requestId: null, status: "idle" }));
      return;
    }
    activeWordExampleRequestId.current = null;
    setWordExample((current) => ({ ...current, requestId: null, status: "idle" }));
    try {
      await invokeCommand("cancel_word_example", { requestId });
    } catch {
      // 取消只影响当前请求；请求已经从前端状态中移除。
    }
  }, []);

  const handleWordExampleRequested = useCallback(async (request: WordExampleRequestInput | null) => {
    if (!request) {
      const requestId = activeWordExampleRequestId.current;
      activeWordExampleRequestId.current = null;
      if (requestId) void invokeCommand("cancel_word_example", { requestId });
      setWordExample(DEFAULT_WORD_EXAMPLE_STATE);
      return;
    }
    const previousRequestId = activeWordExampleRequestId.current;
    if (previousRequestId) {
      void invokeCommand("cancel_word_example", { requestId: previousRequestId });
    }
    const requestId = crypto.randomUUID();
    activeWordExampleRequestId.current = requestId;
    setWordExample({
      exampleId: request.exampleId,
      requestId,
      translation: "",
      partOfSpeech: "",
      status: "streaming",
      cacheHit: false,
      error: null,
    });
    try {
      const rawResult = await invokeCommand<unknown>("generate_word_example", {
        request: {
          requestId,
          exampleId: request.exampleId,
          word: request.word,
          canonicalWord: request.canonicalWord,
          targetLanguage: request.targetLanguage,
        },
      });
      const result = decodeWordExampleCommandResult(rawResult);
      if (result && activeWordExampleRequestId.current === requestId) {
        applyWordExampleResult(requestId, result);
      }
    } catch (reason) {
      if (activeWordExampleRequestId.current !== requestId) return;
      activeWordExampleRequestId.current = null;
      setWordExample((current) => ({
        ...current,
        requestId: null,
        status: "failed",
        error: describeError(reason, "单词例句生成失败"),
      }));
    }
  }, [applyWordExampleResult]);

  const selectedModel = useMemo(() => {
    const known = snapshot.models.find((model) => model.id === snapshot.provider.modelId);
    return known?.label ?? snapshot.provider.modelId;
  }, [snapshot.models, snapshot.provider.modelId]);

  const handleTranslate = async () => {
    if (!translationEventsReady) {
      setError(translationEventsError ?? "翻译事件监听尚未就绪，请稍后再试。");
      return;
    }
    const text = sourceText.trim();
    if (!text) {
      setError("请先输入需要翻译的段落。");
      return;
    }
    const requestId = crypto.randomUUID();
    activeRequestId.current = requestId;
    setError(null);
    setNotice(null);
    setTranslatedText("");
    setLastCacheHit(false);
    setStatus("streaming");
    try {
      const rawResult = await invokeCommand<unknown>("translate", {
        request: {
          requestId,
          sourceText: text,
          sourceLanguage,
          targetLanguage,
          modelId: snapshot.provider.modelId,
          promptId: snapshot.provider.promptId,
        },
      });
      if (activeRequestId.current !== requestId) return;
      const result = decodeTranslationCommandResult(rawResult);
      if (!result) {
        activeRequestId.current = null;
        setError("翻译命令返回了无法识别的终态。");
        setStatus("failed");
        return;
      }
      applyTranslationResult(requestId, result);
    } catch (reason) {
      if (activeRequestId.current !== requestId) return;
      activeRequestId.current = null;
      setError(describeError(reason, "翻译请求失败"));
      setStatus("failed");
    }
  };

  const handleCancel = async () => {
    const requestId = activeRequestId.current;
    if (!requestId) return;
    setStatus("cancelling");
    try {
      const result = await invokeCommand<unknown>("cancel_translation", { requestId });
      if (activeRequestId.current !== requestId) return;
      if (typeof result !== "boolean") {
        setError("取消命令返回了无法识别的状态。");
        setStatus("streaming");
        return;
      }
      if (!result) {
        setError(null);
        setStatus("idle");
      }
    } catch (reason) {
      if (activeRequestId.current !== requestId) return;
      setError(describeError(reason, "取消请求失败"));
      setStatus("streaming");
    }
  };

  const handleCopy = async () => {
    if (!translatedText) return;
    await navigator.clipboard.writeText(translatedText);
    setNotice("译文已复制");
    window.setTimeout(() => setNotice(null), 1800);
  };

  const handleSettingsSaved = (next: AppSnapshot) => {
    setSnapshot(next);
    setNotice("设置已保存");
    window.setTimeout(() => setNotice(null), 1800);
  };

  return (
    <div className="app-shell">
      <header className="topbar">
        <div className="brand-lockup">
          <div className="brand-mark">L</div>
          <div>
            <div className="brand-name">Lilt</div>
            <div className="brand-caption">阅读辅助工具</div>
          </div>
        </div>
        <div className="topbar-status">
          <span className="status-dot" />
          本地运行
        </div>
      </header>

      <div className="workspace">
        <aside className="sidebar">
          <nav className="nav-list" aria-label="主导航">
            <NavItem icon={<Languages size={17} />} label="段落翻译" active={tab === "translate"} onClick={() => setTab("translate")} />
            <NavItem icon={<BookOpen size={17} />} label="词典" active={tab === "dictionary"} onClick={() => setTab("dictionary")} />
            <NavItem icon={<FileText size={17} />} label="术语表" active={tab === "glossary"} onClick={() => setTab("glossary")} />
            <NavItem icon={<History size={17} />} label="翻译历史" active={tab === "history"} onClick={() => setTab("history")} />
            <NavItem icon={<Settings size={17} />} label="设置" active={tab === "settings"} onClick={() => setTab("settings")} />
          </nav>
          <div className="sidebar-note">
            <WandSparkles size={16} />
            <span>首版聚焦段落翻译</span>
          </div>
        </aside>

        <main className="main-content">
          {tab === "translate" && (
            <TranslateView
              sourceText={sourceText}
              translatedText={translatedText}
              sourceLanguage={sourceLanguage}
              targetLanguage={targetLanguage}
              selectedModel={selectedModel}
              status={status}
              error={error ?? translationEventsError}
              notice={notice}
              cacheHit={lastCacheHit}
              eventsReady={translationEventsReady}
              onSourceTextChange={setSourceText}
              onSourceLanguageChange={setSourceLanguage}
              onTargetLanguageChange={setTargetLanguage}
              onTranslate={() => void handleTranslate()}
              onCancel={() => void handleCancel()}
              onCopy={() => void handleCopy()}
            />
          )}
          {tab === "dictionary" && (
            <DictionaryView
              state={snapshot.dictionary}
              history={snapshot.dictionaryHistory}
              progress={dictionaryProgress}
              targetLanguage={targetLanguage}
              wordExample={wordExample}
              onUpdate={handleDictionaryUpdate}
              onHistoryChanged={handleDictionaryHistoryChanged}
              onSnapshotChanged={refreshSnapshot}
              onWordExampleRequested={(request) => { void handleWordExampleRequested(request); }}
              onWordExampleCancelled={() => { void cancelWordExampleRequest(); }}
            />
          )}
          {tab === "glossary" && (
            <GlossaryView terms={snapshot.glossaryTerms} onChanged={() => void refreshSnapshot()} />
          )}
          {tab === "history" && <HistoryView history={snapshot.history} />}
          {tab === "settings" && (
            <SettingsView
              snapshot={snapshot}
              dictionaryProgress={dictionaryProgress}
              dictionaryEventsError={dictionaryEventsError}
              onDictionaryUpdate={handleDictionaryUpdate}
              onSaved={handleSettingsSaved}
            />
          )}
        </main>
      </div>
    </div>
  );
}

function NavItem({ icon, label, active, onClick }: { icon: React.ReactNode; label: string; active: boolean; onClick: () => void }) {
  return (
    <button className={`nav-item ${active ? "is-active" : ""}`} onClick={onClick} type="button">
      {icon}
      <span>{label}</span>
    </button>
  );
}

interface TranslateViewProps {
  sourceText: string;
  translatedText: string;
  sourceLanguage: string;
  targetLanguage: string;
  selectedModel: string;
  status: TranslationStatus;
  error: string | null;
  notice: string | null;
  cacheHit: boolean;
  eventsReady: boolean;
  onSourceTextChange: (value: string) => void;
  onSourceLanguageChange: (value: string) => void;
  onTargetLanguageChange: (value: string) => void;
  onTranslate: () => void;
  onCancel: () => void;
  onCopy: () => void;
}

function TranslateView(props: TranslateViewProps) {
  const isBusy = props.status === "streaming" || props.status === "cancelling";
  return (
    <section className="page-section translate-page">
      <div className="page-heading">
        <div>
          <p className="eyebrow">TRANSLATE</p>
          <h1>段落翻译</h1>
          <p className="page-description">保留上下文，把一段文字译成适合阅读的中文。</p>
        </div>
        <div className="model-pill">{props.selectedModel || "未配置模型"}</div>
      </div>

      <div className="translation-grid">
        <div className="translation-panel">
          <div className="panel-toolbar">
            <label htmlFor="source-language">原文</label>
            <select id="source-language" value={props.sourceLanguage} onChange={(event) => props.onSourceLanguageChange(event.target.value)}>
              {LANGUAGE_OPTIONS.map(([label, value]) => <option key={value} value={value}>{label}</option>)}
            </select>
          </div>
          <textarea
            value={props.sourceText}
            onChange={(event) => props.onSourceTextChange(event.target.value)}
            placeholder="粘贴需要翻译的英文段落……"
            spellCheck={false}
          />
          <div className="panel-footer"><span>{props.sourceText.length} 字符</span></div>
        </div>

        <div className="translation-panel result-panel">
          <div className="panel-toolbar">
            <label htmlFor="target-language">译文</label>
            <select id="target-language" value={props.targetLanguage} onChange={(event) => props.onTargetLanguageChange(event.target.value)}>
              {LANGUAGE_OPTIONS.map(([label, value]) => <option key={value} value={value}>{label}</option>)}
            </select>
          </div>
          <div className={`result-content ${props.translatedText ? "has-content" : ""}`}>
            {props.translatedText || <span className="empty-result">译文会显示在这里</span>}
            {props.status === "streaming" && <span className="stream-caret" />}
          </div>
          <div className="panel-footer result-footer">
            <span>{props.cacheHit ? "来自段落缓存" : props.status === "completed" ? "已完成" : ""}</span>
            <button className="icon-button" title="复制译文" aria-label="复制译文" onClick={props.onCopy} disabled={!props.translatedText} type="button"><Copy size={16} /></button>
          </div>
        </div>
      </div>

      <div className="action-row">
        <div className="message-area">
          {props.error && <span className="error-message">{props.error}</span>}
          {!props.error && props.notice && <span className="notice-message">{props.notice}</span>}
        </div>
        {isBusy ? (
          <button className="primary-button cancel-button" type="button" onClick={props.onCancel} disabled={props.status === "cancelling"}>
            {props.status === "cancelling" ? <LoaderCircle className="spin" size={16} /> : <Square size={14} fill="currentColor" />}
            {props.status === "cancelling" ? "正在取消" : "取消翻译"}
          </button>
        ) : (
          <button className="primary-button" type="button" onClick={props.onTranslate} disabled={!props.eventsReady}>
            <WandSparkles size={16} />
            开始翻译
          </button>
        )}
      </div>

      <div className="quick-facts">
        <div><span className="fact-label">当前方向</span><strong>{languageName(props.sourceLanguage)} → {languageName(props.targetLanguage)}</strong></div>
        <div><span className="fact-label">段落缓存</span><strong>默认开启</strong></div>
        <div><span className="fact-label">历史记录</span><strong>始终保留</strong></div>
      </div>
    </section>
  );
}

function languageName(value: string): string {
  return LANGUAGE_OPTIONS.find(([, code]) => code === value)?.[0] ?? value;
}

function GlossaryView({ terms, onChanged }: { terms: GlossaryTerm[]; onChanged: () => void }) {
  const [source, setSource] = useState("");
  const [target, setTarget] = useState("");
  const [note, setNote] = useState("");
  const [error, setError] = useState<string | null>(null);
  const addTerm = async () => {
    if (!source.trim() || !target.trim()) {
      setError("原文术语和译文都不能为空。");
      return;
    }
    try {
      await invokeCommand("upsert_glossary_term", { source: source.trim(), target: target.trim(), note: note.trim() || null });
      setSource(""); setTarget(""); setNote(""); setError(null); onChanged();
    } catch (reason) {
      setError(describeError(reason, "术语保存失败"));
    }
  };
  return (
    <section className="page-section narrow-page">
      <PageTitle eyebrow="GLOSSARY" title="术语表" description="全局术语会在段落翻译时按命中情况加入提示。" />
      <div className="simple-card">
        <div className="form-grid glossary-form">
          <label>原文<input value={source} onChange={(event) => setSource(event.target.value)} placeholder="例如：embedding" /></label>
          <label>译文<input value={target} onChange={(event) => setTarget(event.target.value)} placeholder="例如：嵌入" /></label>
          <label className="wide-field">备注<input value={note} onChange={(event) => setNote(event.target.value)} placeholder="可选" /></label>
        </div>
        <div className="form-actions"><span className="error-message">{error}</span><button className="secondary-button" type="button" onClick={() => void addTerm()}>添加术语</button></div>
      </div>
      <div className="list-card">
        <div className="list-card-heading"><strong>已添加术语</strong><span>{terms.length} 条</span></div>
        {terms.length === 0 ? <div className="empty-list">还没有术语。</div> : terms.map((term) => <GlossaryRow key={term.id} term={term} onChanged={onChanged} />)}
      </div>
    </section>
  );
}

function GlossaryRow({ term, onChanged }: { term: GlossaryTerm; onChanged: () => void }) {
  const remove = async () => {
    await invokeCommand("delete_glossary_term", { id: term.id });
    onChanged();
  };
  return <div className="list-row"><div><strong>{term.source}</strong><span className="arrow">→</span><span>{term.target}</span>{term.note && <small>{term.note}</small>}</div><button className="text-button danger-text" type="button" onClick={() => void remove()}>删除</button></div>;
}

function HistoryView({ history }: { history: HistoryEntry[] }) {
  return (
    <section className="page-section narrow-page">
      <PageTitle eyebrow="HISTORY" title="翻译历史" description="历史记录属于客户端固定功能，按设置中的条数保留。" />
      <div className="list-card history-card">
        <div className="list-card-heading"><strong>最近翻译</strong><span>{history.length} 条</span></div>
        {history.length === 0 ? <div className="empty-list">完成一次段落翻译后，记录会出现在这里。</div> : history.map((item) => <HistoryRow key={item.id} item={item} />)}
      </div>
    </section>
  );
}

function HistoryRow({ item }: { item: HistoryEntry }) {
  return <article className="history-row"><div className="history-meta"><span>{formatDate(item.createdAt)}</span><span>{item.modelId}</span>{item.cacheHit && <span className="tag">缓存命中</span>}</div><p className="history-source">{item.sourceText}</p><p className="history-result">{item.translatedText}</p></article>;
}

function SettingsView({
  snapshot,
  dictionaryProgress,
  dictionaryEventsError,
  onDictionaryUpdate,
  onSaved,
}: {
  snapshot: AppSnapshot;
  dictionaryProgress: DictionaryProgress | null;
  dictionaryEventsError: string | null;
  onDictionaryUpdate: () => Promise<void>;
  onSaved: (snapshot: AppSnapshot) => void;
}) {
  const [baseUrl, setBaseUrl] = useState(snapshot.provider.baseUrl);
  const [modelId, setModelId] = useState(snapshot.provider.modelId);
  const [promptId, setPromptId] = useState(snapshot.provider.promptId);
  const [apiKey, setApiKey] = useState("");
  const [settings, setSettings] = useState<AppSettings>(snapshot.settings);
  const [selectionMode, setSelectionMode] = useState(snapshot.settings.selectionMode);
  const [selectionShortcut, setSelectionShortcut] = useState(snapshot.settings.selectionShortcut);
  const [selectionStatus, setSelectionStatus] = useState<SelectionRuntimeStatus | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setBaseUrl(snapshot.provider.baseUrl);
    setModelId(snapshot.provider.modelId);
    setPromptId(snapshot.provider.promptId);
    setSettings(snapshot.settings);
    setSelectionMode(snapshot.settings.selectionMode);
    setSelectionShortcut(snapshot.settings.selectionShortcut);
  }, [snapshot]);

  useEffect(() => {
    let disposed = false;
    const loadStatus = async () => {
      try {
        const raw = await invokeCommand<unknown>("get_selection_status");
        const next = decodeSelectionStatus(raw);
        if (!disposed && next) setSelectionStatus(next);
      } catch {
        // 设置页仍可编辑，运行时状态会在后端完成初始化后再次同步。
      }
    };
    void loadStatus();
    let unlisten: (() => void) | null = null;
    void listenTo<unknown>("selection_status_changed", (payload) => {
      const next = decodeSelectionStatus(payload);
      if (!disposed && next) setSelectionStatus(next);
    }).then((next) => {
      if (disposed) next();
      else unlisten = next;
    }).catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [snapshot.settings.selectionMode, snapshot.settings.selectionShortcut]);

  const saveProvider = async () => {
    setError(null);
    setMessage(null);
    try {
      await invokeCommand("save_provider_config", { baseUrl, modelId, promptId, apiKey: apiKey || null });
      setApiKey("");
      const next = await invokeCommand<AppSnapshot>("get_app_snapshot");
      onSaved(next);
      setMessage("Provider 设置已保存");
    } catch (reason) {
      setError(describeError(reason, "Provider 设置保存失败"));
      setMessage(null);
    }
  };

  const saveAppSettings = async () => {
    setError(null);
    setMessage(null);
    try {
      await invokeCommand("save_app_settings", {
        historyRetention: settings.historyRetention,
        cacheEnabled: settings.cacheEnabled,
        cacheMaxBytes: settings.cacheMaxBytes,
        wordAiCacheEnabled: settings.wordAiCacheEnabled,
        paragraphExampleLookupEnabled: settings.paragraphExampleLookupEnabled,
      });
      const next = await invokeCommand<AppSnapshot>("get_app_snapshot");
      onSaved(next);
      setMessage("本地设置已保存");
    } catch (reason) {
      setError(describeError(reason, "本地设置保存失败"));
      setMessage(null);
    }
  };

  const saveSelectionSettings = async () => {
    const shortcut = selectionShortcut.trim();
    if (!shortcut) {
      setError("快捷键不能为空。");
      setMessage(null);
      return;
    }
    setError(null);
    setMessage(null);
    try {
      await invokeCommand("configure_selection", { mode: selectionMode, shortcut });
      const rawStatus = await invokeCommand<unknown>("get_selection_status");
      const nextStatus = decodeSelectionStatus(rawStatus);
      if (nextStatus) setSelectionStatus(nextStatus);
      const next = await invokeCommand<AppSnapshot>("get_app_snapshot");
      onSaved(next);
      setMessage("划词翻译设置已保存");
    } catch (reason) {
      setError(describeError(reason, "划词翻译设置保存失败"));
      setMessage(null);
    }
  };

  const fetchModels = async () => {
    setError(null);
    setMessage(null);
    try {
      const models = await invokeCommand<ModelInfo[]>("fetch_models", {
        baseUrl: baseUrl.trim() || null,
        apiKey: apiKey.trim() || null,
      });
      setMessage(`模型列表已更新，共 ${models.length} 个模型`);
    } catch (reason) {
      setError(describeError(reason, "模型列表读取失败，可手动填写 Model ID"));
      setMessage(null);
    }
  };

  const dictionaryUpdating = snapshot.dictionary.status === "updating" || dictionaryProgress !== null;
  const dictionaryStatusLabel = snapshot.dictionary.status === "ready"
    ? "已安装"
    : snapshot.dictionary.status === "updating"
      ? "更新中"
      : snapshot.dictionary.status === "failed"
        ? "需要处理"
        : "未安装";
  const dictionaryProgressPercent = dictionaryProgress && dictionaryProgress.total > 0
    ? Math.min(100, Math.round((dictionaryProgress.current / dictionaryProgress.total) * 100))
    : 0;
  const activeSelectionMode = selectionStatus?.mode ?? snapshot.settings.selectionMode;

  return (
    <section className="page-section narrow-page">
      <PageTitle eyebrow="SETTINGS" title="设置" description="配置模型连接，并管理本地历史与段落缓存。" />
      <div className="settings-stack">
        <div className="simple-card">
          <div className="card-heading"><div><strong>OpenAI-compatible Provider</strong><span>首版仅支持这一种协议</span></div><span className={`connection-status ${snapshot.provider.hasApiKey ? "connected" : ""}`}>{snapshot.provider.hasApiKey ? "已配置密钥" : "未配置密钥"}</span></div>
          <div className="form-grid">
            <label className="wide-field">Base URL<input value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} placeholder="https://api.openai.com/v1" /></label>
            <label>Model ID<input value={modelId} onChange={(event) => setModelId(event.target.value)} placeholder="gpt-4o-mini" /></label>
            <label>Prompt<select value={promptId} onChange={(event) => setPromptId(event.target.value)}>{snapshot.prompts.map((prompt) => <option key={prompt.id} value={prompt.id}>{prompt.name} · v{prompt.version}</option>)}</select></label>
            <label className="wide-field">API Key<input type="password" value={apiKey} onChange={(event) => setApiKey(event.target.value)} placeholder={snapshot.provider.hasApiKey ? "已保存，留空表示不修改" : "保存在 Windows 凭据管理器"} autoComplete="off" /></label>
          </div>
          <div className="form-actions"><span className="muted-text">模型列表读取失败时，Model ID 仍可手动填写。</span><div className="button-group"><button className="secondary-button" type="button" onClick={() => void fetchModels()}>读取模型</button><button className="primary-button small-button" type="button" onClick={() => void saveProvider()}>保存 Provider</button></div></div>
        </div>

        <div className="simple-card dictionary-settings-card">
          <div className="card-heading"><div><strong>本地词典</strong><span>open-dictionary，离线查询，不依赖 Provider</span></div><span className={`connection-status ${snapshot.dictionary.status === "ready" ? "connected" : ""}`}>{dictionaryStatusLabel}</span></div>
          <div className="dictionary-settings-grid">
            <div><span className="fact-label">Release</span><strong>{snapshot.dictionary.installedRelease ?? "尚未安装"}</strong></div>
            <div><span className="fact-label">词条数量</span><strong>{snapshot.dictionary.entryCount?.toLocaleString("zh-CN") ?? "—"}</strong></div>
            <div><span className="fact-label">占用空间</span><strong>{formatBytes(snapshot.dictionary.cacheSizeBytes)}</strong></div>
          </div>
          {dictionaryUpdating && (
            <div className="dictionary-progress settings-progress" aria-live="polite">
              <div className="dictionary-progress-label"><span>{dictionaryProgress?.phase === "verify" ? "正在校验" : dictionaryProgress?.phase === "extract" ? "正在解压" : "正在下载"}</span><span>{dictionaryProgress ? `${dictionaryProgressPercent}%` : ""}</span></div>
              <div className="dictionary-progress-track"><span style={{ width: `${dictionaryProgressPercent}%` }} /></div>
            </div>
          )}
          {dictionaryEventsError && <p className="error-message settings-message">{dictionaryEventsError}</p>}
          {snapshot.dictionary.error && !dictionaryUpdating && <p className="error-message settings-message">{snapshot.dictionary.error}</p>}
          <div className="form-actions"><span className="muted-text">数据版本 {snapshot.dictionary.distributionSchemaVersion ?? "—"} · SQLite {snapshot.dictionary.sqliteSchemaVersion ?? "—"}</span><button className="secondary-button" type="button" onClick={() => void onDictionaryUpdate()} disabled={dictionaryUpdating}>{dictionaryUpdating ? <LoaderCircle className="spin" size={15} /> : <BookOpen size={15} />}{snapshot.dictionary.status === "ready" ? "手动更新" : "下载词典"}</button></div>
        </div>

        <div className="simple-card selection-settings-card">
          <div className="card-heading"><div><strong>划词翻译</strong><span>从其他 Windows 应用读取选中文本，浮窗复用当前翻译方向。</span></div><span className={`connection-status ${selectionStatus && (activeSelectionMode === "shortcut" ? selectionStatus.shortcutRegistered : selectionStatus.uiAutomationReady) ? "connected" : ""}`}>{activeSelectionMode === "shortcut" ? selectionStatus?.shortcutRegistered ? "快捷键已启用" : "快捷键未启用" : selectionStatus?.uiAutomationReady ? "自动监听已启用" : "自动监听不可用"}</span></div>
          <div className="form-grid selection-settings-grid">
            <label>触发方式<select value={selectionMode} onChange={(event) => setSelectionMode(event.target.value as AppSettings["selectionMode"])}><option value="shortcut">按快捷键</option><option value="automatic">自动监听选区</option></select></label>
            <label>快捷键<input value={selectionShortcut} onChange={(event) => setSelectionShortcut(event.target.value)} placeholder="Ctrl+Shift+L" /></label>
          </div>
          <p className="settings-hint">快捷键格式使用 Ctrl+Shift+L 这样的组合。按快捷键模式读取当前选区；自动监听模式在选区稳定 500 毫秒后显示结果。自动模式仍保留快捷键设置，切换回来即可使用。</p>
          {selectionStatus?.message && <p className="error-message settings-message">{selectionStatus.message}</p>}
          <div className="form-actions"><span className="muted-text">当前状态：{activeSelectionMode === "shortcut" ? selectionStatus?.shortcutRegistered ? "快捷键正常" : "等待注册" : selectionStatus?.uiAutomationReady ? "UI Automation 正常" : "等待初始化"}</span><button className="primary-button small-button" type="button" onClick={() => void saveSelectionSettings()}>保存划词设置</button></div>
        </div>

        <div className="simple-card">
          <div className="card-heading"><div><strong>本地数据</strong><span>数据只保存在当前设备</span></div></div>
          <label className="setting-line"><span><strong>翻译历史保留条数</strong><small>历史功能不可关闭，只控制保留数量。</small></span><input className="number-input" type="number" min={1} max={1000} value={settings.historyRetention} onChange={(event) => setSettings({ ...settings, historyRetention: Number(event.target.value) })} /></label>
          <label className="setting-line"><span><strong>启用段落翻译缓存</strong><small>缓存命中后仍会写入一条历史记录。</small></span><input type="checkbox" checked={settings.cacheEnabled} onChange={(event) => setSettings({ ...settings, cacheEnabled: event.target.checked })} /></label>
          <label className="setting-line"><span><strong>缓存单词 AI 见解</strong><small>关闭后，每次查询都会重新生成例句译文和词性。</small></span><input type="checkbox" checked={settings.wordAiCacheEnabled} onChange={(event) => setSettings({ ...settings, wordAiCacheEnabled: event.target.checked })} /></label>
          <label className="setting-line"><span><strong>从段落缓存查找例句</strong><small>关闭后，词典查询不再读取段落翻译缓存中的例句。</small></span><input type="checkbox" checked={settings.paragraphExampleLookupEnabled} onChange={(event) => setSettings({ ...settings, paragraphExampleLookupEnabled: event.target.checked })} /></label>
          <label className="setting-line slider-line"><span><strong>段落缓存上限</strong><small>已使用 {formatBytes(snapshot.cacheStats.usageBytes)}，上限 {formatBytes(settings.cacheMaxBytes)}。</small></span><input type="range" min={16} max={2048} step={16} value={Math.round(settings.cacheMaxBytes / (1024 * 1024))} onChange={(event) => setSettings({ ...settings, cacheMaxBytes: Number(event.target.value) * 1024 * 1024 })} /></label>
          <div className="form-actions"><span className="muted-text">缓存不包含 API Key。</span><button className="primary-button small-button" type="button" onClick={() => void saveAppSettings()}>保存本地设置</button></div>
        </div>
      </div>
      {message && <p className="notice-message settings-message">{message}</p>}
      {error && <p className="error-message settings-message">{error}</p>}
    </section>
  );
}

function PageTitle({ eyebrow, title, description }: { eyebrow: string; title: string; description: string }) {
  return <div className="page-heading"><div><p className="eyebrow">{eyebrow}</p><h1>{title}</h1><p className="page-description">{description}</p></div></div>;
}

export default App;
