import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type AnimationEvent as ReactAnimationEvent, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent as ReactMouseEvent, type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Check, ChevronDown, Copy, FileText, History, Languages, LoaderCircle, Settings, Square, WandSparkles, BookOpen, X, Maximize2, Minimize2, Minus, Trash2 } from "lucide-react";
import liltLogo from "../lilt_logo.svg";
import { describeError } from "./lib/errors";
import { invokeCommand, listenTo } from "./lib/tauri";
import DictionaryView, { type DictionaryProgress, type WordExampleRequestInput } from "./DictionaryView";
import type { DictionaryOpenRequest } from "./DictionaryView";
import PersonalDictionaryView from "./PersonalDictionaryView";
import {
  DEFAULT_SNAPSHOT,
  type AppSettings,
  type AppSnapshot,
  type AppTab,
  type CloseBehavior,
  type GlossaryTerm,
  type HistoryEntry,
  type ModelInfo,
  type PersonalDictionaryEntry,
  type Prompt,
  type ThinkingEffort,
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
  decodePrompt,
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

interface TranslationSummary {
  durationMs: number;
  cacheHit: boolean;
}

function formatTranslationSummary(summary: TranslationSummary): string {
  return `${(summary.durationMs / 1000).toFixed(2)}秒·${summary.cacheHit ? "缓存命中" : "未命中缓存"}`;
}

function languageLabel(value: string): string {
  return LANGUAGE_OPTIONS.find(([, code]) => code === value)?.[0] ?? value;
}

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

const REDUCED_MOTION_QUERY = "(prefers-reduced-motion: reduce)";

function usePrefersReducedMotion(): boolean {
  const [reducedMotion, setReducedMotion] = useState(() => (
    typeof window !== "undefined"
    && typeof window.matchMedia === "function"
    && window.matchMedia(REDUCED_MOTION_QUERY).matches
  ));

  useEffect(() => {
    if (typeof window.matchMedia !== "function") return;
    const mediaQuery = window.matchMedia(REDUCED_MOTION_QUERY);
    const update = () => setReducedMotion(mediaQuery.matches);
    update();
    mediaQuery.addEventListener("change", update);
    return () => mediaQuery.removeEventListener("change", update);
  }, []);

  return reducedMotion;
}

interface PageTransitionLayer {
  id: number;
  node: ReactNode;
}

function PageTransition({ activeKey, children }: { activeKey: string; children: ReactNode }) {
  const reducedMotion = usePrefersReducedMotion();
  const [outgoing, setOutgoing] = useState<PageTransitionLayer | null>(null);
  const previousKeyRef = useRef(activeKey);
  const previousChildrenRef = useRef<ReactNode>(children);
  const transitionIdRef = useRef(0);

  useLayoutEffect(() => {
    if (reducedMotion) {
      previousKeyRef.current = activeKey;
      previousChildrenRef.current = children;
      setOutgoing(null);
      return;
    }

    if (activeKey === previousKeyRef.current) {
      previousChildrenRef.current = children;
      return;
    }

    const previousChildren = previousChildrenRef.current;
    previousKeyRef.current = activeKey;
    previousChildrenRef.current = children;
    const activeElement = document.activeElement;
    if (activeElement instanceof HTMLElement && activeElement.closest(".page-transition-layer")) {
      activeElement.blur();
    }
    transitionIdRef.current += 1;
    setOutgoing({ id: transitionIdRef.current, node: previousChildren });
  }, [activeKey, children, reducedMotion]);

  const handleOutgoingAnimationEnd = useCallback((id: number, event: ReactAnimationEvent<HTMLDivElement>) => {
    if (event.target !== event.currentTarget) return;
    setOutgoing((current) => current?.id === id ? null : current);
  }, []);

  return (
    <div className="page-transition-stack">
      {outgoing && (
        <div
          key={`outgoing-${outgoing.id}`}
          className="page-transition-layer page-transition-layer-outgoing"
          aria-hidden="true"
          onAnimationEnd={(event) => handleOutgoingAnimationEnd(outgoing.id, event)}
        >
          {outgoing.node}
        </div>
      )}
      <div key={`incoming-${activeKey}`} className="page-transition-layer page-transition-layer-incoming">
        {children}
      </div>
    </div>
  );
}

type OverlayPhase = "opening" | "open" | "closing";

function AnimatedOverlay({
  open,
  className,
  children,
  onClosed,
  onBackdropClick,
}: {
  open: boolean;
  className: string;
  children: ReactNode;
  onClosed: () => void;
  onBackdropClick?: (event: ReactMouseEvent<HTMLDivElement>) => void;
}) {
  const reducedMotion = usePrefersReducedMotion();
  const [phase, setPhase] = useState<OverlayPhase>(open ? "opening" : "closing");
  const closeNotifiedRef = useRef(false);
  const overlayRef = useRef<HTMLDivElement | null>(null);

  const notifyClosed = useCallback(() => {
    if (closeNotifiedRef.current) return;
    closeNotifiedRef.current = true;
    onClosed();
  }, [onClosed]);

  useEffect(() => {
    if (open) {
      closeNotifiedRef.current = false;
      setPhase("opening");
      if (reducedMotion) {
        setPhase("open");
        return;
      }
      return undefined;
    }

    setPhase("closing");
    const activeElement = document.activeElement;
    if (activeElement instanceof HTMLElement && overlayRef.current?.contains(activeElement)) {
      activeElement.blur();
    }
    if (reducedMotion) notifyClosed();
    return undefined;
  }, [notifyClosed, open, reducedMotion]);

  const handleAnimationEnd = useCallback((event: ReactAnimationEvent<HTMLDivElement>) => {
    if (event.target !== event.currentTarget) return;
    if (open && phase === "opening") {
      setPhase("open");
      return;
    }
    if (!open && phase === "closing") notifyClosed();
  }, [notifyClosed, open, phase]);

  return (
    <div
      ref={overlayRef}
      className={`${className} overlay-phase-${phase}`}
      role="presentation"
      aria-hidden={phase === "closing"}
      onClick={onBackdropClick}
      onAnimationEnd={handleAnimationEnd}
    >
      {children}
    </div>
  );
}

function FeedbackMessage({
  message,
  kind,
  as = "span",
  className = "",
}: {
  message: string | null;
  kind: "error" | "notice";
  as?: "span" | "p";
  className?: string;
}) {
  const reducedMotion = usePrefersReducedMotion();
  const [visible, setVisible] = useState<{ message: string; kind: "error" | "notice" } | null>(
    () => message ? { message, kind } : null,
  );
  const [phase, setPhase] = useState<"entering" | "visible" | "exiting">("visible");
  const frameRef = useRef<number | null>(null);
  const hasVisibleRef = useRef(Boolean(message));

  useEffect(() => {
    if (frameRef.current !== null) {
      window.cancelAnimationFrame(frameRef.current);
      frameRef.current = null;
    }

    if (message) {
      hasVisibleRef.current = true;
      setVisible({ message, kind });
      if (reducedMotion) {
        setPhase("visible");
        return;
      }
      setPhase("entering");
      frameRef.current = window.requestAnimationFrame(() => {
        frameRef.current = null;
        setPhase("visible");
      });
      return () => {
        if (frameRef.current !== null) {
          window.cancelAnimationFrame(frameRef.current);
          frameRef.current = null;
        }
      };
    }

    if (!hasVisibleRef.current) return undefined;
    if (reducedMotion) {
      hasVisibleRef.current = false;
      setVisible(null);
      setPhase("visible");
    } else {
      setPhase("exiting");
    }
    return undefined;
  }, [kind, message, reducedMotion]);

  const handleAnimationEnd = useCallback((event: ReactAnimationEvent<HTMLElement>) => {
    if (event.target !== event.currentTarget || phase !== "exiting") return;
    hasVisibleRef.current = false;
    setVisible(null);
    setPhase("visible");
  }, [phase]);

  if (!visible) return null;
  const Element = as;
  return (
    <Element
      key={`${visible.kind}-${visible.message}`}
      className={`feedback-message ${visible.kind}-message feedback-${phase} ${className}`.trim()}
      onAnimationEnd={handleAnimationEnd}
    >
      {visible.message}
    </Element>
  );
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
  const [translationSummary, setTranslationSummary] = useState<TranslationSummary | null>(null);
  const [translationEventsReady, setTranslationEventsReady] = useState(false);
  const [translationEventsError, setTranslationEventsError] = useState<string | null>(null);
  const [dictionaryProgress, setDictionaryProgress] = useState<DictionaryProgress | null>(null);
  const [dictionaryEventsError, setDictionaryEventsError] = useState<string | null>(null);
  const [wordExample, setWordExample] = useState<WordExampleState>(DEFAULT_WORD_EXAMPLE_STATE);
  const [dictionaryOpenRequest, setDictionaryOpenRequest] = useState<DictionaryOpenRequest | null>(null);
  const [closeDialogOpen, setCloseDialogOpen] = useState(false);
  const [closeDialogMounted, setCloseDialogMounted] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsOverlayMounted, setSettingsOverlayMounted] = useState(false);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [historyOverlayMounted, setHistoryOverlayMounted] = useState(false);
  const [mainWindowMaximized, setMainWindowMaximized] = useState(false);
  const activeRequestId = useRef<string | null>(null);
  const translationStartedAt = useRef<number | null>(null);
  const activeDictionaryOperationId = useRef<string | null>(null);
  const activeWordExampleRequestId = useRef<string | null>(null);
  const settingsDialogRef = useRef<HTMLDivElement | null>(null);
  const settingsReturnFocusRef = useRef<HTMLElement | null>(null);
  const historyDialogRef = useRef<HTMLDivElement | null>(null);
  const historyReturnFocusRef = useRef<HTMLElement | null>(null);
  const closeDialogReturnFocusRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    const window = getCurrentWindow();
    let disposed = false;
    let unlisten: (() => void) | null = null;
    const syncMaximized = async () => {
      try {
        const maximized = await window.isMaximized();
        if (!disposed) setMainWindowMaximized(maximized);
      } catch {
        // 浏览器预览模式没有 Tauri 窗口状态，保留默认的未最大化状态。
      }
    };
    void syncMaximized();
    void window.onResized(() => {
      void syncMaximized();
    }).then((next) => {
      if (disposed) next();
      else unlisten = next;
    }).catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const openSettings = useCallback(() => {
    settingsReturnFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setSettingsOverlayMounted(true);
    setSettingsOpen(true);
  }, []);

  const requestSettingsClose = useCallback(() => {
    setSettingsOpen(false);
  }, []);

  const handleSettingsClosed = useCallback(() => {
    setSettingsOverlayMounted(false);
    const previouslyFocused = settingsReturnFocusRef.current;
    settingsReturnFocusRef.current = null;
    previouslyFocused?.focus();
  }, []);

  const openHistory = useCallback(() => {
    historyReturnFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setHistoryOverlayMounted(true);
    setHistoryOpen(true);
  }, []);

  const requestHistoryClose = useCallback(() => {
    setHistoryOpen(false);
  }, []);

  const handleHistoryClosed = useCallback(() => {
    setHistoryOverlayMounted(false);
    const previouslyFocused = historyReturnFocusRef.current;
    historyReturnFocusRef.current = null;
    previouslyFocused?.focus();
  }, []);

  useEffect(() => {
    if (!settingsOpen) return;
    const frame = window.requestAnimationFrame(() => settingsDialogRef.current?.focus());
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      requestSettingsClose();
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      window.cancelAnimationFrame(frame);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [requestSettingsClose, settingsOpen]);

  useEffect(() => {
    if (!historyOpen) return;
    const frame = window.requestAnimationFrame(() => historyDialogRef.current?.focus());
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      requestHistoryClose();
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      window.cancelAnimationFrame(frame);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [historyOpen, requestHistoryClose]);

  const handleTitlebarMouseDown = useCallback((event: ReactMouseEvent<HTMLElement>) => {
    if (event.button !== 0) return;
    if (event.target instanceof Element && event.target.closest("button, a, input, select, textarea, [data-no-drag]")) return;
    event.preventDefault();
    void getCurrentWindow().startDragging().catch(() => undefined);
  }, []);

  const minimizeMainWindow = useCallback(() => {
    void getCurrentWindow().minimize().catch((reason) => {
      setError(describeError(reason, "无法最小化主窗口"));
    });
  }, []);

  const toggleMainWindowMaximized = useCallback(async () => {
    try {
      await getCurrentWindow().toggleMaximize();
      setMainWindowMaximized(await getCurrentWindow().isMaximized());
    } catch (reason) {
      setError(describeError(reason, "无法切换主窗口大小"));
    }
  }, []);

  const closeMainWindow = useCallback(() => {
    void getCurrentWindow().close().catch((reason) => {
      setError(describeError(reason, "无法关闭主窗口"));
    });
  }, []);

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

  const handlePersonalDictionaryChanged = useCallback(async () => {
    await refreshSnapshot();
  }, [refreshSnapshot]);

  const openPersonalWord = useCallback((entry: PersonalDictionaryEntry) => {
    setDictionaryOpenRequest({
      requestId: crypto.randomUUID(),
      lookupWord: entry.lookupWord,
      canonicalWord: entry.canonicalWord,
    });
    setTab("dictionary");
  }, []);

  const removePersonalWord = useCallback(async (entry: PersonalDictionaryEntry) => {
    try {
      await invokeCommand("remove_personal_word", { canonicalWord: entry.canonicalWord });
      await refreshSnapshot();
    } catch (reason) {
      setError(describeError(reason, "移除个人词典词条失败"));
    }
  }, [refreshSnapshot]);

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

  const openCloseDialog = useCallback(() => {
    closeDialogReturnFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setCloseDialogMounted(true);
    setCloseDialogOpen(true);
  }, []);

  const requestCloseDialog = useCallback(() => {
    setCloseDialogOpen(false);
  }, []);

  const handleCloseDialogClosed = useCallback(() => {
    setCloseDialogMounted(false);
    const previouslyFocused = closeDialogReturnFocusRef.current;
    closeDialogReturnFocusRef.current = null;
    previouslyFocused?.focus();
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listenTo<unknown>("window_close_requested", () => {
      if (!disposed) openCloseDialog();
    }).then((next) => {
      if (disposed) next();
      else unlisten = next;
    }).catch((reason) => {
      if (!disposed) setError(describeError(reason, "关闭确认事件监听初始化失败"));
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [openCloseDialog]);

  const applyTranslationResult = useCallback((requestId: string, result: TranslationCommandResult) => {
    if (requestId !== activeRequestId.current) return;
    activeRequestId.current = null;
    const startedAt = translationStartedAt.current;
    translationStartedAt.current = null;
    switch (result.outcome) {
      case "completed":
        setTranslatedText(result.content ?? "");
        setTranslationSummary({
          durationMs: startedAt === null ? 0 : Math.max(0, performance.now() - startedAt),
          cacheHit: result.cacheHit,
        });
        setError(null);
        setStatus("completed");
        void refreshSnapshot();
        break;
      case "cancelled":
        setTranslationSummary(null);
        setError(null);
        setStatus("idle");
        break;
      case "failed":
        setTranslationSummary(null);
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
    setTranslationSummary(null);
    translationStartedAt.current = performance.now();
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
        translationStartedAt.current = null;
        setError("翻译命令返回了无法识别的终态。");
        setStatus("failed");
        return;
      }
      applyTranslationResult(requestId, result);
    } catch (reason) {
      if (activeRequestId.current !== requestId) return;
      activeRequestId.current = null;
      translationStartedAt.current = null;
      setTranslationSummary(null);
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
        translationStartedAt.current = null;
        setTranslationSummary(null);
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

  const openPersonalDictionary = useCallback(() => {
    setTab("personal");
  }, []);
  const usesBoundedListLayout = tab === "personal" || tab === "glossary";

  return (
    <div className="app-shell">
      <div className="main-surface">
      <header className="app-chrome" onMouseDown={handleTitlebarMouseDown}>
        <div className="brand-lockup">
          <img className="brand-logo" src={liltLogo} alt="" />
          <span className="brand-name">Lilt</span>
        </div>
        <ModeSwitcher activeTab={tab === "personal" ? "dictionary" : tab} onChange={setTab} />
        <div className="chrome-actions" data-no-drag>
          <button className={`chrome-icon-button ${settingsOpen ? "is-active" : ""}`} type="button" onClick={openSettings} aria-label="打开设置" aria-expanded={settingsOpen} title="设置"><Settings size={16} /></button>
          <div className="window-controls" data-no-drag>
            <button className="window-control" type="button" onClick={minimizeMainWindow} aria-label="最小化窗口" title="最小化"><Minus size={14} /></button>
            <button className="window-control" type="button" onClick={() => void toggleMainWindowMaximized()} aria-label={mainWindowMaximized ? "还原窗口" : "最大化窗口"} title={mainWindowMaximized ? "还原" : "最大化"}>{mainWindowMaximized ? <Minimize2 size={13} /> : <Maximize2 size={13} />}</button>
            <button className="window-control window-control-close" type="button" onClick={closeMainWindow} aria-label="关闭窗口" title="关闭"><X size={14} /></button>
          </div>
        </div>
      </header>

      <main className={`main-content ${tab === "translate" ? "translate-main-content" : ""} ${usesBoundedListLayout ? "bounded-list-main-content" : ""}`}>
        <PageTransition activeKey={tab}>
          <div className="page-view" key={tab}>
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
                translationSummary={translationSummary}
                eventsReady={translationEventsReady}
                onSourceTextChange={setSourceText}
                onSourceLanguageChange={setSourceLanguage}
                onTargetLanguageChange={setTargetLanguage}
                onTranslate={() => void handleTranslate()}
                onCancel={() => void handleCancel()}
                onCopy={() => void handleCopy()}
                history={snapshot.history}
                onOpenHistory={openHistory}
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
                personalDictionary={snapshot.personalDictionary}
                openRequest={dictionaryOpenRequest}
                onOpenRequestHandled={() => setDictionaryOpenRequest(null)}
                onPersonalDictionaryChanged={handlePersonalDictionaryChanged}
                onOpenPersonalDictionary={openPersonalDictionary}
              />
            )}
            {tab === "personal" && (
              <PersonalDictionaryView
                entries={snapshot.personalDictionary}
                onOpen={openPersonalWord}
                onRemove={(entry) => { void removePersonalWord(entry); }}
              />
            )}
            {tab === "glossary" && (
              <GlossaryView terms={snapshot.glossaryTerms} onChanged={() => void refreshSnapshot()} />
            )}
          </div>
        </PageTransition>
      </main>
      </div>
      {historyOverlayMounted && (
        <AnimatedOverlay
          className="settings-overlay history-overlay"
          open={historyOpen}
          onClosed={handleHistoryClosed}
          onBackdropClick={(event) => {
            if (event.target === event.currentTarget) requestHistoryClose();
          }}
        >
          <div className="settings-dialog history-dialog" ref={historyDialogRef} role="dialog" aria-modal="true" aria-labelledby="history-dialog-title" tabIndex={-1}>
            <div className="settings-dialog-heading">
              <div>
                <span className="settings-dialog-eyebrow">HISTORY</span>
                <strong id="history-dialog-title">翻译历史</strong>
              </div>
              <button className="icon-button" type="button" onClick={requestHistoryClose} aria-label="关闭翻译历史" title="关闭翻译历史"><X size={17} /></button>
            </div>
            <div className="settings-dialog-scroll history-dialog-scroll">
              <HistoryContent history={snapshot.history} />
            </div>
          </div>
        </AnimatedOverlay>
      )}
      {settingsOverlayMounted && (
        <AnimatedOverlay
          className="settings-overlay"
          open={settingsOpen}
          onClosed={handleSettingsClosed}
          onBackdropClick={(event) => {
            if (event.target === event.currentTarget) requestSettingsClose();
          }}
        >
          <div className="settings-dialog" ref={settingsDialogRef} role="dialog" aria-modal="true" aria-labelledby="settings-dialog-title" tabIndex={-1}>
            <div className="settings-dialog-heading">
              <div>
                <span className="settings-dialog-eyebrow">SETTINGS</span>
                <strong id="settings-dialog-title">设置</strong>
              </div>
              <button className="icon-button" type="button" onClick={requestSettingsClose} aria-label="关闭设置" title="关闭设置"><X size={17} /></button>
            </div>
            <div className="settings-dialog-scroll">
              <SettingsView
                snapshot={snapshot}
                dictionaryProgress={dictionaryProgress}
                dictionaryEventsError={dictionaryEventsError}
                onDictionaryUpdate={handleDictionaryUpdate}
                onSaved={handleSettingsSaved}
              />
            </div>
          </div>
        </AnimatedOverlay>
      )}
      {closeDialogMounted && (
        <CloseBehaviorDialog
          open={closeDialogOpen}
          onResolved={requestCloseDialog}
          onClosed={handleCloseDialogClosed}
        />
      )}
    </div>
  );
}

function CloseBehaviorDialog({
  open,
  onResolved,
  onClosed,
}: {
  open: boolean;
  onResolved: () => void;
  onClosed: () => void;
}) {
  const [remember, setRemember] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const dialogRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const frame = window.requestAnimationFrame(() => dialogRef.current?.focus());
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      onResolved();
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      window.cancelAnimationFrame(frame);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [onResolved, open]);

  const resolve = async (action: Exclude<CloseBehavior, "ask">) => {
    setError(null);
    try {
      await invokeCommand("resolve_window_close", { action, remember });
      onResolved();
    } catch (reason) {
      setError(describeError(reason, "关闭窗口失败"));
    }
  };
  return (
    <AnimatedOverlay
      className="modal-backdrop"
      open={open}
      onClosed={onClosed}
      onBackdropClick={(event) => {
        if (event.target === event.currentTarget) onResolved();
      }}
    >
      <div className="modal-card" ref={dialogRef} role="dialog" aria-modal="true" aria-labelledby="close-dialog-title" tabIndex={-1}>
        <div className="modal-heading"><div><strong id="close-dialog-title">关闭 Lilt</strong><span>选择本次关闭窗口的处理方式。</span></div><button className="icon-button" type="button" onClick={onResolved} aria-label="取消"><X size={17} /></button></div>
        <label className="modal-check"><input type="checkbox" checked={remember} onChange={(event) => setRemember(event.target.checked)} />记住我的选择</label>
        {error && <p className="error-message settings-message">{error}</p>}
        <div className="form-actions modal-actions"><button className="secondary-button" type="button" onClick={() => void resolve("tray")}>缩小到托盘</button><button className="primary-button" type="button" onClick={() => void resolve("exit")}>退出程序</button></div>
      </div>
    </AnimatedOverlay>
  );
}

function ModeSwitcher({ activeTab, onChange }: { activeTab: AppTab; onChange: (tab: AppTab) => void }) {
  const switcherRef = useRef<HTMLElement | null>(null);
  const buttonRefs = useRef<Partial<Record<AppTab, HTMLButtonElement | null>>>({});
  const [indicator, setIndicator] = useState({ left: 0, width: 0, ready: false });

  const updateIndicator = useCallback(() => {
    const switcher = switcherRef.current;
    const button = buttonRefs.current[activeTab];
    if (!switcher || !button) return;
    const switcherRect = switcher.getBoundingClientRect();
    const buttonRect = button.getBoundingClientRect();
    setIndicator({
      left: buttonRect.left - switcherRect.left,
      width: buttonRect.width,
      ready: true,
    });
  }, [activeTab]);

  useLayoutEffect(() => {
    updateIndicator();
  }, [updateIndicator]);

  useEffect(() => {
    const switcher = switcherRef.current;
    if (!switcher) return;
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(updateIndicator);
    observer?.observe(switcher);
    Object.values(buttonRefs.current).forEach((button) => {
      if (button) observer?.observe(button);
    });
    window.addEventListener("resize", updateIndicator);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", updateIndicator);
    };
  }, [updateIndicator]);

  const setButtonRef = (tab: AppTab) => (button: HTMLButtonElement | null) => {
    buttonRefs.current[tab] = button;
  };

  return (
    <nav ref={switcherRef} className="mode-switcher" aria-label="工作模式" data-no-drag>
      <span
        className="mode-switcher-indicator"
        aria-hidden="true"
        style={{
          width: `${indicator.width}px`,
          transform: `translateX(${indicator.left}px)`,
          opacity: indicator.ready ? 1 : 0,
        }}
      />
      <ModeButton buttonRef={setButtonRef("translate")} icon={<Languages size={15} />} label="段落翻译" active={activeTab === "translate"} onClick={() => onChange("translate")} />
      <ModeButton buttonRef={setButtonRef("dictionary")} icon={<BookOpen size={15} />} label="词典" active={activeTab === "dictionary"} onClick={() => onChange("dictionary")} />
      <ModeButton buttonRef={setButtonRef("glossary")} icon={<FileText size={15} />} label="术语表" active={activeTab === "glossary"} onClick={() => onChange("glossary")} />
    </nav>
  );
}

function ModeButton({ icon, label, active, onClick, buttonRef }: { icon: React.ReactNode; label: string; active: boolean; onClick: () => void; buttonRef?: (button: HTMLButtonElement | null) => void }) {
  return (
    <button ref={buttonRef} className={`nav-item ${active ? "is-active" : ""}`} onClick={onClick} type="button" aria-pressed={active} title={label}>
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
  translationSummary: TranslationSummary | null;
  eventsReady: boolean;
  onSourceTextChange: (value: string) => void;
  onSourceLanguageChange: (value: string) => void;
  onTargetLanguageChange: (value: string) => void;
  onTranslate: () => void;
  onCancel: () => void;
  onCopy: () => void;
  history: HistoryEntry[];
  onOpenHistory: () => void;
}

function LanguageSelect({
  id,
  ariaLabel,
  value,
  onChange,
}: {
  id: string;
  ariaLabel: string;
  value: string;
  onChange: (value: string) => void;
}) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const buttonRef = useRef<HTMLButtonElement | null>(null);
  const optionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const [open, setOpen] = useState(false);
  const selectedIndex = Math.max(0, LANGUAGE_OPTIONS.findIndex(([, code]) => code === value));
  const [highlightedIndex, setHighlightedIndex] = useState(selectedIndex);

  useEffect(() => {
    if (!open) return;
    setHighlightedIndex(selectedIndex);
    const frame = window.requestAnimationFrame(() => optionRefs.current[selectedIndex]?.focus());
    const handlePointerDown = (event: PointerEvent) => {
      if (event.target instanceof Node && containerRef.current?.contains(event.target)) return;
      setOpen(false);
      buttonRef.current?.focus();
    };
    document.addEventListener("pointerdown", handlePointerDown);
    return () => {
      window.cancelAnimationFrame(frame);
      document.removeEventListener("pointerdown", handlePointerDown);
    };
  }, [open, selectedIndex]);

  const focusOption = (index: number) => {
    setHighlightedIndex(index);
    window.requestAnimationFrame(() => optionRefs.current[index]?.focus());
  };

  const selectValue = (nextValue: string) => {
    onChange(nextValue);
    setOpen(false);
    buttonRef.current?.focus();
  };

  const handleButtonKeyDown = (event: ReactKeyboardEvent<HTMLButtonElement>) => {
    if (event.key === "Escape" && open) {
      event.preventDefault();
      setOpen(false);
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const direction = event.key === "ArrowDown" ? 1 : -1;
      const nextIndex = open
        ? (highlightedIndex + direction + LANGUAGE_OPTIONS.length) % LANGUAGE_OPTIONS.length
        : selectedIndex;
      setOpen(true);
      focusOption(nextIndex);
      return;
    }
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      setOpen((current) => !current);
    }
  };

  const handleOptionKeyDown = (event: ReactKeyboardEvent<HTMLButtonElement>, index: number, optionValue: string) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const direction = event.key === "ArrowDown" ? 1 : -1;
      focusOption((index + direction + LANGUAGE_OPTIONS.length) % LANGUAGE_OPTIONS.length);
      return;
    }
    if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      focusOption(event.key === "Home" ? 0 : LANGUAGE_OPTIONS.length - 1);
      return;
    }
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      selectValue(optionValue);
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      setOpen(false);
      buttonRef.current?.focus();
      return;
    }
    if (event.key === "Tab") setOpen(false);
  };

  return (
    <div className="translation-language-control" ref={containerRef}>
      <button
        className="translation-language-button"
        ref={buttonRef}
        type="button"
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={`${id}-menu`}
        onClick={() => setOpen((current) => !current)}
        onKeyDown={handleButtonKeyDown}
      >
        <span>{languageLabel(value)}</span>
        <ChevronDown size={13} strokeWidth={1.8} aria-hidden="true" />
      </button>
      {open && (
        <div className="translation-language-menu" id={`${id}-menu`} role="listbox" aria-label={ariaLabel}>
          {LANGUAGE_OPTIONS.map(([label, optionValue], index) => (
            <button
              className={`translation-language-option ${index === selectedIndex ? "is-selected" : ""} ${index === highlightedIndex ? "is-highlighted" : ""}`}
              key={optionValue}
              ref={(element) => { optionRefs.current[index] = element; }}
              type="button"
              role="option"
              aria-selected={index === selectedIndex}
              onMouseEnter={() => setHighlightedIndex(index)}
              onClick={() => selectValue(optionValue)}
              onKeyDown={(event) => handleOptionKeyDown(event, index, optionValue)}
            >
              <span>{label}</span>
              {index === selectedIndex && <Check size={14} strokeWidth={2} aria-hidden="true" />}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function TranslateView(props: TranslateViewProps) {
  const isBusy = props.status === "streaming" || props.status === "cancelling";
  return (
    <section className="page-section translate-page">
      <div className="page-heading">
        <div className="page-title-block">
          <p className="eyebrow">TRANSLATE</p>
          <div className="page-title-line">
            <h1>段落翻译</h1>
            <div className="page-title-meta" aria-label="当前翻译模型">
              <span>模型 {props.selectedModel || "未配置模型"}</span>
            </div>
          </div>
        </div>
      </div>

      <div className="translation-grid">
        <div className="translation-column">
          <div className="translation-language">
            <span>原文</span>
            <LanguageSelect id="source-language" ariaLabel="原文语言" value={props.sourceLanguage} onChange={props.onSourceLanguageChange} />
          </div>
          <div className="translation-panel">
            <div className="translation-scroll-region">
              <textarea
                value={props.sourceText}
                onChange={(event) => props.onSourceTextChange(event.target.value)}
                placeholder="粘贴需要翻译的英文段落……"
                spellCheck={false}
              />
            </div>
            <div className="panel-footer"><span>{props.sourceText.length} 字符</span></div>
          </div>
        </div>

        <div className="translation-column">
          <div className="translation-language">
            <span>译文</span>
            <LanguageSelect id="target-language" ariaLabel="译文语言" value={props.targetLanguage} onChange={props.onTargetLanguageChange} />
          </div>
          <div className="translation-panel result-panel">
            <div className="translation-scroll-region">
              <div className={`result-content ${props.translatedText ? "has-content" : ""}`}>
                {props.translatedText || <span className="empty-result">译文会显示在这里</span>}
                {props.status === "streaming" && <span className="stream-caret" />}
              </div>
            </div>
            <div className="panel-footer result-footer">
              <span>{props.status === "completed" && props.translationSummary ? formatTranslationSummary(props.translationSummary) : ""}</span>
              <div className="result-footer-actions">
                <button className="icon-button" title="翻译历史" aria-label="打开翻译历史" onClick={props.onOpenHistory} type="button"><History size={16} /></button>
                <button className="icon-button" title="复制译文" aria-label="复制译文" onClick={props.onCopy} disabled={!props.translatedText} type="button"><Copy size={16} /></button>
              </div>
            </div>
          </div>
        </div>
      </div>

      <div className="action-row">
        <div className="message-area">
          <FeedbackMessage
            message={props.error ?? props.notice}
            kind={props.error ? "error" : "notice"}
          />
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

    </section>
  );
}

function GlossaryView({ terms, onChanged }: { terms: GlossaryTerm[]; onChanged: () => void }) {
  const [source, setSource] = useState("");
  const [target, setTarget] = useState("");
  const [note, setNote] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [visibleCount, setVisibleCount] = useState(10);
  const visibleTerms = terms.slice(0, visibleCount);
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
    <section className="page-section narrow-page bounded-list-page glossary-page">
      <PageTitle eyebrow="GLOSSARY" title="术语表" />
      <div className="simple-card">
        <div className="form-grid glossary-form">
          <label>原文<input value={source} onChange={(event) => setSource(event.target.value)} /></label>
          <label>译文<input value={target} onChange={(event) => setTarget(event.target.value)} /></label>
          <label className="wide-field">备注<input value={note} onChange={(event) => setNote(event.target.value)} placeholder="可选" /></label>
        </div>
        <div className="form-actions"><span className="error-message">{error}</span><button className="secondary-button" type="button" onClick={() => void addTerm()}>添加术语</button></div>
      </div>
      <div className="list-card bounded-list-card glossary-terms-card">
        <div className="list-card-heading"><strong>已添加术语</strong><span>{terms.length} 条</span></div>
        <div className="bounded-list-card-scroll">
          {terms.length === 0 ? <div className="empty-list">还没有术语。</div> : (
            <>
              {visibleTerms.map((term) => <GlossaryRow key={term.id} term={term} onChanged={onChanged} />)}
              {visibleTerms.length < terms.length && <button className="list-load-more" type="button" onClick={() => setVisibleCount((current) => Math.min(current + 10, terms.length))}>更多</button>}
            </>
          )}
        </div>
      </div>
    </section>
  );
}

function GlossaryRow({ term, onChanged }: { term: GlossaryTerm; onChanged: () => void }) {
  const remove = async () => {
    await invokeCommand("delete_glossary_term", { id: term.id });
    onChanged();
  };
  return <div className="list-row"><div><strong>{term.source}</strong><span className="arrow">→</span><span>{term.target}</span>{term.note && <small>{term.note}</small>}</div><button className="icon-button danger-icon-button" type="button" onClick={() => void remove()} title="删除术语" aria-label="删除术语"><Trash2 size={14} /></button></div>;
}

function HistoryContent({ history }: { history: HistoryEntry[] }) {
  return (
    <div className="history-dialog-content">
      <div className="list-card history-card">
        {history.length === 0 ? <div className="empty-list">完成一次段落翻译后，记录会出现在这里。</div> : history.map((item) => <HistoryRow key={item.id} item={item} />)}
      </div>
    </div>
  );
}

function HistoryRow({ item }: { item: HistoryEntry }) {
  return <article className="history-row"><div className="history-meta"><span>{formatDate(item.createdAt)}</span><span>{item.modelId}</span>{item.cacheHit && <span className="tag">缓存命中</span>}</div><p className="history-source">{item.sourceText}</p><p className="history-result">{item.translatedText}</p></article>;
}

function PromptManager({
  prompts,
  currentPromptId,
  onChanged,
}: {
  prompts: Prompt[];
  currentPromptId: string;
  onChanged: () => Promise<void>;
}) {
  const [selectedId, setSelectedId] = useState<string | null>(currentPromptId || null);
  const [name, setName] = useState("");
  const [content, setContent] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const selectedPrompt = prompts.find((prompt) => prompt.id === selectedId) ?? null;

  useEffect(() => {
    if (selectedId === null) return;
    const selected = prompts.find((prompt) => prompt.id === selectedId);
    if (!selected) return;
    setName(selected.name);
    setContent(selected.content);
  }, [currentPromptId, prompts, selectedId]);

  const selectPrompt = (prompt: Prompt) => {
    setSelectedId(prompt.id);
    setName(prompt.name);
    setContent(prompt.content);
    setError(null);
    setMessage(null);
  };

  const startNew = () => {
    setSelectedId(null);
    setName("我的 Prompt");
    setContent("");
    setError(null);
    setMessage(null);
  };

  const save = async () => {
    if (!name.trim() || !content.trim()) {
      setError("Prompt 名称和内容不能为空。");
      setMessage(null);
      return;
    }
    setError(null);
    setMessage(null);
    try {
      const raw = selectedId
        ? await invokeCommand<unknown>("update_prompt", { id: selectedId, name, content })
        : await invokeCommand<unknown>("create_prompt", { name, content });
      const next = decodePrompt(raw);
      if (!next) throw new Error("Prompt 命令返回了无法识别的结果。");
      setSelectedId(next.id);
      setName(next.name);
      setContent(next.content);
      await onChanged();
      setMessage(selectedId ? "Prompt 已更新" : "Prompt 已创建");
    } catch (reason) {
      setError(describeError(reason, "Prompt 保存失败"));
    }
  };

  const duplicate = async (prompt: Prompt) => {
    setError(null);
    setMessage(null);
    try {
      const raw = await invokeCommand<unknown>("duplicate_prompt", { id: prompt.id });
      const next = decodePrompt(raw);
      if (!next) throw new Error("Prompt 命令返回了无法识别的结果。");
      setSelectedId(next.id);
      setName(next.name);
      setContent(next.content);
      await onChanged();
      setMessage("已复制 Prompt，可以继续编辑");
    } catch (reason) {
      setError(describeError(reason, "复制 Prompt 失败"));
    }
  };

  const setDefault = async (prompt: Prompt) => {
    try {
      await invokeCommand("set_default_prompt", { id: prompt.id });
      await onChanged();
      setMessage(`已将「${prompt.name}」设为默认 Prompt`);
      setError(null);
    } catch (reason) {
      setError(describeError(reason, "设置默认 Prompt 失败"));
      setMessage(null);
    }
  };

  const remove = async (prompt: Prompt) => {
    if (!window.confirm(`确定删除 Prompt「${prompt.name}」吗？`)) return;
    try {
      await invokeCommand("delete_prompt", { id: prompt.id });
      await onChanged();
      setSelectedId(currentPromptId);
      setMessage("Prompt 已删除");
      setError(null);
    } catch (reason) {
      setError(describeError(reason, "删除 Prompt 失败"));
      setMessage(null);
    }
  };

  return (
    <div className="simple-card prompt-manager-card">
      <div className="card-heading"><div><strong>Prompt</strong><span>内置 Prompt 只读；复制后可以编辑并设为默认。</span></div><button className="secondary-button small-button" type="button" onClick={startNew}>新建</button></div>
      <div className="prompt-manager-grid">
        <div className="prompt-list">
          {prompts.map((prompt) => (
            <button className={`prompt-list-item ${prompt.id === selectedId ? "is-active" : ""}`} type="button" key={prompt.id} onClick={() => selectPrompt(prompt)}>
              <span><strong>{prompt.name}</strong><small>{prompt.isBuiltin ? "内置" : "自定义"} · v{prompt.version}</small></span>
              {prompt.id === currentPromptId && <em>默认</em>}
            </button>
          ))}
        </div>
        <div className="prompt-editor-panel">
          <label>名称<input value={name} onChange={(event) => setName(event.target.value)} disabled={selectedPrompt?.isBuiltin ?? false} /></label>
          <label>内容<textarea className="prompt-editor" value={content} onChange={(event) => setContent(event.target.value)} readOnly={selectedPrompt?.isBuiltin ?? false} /></label>
          <div className="form-actions prompt-actions">
            <div className="button-group">
              {selectedPrompt?.isBuiltin ? <button className="secondary-button" type="button" onClick={() => void duplicate(selectedPrompt)}>复制并编辑</button> : <button className="primary-button small-button" type="button" onClick={() => void save()}>保存 Prompt</button>}
              {selectedPrompt && selectedPrompt.id !== currentPromptId && <button className="secondary-button" type="button" onClick={() => void setDefault(selectedPrompt)}>设为默认</button>}
              {selectedPrompt && !selectedPrompt.isBuiltin && selectedPrompt.id !== currentPromptId && <button className="text-button danger-text" type="button" onClick={() => void remove(selectedPrompt)}>删除</button>}
            </div>
          </div>
        </div>
      </div>
      {message && <p className="notice-message settings-message">{message}</p>}
      {error && <p className="error-message settings-message">{error}</p>}
    </div>
  );
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
  const [thinkingEffort, setThinkingEffort] = useState<ThinkingEffort>(snapshot.provider.thinkingEffort ?? "none");
  const [availableModels, setAvailableModels] = useState<ModelInfo[] | null>(null);
  const [apiKey, setApiKey] = useState("");
  const [settings, setSettings] = useState<AppSettings>(snapshot.settings);
  const [selectionMode, setSelectionMode] = useState(snapshot.settings.selectionMode);
  const [selectionShortcut, setSelectionShortcut] = useState(snapshot.settings.selectionShortcut);
  const [selectionStatus, setSelectionStatus] = useState<SelectionRuntimeStatus | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [providerMessage, setProviderMessage] = useState<string | null>(null);
  const [providerError, setProviderError] = useState<string | null>(null);

  useEffect(() => {
    setBaseUrl(snapshot.provider.baseUrl);
    setModelId(snapshot.provider.modelId);
    setThinkingEffort(snapshot.provider.thinkingEffort ?? "none");
    setAvailableModels(null);
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
    setProviderError(null);
    setProviderMessage(null);
    try {
      await invokeCommand("save_provider_config", {
        baseUrl,
        modelId,
        thinkingEffort,
        apiKey: apiKey || null,
      });
      setApiKey("");
      const next = await invokeCommand<AppSnapshot>("get_app_snapshot");
      onSaved(next);
      setProviderMessage("Provider 设置已保存");
    } catch (reason) {
      setProviderError(describeError(reason, "Provider 设置保存失败"));
      setProviderMessage(null);
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
    setProviderError(null);
    setProviderMessage(null);
    try {
      const models = await invokeCommand<ModelInfo[]>("fetch_models", {
        baseUrl: baseUrl.trim() || null,
        apiKey: apiKey.trim() || null,
      });
      setAvailableModels(models.length > 0 ? models : null);
      if (models.length > 0 && !models.some((model) => model.id === modelId)) {
        setModelId(models[0]?.id ?? modelId);
      }
      setProviderMessage(`模型列表已更新，共 ${models.length} 个模型`);
    } catch (reason) {
      setAvailableModels(null);
      setProviderError(describeError(reason, "模型列表读取失败，可手动填写 Model ID"));
      setProviderMessage(null);
    }
  };

  const refreshAfterPromptChange = async () => {
    const next = await invokeCommand<AppSnapshot>("get_app_snapshot");
    onSaved(next);
  };

  const resetCloseBehavior = async () => {
    try {
      await invokeCommand("reset_close_behavior");
      const next = await invokeCommand<AppSnapshot>("get_app_snapshot");
      onSaved(next);
      setMessage("关闭行为已恢复为每次询问");
      setError(null);
    } catch (reason) {
      setError(describeError(reason, "恢复关闭行为失败"));
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
          <div className="card-heading"><div><strong>OpenAI-compatible Provider</strong><span>即将支持其他协议。</span></div><span className={`connection-status ${snapshot.provider.hasApiKey ? "connected" : ""}`}>{snapshot.provider.hasApiKey ? "已配置密钥" : "未配置密钥"}</span></div>
          <div className="form-grid">
            <label className="wide-field">Base URL<input value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} placeholder="https://api.openai.com/v1" /></label>
            <label>Model ID{availableModels ? <select value={modelId} onChange={(event) => setModelId(event.target.value)}>{!availableModels.some((model) => model.id === modelId) && <option value={modelId}>当前：{modelId}</option>}{availableModels.map((model) => <option key={model.id} value={model.id}>{model.label}</option>)}</select> : <input value={modelId} onChange={(event) => setModelId(event.target.value)} placeholder="gpt-4o-mini" />}</label>
            <label>思考强度<select value={thinkingEffort} onChange={(event) => setThinkingEffort(event.target.value as ThinkingEffort)}><option value="none">none</option><option value="low">low</option><option value="medium">medium</option><option value="high">high</option></select></label>
            <label className="wide-field">API Key<input type="password" value={apiKey} onChange={(event) => setApiKey(event.target.value)} placeholder={snapshot.provider.hasApiKey ? "已保存，留空表示不修改" : "保存在 Windows 凭据管理器"} autoComplete="off" /></label>
          </div>
          {providerMessage && <p className="notice-message settings-message">{providerMessage}</p>}
          {providerError && <p className="error-message settings-message">{providerError}</p>}
          <div className="form-actions"><span className="muted-text">模型列表读取失败时，Model ID 仍可手动填写。</span><div className="button-group"><button className="secondary-button" type="button" onClick={() => void fetchModels()}>读取模型</button><button className="primary-button small-button" type="button" onClick={() => void saveProvider()}>保存 Provider</button></div></div>
        </div>

        <PromptManager prompts={snapshot.prompts} currentPromptId={snapshot.provider.promptId} onChanged={refreshAfterPromptChange} />

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

        <div className="simple-card">
          <div className="card-heading"><div><strong>关闭行为</strong><span>点击主窗口关闭按钮时的处理方式。</span></div></div>
          <div className="setting-line"><span><strong>{settings.closeBehavior === "ask" ? "每次询问" : settings.closeBehavior === "tray" ? "缩小到系统托盘" : "退出程序"}</strong><small>{settings.closeBehavior === "ask" ? "关闭窗口时显示选择对话框。" : "已经记住选择，可在这里恢复询问。"}</small></span><button className="secondary-button" type="button" onClick={() => void resetCloseBehavior()} disabled={settings.closeBehavior === "ask"}>恢复每次询问</button></div>
        </div>
      </div>
      {message && <p className="notice-message settings-message">{message}</p>}
      {error && <p className="error-message settings-message">{error}</p>}
    </section>
  );
}

function PageTitle({ eyebrow, title, description }: { eyebrow: string; title: string; description?: string }) {
  return <div className="page-heading"><div><p className="eyebrow">{eyebrow}</p><h1>{title}</h1>{description && <p className="page-description">{description}</p>}</div></div>;
}

export default App;
