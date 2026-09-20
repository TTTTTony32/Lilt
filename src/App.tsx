import { useCallback, useEffect, useLayoutEffect, useMemo, useReducer, useRef, useState, type AnimationEvent as ReactAnimationEvent, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent as ReactMouseEvent, type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open, save } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";
import { ArrowLeft, Check, ChevronDown, Copy, Database, ExternalLink, FileText, FileType2, History, Info, Languages, LoaderCircle, Settings, Square, WandSparkles, BookOpen, Upload, X, Maximize2, Minimize2, Minus, Trash2, Download } from "lucide-react";
import packageJson from "../package.json";
import liltLogo from "../source/lilt_logo.svg";
import { describeError } from "./lib/errors";
import { invokeCommand, listenTo } from "./lib/tauri";
import DictionaryView, { type DictionaryProgress, type WordExampleRequestInput } from "./DictionaryView";
import type { DictionaryOpenRequest } from "./DictionaryView";
import PdfView, { type PdfTranslationFloatingState } from "./PdfView";
import { PdfEnginePanel } from "./PdfEnginePanel";
import PersonalDictionaryView from "./PersonalDictionaryView";
import { AnimatedOverlay } from "./components/AnimatedOverlay";
import { usePrefersReducedMotion } from "./components/usePrefersReducedMotion";
import { DownloadActivityStack } from "./components/DownloadActivityStack";
import { ResourceDownloadDialog, type ResourceDownloadDialogStatus } from "./components/ResourceDownloadDialog";
import SegmentedSourceEditor from "./components/SegmentedSourceEditor";
import { usePdfEngineRuntime, type PdfEngineRuntime } from "./lib/usePdfEngineRuntime";
import { summarizeUpdaterUpdate, type GitHubReleaseSummary } from "./lib/release-version";
import { formatSelectionShortcut } from "./lib/selection-shortcut";
import {
  downloadActivityKey,
  downloadActivityReducer,
  initialDownloadActivityState,
  listDownloadActivities,
  normalizeStagePercent,
  selectDownloadActivity,
  type DownloadActivity,
  type DownloadResource,
  type ResourceDownloadPromptRequest,
} from "./lib/download-activity";
import {
  DEFAULT_SNAPSHOT,
  type AppSettings,
  type AppSnapshot,
  type AppTab,
  type CloseBehavior,
  type GlossaryExportResult,
  type GlossaryImportResult,
  type GlossaryTerm,
  type HistoryEntry,
  type ModelInfo,
  type PersonalDictionaryEntry,
  type PersonalDictionaryExportResult,
  type Prompt,
  type ThinkingEffort,
  type ParagraphLearningResult,
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
  decodeParagraphLearningResult,
  decodeWordExampleCommandResult,
  decodeWordExampleEvent,
  decodePrompt,
  decodeGlossaryExportResult,
  decodeGlossaryImportResult,
  decodePersonalDictionaryExportResult,
  decodePdfEngineEvent,
  PDF_ENGINE_EVENT_NAMES,
  type PdfEngineEvent,
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
const SOURCE_LANGUAGE_OPTIONS = [["自动检测", "auto"], ...LANGUAGE_OPTIONS] as const;

const APP_VERSION = packageJson.version;
const MIN_CACHE_SIZE_MB = 16;
const MAX_CACHE_SIZE_MB = 512;
const GITHUB_URL = "https://github.com/TTTTTony32/Lilt";
const DEVELOPER_EMAIL = "imtony32@gmail.com";

function openExternalUrl(url: string) {
  void openUrl(url).catch(() => {
    window.open(url, "_blank", "noopener,noreferrer");
  });
}

interface TranslationSummary {
  durationMs: number;
  cacheHit: boolean;
}

type TranslationRequestMode = "plain" | "learning";

const SETTINGS_SECTIONS = [
  { id: "provider", label: "LLM提供商", icon: Settings },
  { id: "prompt", label: "提示词", icon: FileText },
  { id: "selection", label: "划词翻译", icon: Languages },
  { id: "pdf", label: "PDF 全文翻译", icon: FileType2 },
  { id: "local", label: "本地数据", icon: Database },
  { id: "behavior", label: "关闭行为", icon: X },
  { id: "about", label: "关于", icon: Info },
] as const;

type SelectionModeOption = Exclude<AppSettings["selectionMode"], "none" | "both">;

const SELECTION_MODE_OPTIONS: ReadonlyArray<{
  value: SelectionModeOption;
  label: string;
}> = [
  { value: "shortcut", label: "按快捷键" },
  { value: "automatic", label: "自动监听选区" },
];

function isSelectionModeEnabled(mode: AppSettings["selectionMode"], option: SelectionModeOption): boolean {
  return mode === option || mode === "both";
}

function toggleSelectionMode(mode: AppSettings["selectionMode"], option: SelectionModeOption): AppSettings["selectionMode"] {
  if (mode === "both") return option === "shortcut" ? "automatic" : "shortcut";
  if (mode === option) return "none";
  return mode === "none" ? option : "both";
}

type SettingsSectionId = (typeof SETTINGS_SECTIONS)[number]["id"];
type SettingsResourceModal = "dictionary" | "pdf-engine";
type SettingsNavigationTarget = {
  sectionId: SettingsSectionId;
  anchor?: "dictionary-version" | "pdf-engine";
  modal?: SettingsResourceModal;
};
const SETTINGS_CHROME_HEIGHT = 76;

interface SettingsDraft {
  baseUrl: string;
  modelId: string;
  thinkingEffort: ThinkingEffort;
  apiKey: string;
  settings: AppSettings;
  selectionMode: AppSettings["selectionMode"];
  selectionShortcut: string;
}

function createSettingsDraft(snapshot: AppSnapshot): SettingsDraft {
  return {
    baseUrl: snapshot.provider.baseUrl,
    modelId: snapshot.provider.modelId,
    thinkingEffort: snapshot.provider.thinkingEffort ?? "none",
    apiKey: "",
    settings: snapshot.settings,
    selectionMode: snapshot.settings.selectionMode,
    selectionShortcut: snapshot.settings.selectionShortcut,
  };
}

function formatTranslationSummary(summary: TranslationSummary): string {
  return `${(summary.durationMs / 1000).toFixed(2)}秒·${summary.cacheHit ? "缓存命中" : "未命中缓存"}`;
}

function languageLabel(value: string, options: readonly (readonly [string, string])[] = LANGUAGE_OPTIONS): string {
  return options.find(([, code]) => code === value)?.[0] ?? value;
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

type DataTransferMode = "personalExport" | "glossaryExport" | "glossaryImport";
type DataTransferStatus = "selecting" | "processing" | "success" | "empty" | "cancelled" | "error";
type ToastKind = "error" | "notice";
type ReleaseUpdateStatus = "ready" | "downloading" | "installing" | "failed";
type AppToast = { message: string; kind: ToastKind };
type ShowToast = (message: string, kind?: ToastKind) => void;

function FeedbackMessage({
  message,
  kind,
  as = "span",
  className = "",
}: {
  message: string | null;
  kind: "error" | "notice";
  as?: "span" | "p" | "div";
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
      role={kind === "error" ? "alert" : "status"}
      aria-live={kind === "error" ? "assertive" : "polite"}
      onAnimationEnd={handleAnimationEnd}
    >
      {visible.message}
    </Element>
  );
}

function App() {
  const reducedMotion = usePrefersReducedMotion();
  const [snapshot, setSnapshot] = useState<AppSnapshot>(DEFAULT_SNAPSHOT);
  const [snapshotReady, setSnapshotReady] = useState(false);
  const [tab, setTab] = useState<AppTab>("translate");
  const [sourceText, setSourceText] = useState("");
  const [translatedText, setTranslatedText] = useState("");
  const [sourceLanguage, setSourceLanguage] = useState("auto");
  const [targetLanguage, setTargetLanguage] = useState("zh-CN");
  const [status, setStatus] = useState<TranslationStatus>("idle");
  const [activeRequestMode, setActiveRequestMode] = useState<TranslationRequestMode | null>(null);
  const [learningResult, setLearningResult] = useState<ParagraphLearningResult | null>(null);
  const [learningModeSaving, setLearningModeSaving] = useState(false);
  const [pdfPreflightSaving, setPdfPreflightSaving] = useState(false);
  const pdfPreflightSaveSequenceRef = useRef(0);
  const pdfPreflightSaveActiveRef = useRef<number | null>(null);
  const [error, setError] = useState<string | null>(null);
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
  const [settingsWorkspaceMounted, setSettingsWorkspaceMounted] = useState(false);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [historyOverlayMounted, setHistoryOverlayMounted] = useState(false);
  const [dataTransferOpen, setDataTransferOpen] = useState(false);
  const [dataTransferOverlayMounted, setDataTransferOverlayMounted] = useState(false);
  const [dataTransferMode, setDataTransferMode] = useState<DataTransferMode>("personalExport");
  const [releaseNotice, setReleaseNotice] = useState<GitHubReleaseSummary | null>(null);
  const [releaseNoticeOpen, setReleaseNoticeOpen] = useState(false);
  const [releaseNoticeMounted, setReleaseNoticeMounted] = useState(false);
  const [releaseCheckMessage, setReleaseCheckMessage] = useState<string | null>(null);
  const [releaseCheckPending, setReleaseCheckPending] = useState(false);
  const [releaseUpdateStatus, setReleaseUpdateStatus] = useState<ReleaseUpdateStatus>("ready");
  const [releaseDownloadBytes, setReleaseDownloadBytes] = useState(0);
  const [releaseDownloadTotal, setReleaseDownloadTotal] = useState<number | null>(null);
  const [releaseUpdateError, setReleaseUpdateError] = useState<string | null>(null);
  const [appToast, setAppToast] = useState<AppToast | null>(null);
  const [mainWindowMaximized, setMainWindowMaximized] = useState(false);
  const [downloadActivityState, dispatchDownloadActivity] = useReducer(
    downloadActivityReducer,
    initialDownloadActivityState,
  );
  const [pdfTranslationFloating, setPdfTranslationFloating] = useState<PdfTranslationFloatingState | null>(null);
  const [resourceDownloadPrompt, setResourceDownloadPrompt] = useState<ResourceDownloadPromptRequest | null>(null);
  const [resourceDownloadDialogOpen, setResourceDownloadDialogOpen] = useState(false);
  const [resourceDownloadDialogMounted, setResourceDownloadDialogMounted] = useState(false);
  const [resourceDownloadTerminalState, setResourceDownloadTerminalState] = useState<Partial<Record<DownloadActivity["resource"], ResourceDownloadDialogStatus>>>({});
  const [settingsNavigationTarget, setSettingsNavigationTarget] = useState<SettingsNavigationTarget | null>(null);
  const pdfEngine = usePdfEngineRuntime();
  const activeRequestId = useRef<string | null>(null);
  const activeRequestModeRef = useRef<TranslationRequestMode | null>(null);
  const activeRequestSourceTextRef = useRef<string | null>(null);
  const activeRequestRawSourceTextRef = useRef<string | null>(null);
  const sourceTextRef = useRef(sourceText);
  const translationStartedAt = useRef<number | null>(null);
  const activeDictionaryOperationId = useRef<string | null>(null);
  const activePdfEngineOperationId = useRef<string | null>(null);
  const activeWordExampleRequestId = useRef<string | null>(null);
  const settingsWorkspaceRef = useRef<HTMLDivElement | null>(null);
  const settingsOpenRef = useRef(settingsOpen);
  const settingsReturnFocusRef = useRef<HTMLElement | null>(null);
  const settingsReturnFocusIdRef = useRef<string | null>(null);
  const settingsToggleRef = useRef<HTMLButtonElement | null>(null);
  const historyDialogRef = useRef<HTMLDivElement | null>(null);
  const historyReturnFocusRef = useRef<HTMLElement | null>(null);
  const dataTransferReturnFocusRef = useRef<HTMLElement | null>(null);
  const closeDialogReturnFocusRef = useRef<HTMLElement | null>(null);
  const downloadActivityTimersRef = useRef(new Map<string, number>());
  const pendingUpdateRef = useRef<Update | null>(null);
  const releaseCheckGenerationRef = useRef(0);
  const releaseCheckPendingRef = useRef(false);
  const releaseInstallInProgressRef = useRef(false);
  const releaseCheckToastTimerRef = useRef<number | null>(null);
  const appToastTimerRef = useRef<number | null>(null);

  const showReleaseCheckMessage = useCallback((message: string) => {
    if (releaseCheckToastTimerRef.current !== null) {
      window.clearTimeout(releaseCheckToastTimerRef.current);
    }
    setReleaseCheckMessage(message);
    releaseCheckToastTimerRef.current = window.setTimeout(() => {
      releaseCheckToastTimerRef.current = null;
      setReleaseCheckMessage(null);
    }, 2800);
  }, []);

  const showAppToast = useCallback<ShowToast>((message, kind = "notice") => {
    if (appToastTimerRef.current !== null) {
      window.clearTimeout(appToastTimerRef.current);
    }
    setAppToast({ message, kind });
    appToastTimerRef.current = window.setTimeout(() => {
      appToastTimerRef.current = null;
      setAppToast(null);
    }, 2800);
  }, []);

  const showReleaseNotice = useCallback((latest: GitHubReleaseSummary) => {
    setReleaseNotice(latest);
    setReleaseNoticeMounted(true);
    setReleaseNoticeOpen(true);
  }, []);

  const checkForUpdates = useCallback(async (source: "startup" | "manual") => {
    if (releaseCheckPendingRef.current || releaseInstallInProgressRef.current) return;
    releaseCheckPendingRef.current = true;
    const generation = releaseCheckGenerationRef.current;

    if (source === "manual") setReleaseCheckPending(true);

    try {
      const update = await check({ timeout: 8000 });
      if (generation !== releaseCheckGenerationRef.current) {
        if (update) await update.close().catch(() => undefined);
        return;
      }
      if (!update) {
        if (source === "manual") showReleaseCheckMessage("当前已是最新版本");
        return;
      }

      const latest = summarizeUpdaterUpdate(update);
      if (!latest) {
        await update.close().catch(() => undefined);
        if (source === "manual") showReleaseCheckMessage("检查更新失败，请稍后重试");
        return;
      }

      const previousUpdate = pendingUpdateRef.current;
      pendingUpdateRef.current = update;
      if (previousUpdate && previousUpdate !== update) {
        void previousUpdate.close().catch(() => undefined);
      }
      setReleaseUpdateStatus("ready");
      setReleaseDownloadBytes(0);
      setReleaseDownloadTotal(null);
      setReleaseUpdateError(null);
      showReleaseNotice(latest);
    } catch {
      if (generation === releaseCheckGenerationRef.current && source === "manual") {
        showReleaseCheckMessage("检查更新失败，请稍后重试");
      }
    } finally {
      if (generation === releaseCheckGenerationRef.current) {
        releaseCheckPendingRef.current = false;
        if (source === "manual") setReleaseCheckPending(false);
      }
    }
  }, [showReleaseCheckMessage, showReleaseNotice]);

  useEffect(() => {
    sourceTextRef.current = sourceText;
  }, [sourceText]);

  useEffect(() => {
    void checkForUpdates("startup");
  }, [checkForUpdates]);

  const scheduleDownloadActivityRemoval = useCallback((resource: DownloadActivity["resource"], operationId: string, delayMs: number) => {
    const key = downloadActivityKey(resource, operationId);
    const previousTimer = downloadActivityTimersRef.current.get(key);
    if (previousTimer !== undefined) window.clearTimeout(previousTimer);
    const timer = window.setTimeout(() => {
      downloadActivityTimersRef.current.delete(key);
      dispatchDownloadActivity({ type: "removed", resource, operationId });
    }, delayMs);
    downloadActivityTimersRef.current.set(key, timer);
  }, []);

  const openResourceDownloadPrompt = useCallback((request: ResourceDownloadPromptRequest) => {
    setResourceDownloadPrompt(request);
    setResourceDownloadDialogMounted(true);
    setResourceDownloadDialogOpen(true);
  }, []);

  const requestResourceDownloadClose = useCallback(() => {
    setResourceDownloadDialogOpen(false);
  }, []);

  const handleResourceDownloadClosed = useCallback(() => {
    setResourceDownloadDialogMounted(false);
    setResourceDownloadPrompt(null);
  }, []);

  const clearResourceDownloadTerminal = useCallback((resource: DownloadActivity["resource"]) => {
    setResourceDownloadTerminalState((current) => {
      if (!(resource in current)) return current;
      const next = { ...current };
      delete next[resource];
      return next;
    });
  }, []);

  const markResourceDownloadTerminal = useCallback((resource: DownloadActivity["resource"], status: Extract<ResourceDownloadDialogStatus, "completed" | "failed">) => {
    setResourceDownloadTerminalState((current) => current[resource] === status ? current : { ...current, [resource]: status });
  }, []);

  const startResourceDownload = useCallback(() => {
    if (!resourceDownloadPrompt) return;
    clearResourceDownloadTerminal(resourceDownloadPrompt.resource);
    resourceDownloadPrompt.onStart();
  }, [clearResourceDownloadTerminal, resourceDownloadPrompt]);

  const requestReleaseNoticeClose = useCallback(() => {
    if (releaseUpdateStatus === "downloading" || releaseUpdateStatus === "installing") return;
    setReleaseNoticeOpen(false);
  }, [releaseUpdateStatus]);

  const handleReleaseNoticeClosed = useCallback(() => {
    const update = pendingUpdateRef.current;
    pendingUpdateRef.current = null;
    if (update) void update.close().catch(() => undefined);
    setReleaseNoticeMounted(false);
    setReleaseNotice(null);
    setReleaseUpdateStatus("ready");
    setReleaseDownloadBytes(0);
    setReleaseDownloadTotal(null);
    setReleaseUpdateError(null);
  }, []);

  const handleReleaseUpdate = useCallback(async () => {
    const update = pendingUpdateRef.current;
    if (!update || releaseInstallInProgressRef.current) return;

    releaseInstallInProgressRef.current = true;
    setReleaseUpdateStatus("downloading");
    setReleaseDownloadBytes(0);
    setReleaseDownloadTotal(null);
    setReleaseUpdateError(null);
    let downloadedBytes = 0;

    try {
      await update.download((event: DownloadEvent) => {
        if (event.event === "Started") {
          downloadedBytes = 0;
          setReleaseDownloadBytes(0);
          setReleaseDownloadTotal(typeof event.data.contentLength === "number" && event.data.contentLength > 0
            ? event.data.contentLength
            : null);
          return;
        }
        if (event.event === "Progress") {
          downloadedBytes += event.data.chunkLength;
          setReleaseDownloadBytes(downloadedBytes);
          return;
        }
      });
      setReleaseUpdateStatus("installing");
      await invokeCommand("prepare_for_updater_exit");
      await update.install({ restartAfterInstall: true });
    } catch (reason) {
      await invokeCommand("recover_after_updater_failure").catch(() => undefined);
      const message = describeError(reason, "更新下载或安装失败，请重试");
      setReleaseUpdateStatus("failed");
      setReleaseUpdateError(message);
      showAppToast(message, "error");
    } finally {
      releaseInstallInProgressRef.current = false;
    }
  }, [showAppToast]);

  useEffect(() => () => {
    releaseCheckGenerationRef.current += 1;
    releaseCheckPendingRef.current = false;
    const update = pendingUpdateRef.current;
    pendingUpdateRef.current = null;
    if (update) void update.close().catch(() => undefined);
  }, []);

  useEffect(() => () => {
    downloadActivityTimersRef.current.forEach((timer) => window.clearTimeout(timer));
    downloadActivityTimersRef.current.clear();
  }, []);

  useEffect(() => () => {
    if (releaseCheckToastTimerRef.current !== null) {
      window.clearTimeout(releaseCheckToastTimerRef.current);
      releaseCheckToastTimerRef.current = null;
    }
  }, []);

  useEffect(() => () => {
    if (appToastTimerRef.current !== null) {
      window.clearTimeout(appToastTimerRef.current);
      appToastTimerRef.current = null;
    }
  }, []);

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
    if (settingsOpen) return;
    const activeElement = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    settingsReturnFocusRef.current = activeElement;
    settingsReturnFocusIdRef.current = activeElement?.id || null;
    setSettingsNavigationTarget(null);
    setSettingsWorkspaceMounted(true);
    setSettingsOpen(true);
  }, [settingsOpen]);

  const requestSettingsClose = useCallback(() => {
    setSettingsNavigationTarget(null);
    setSettingsOpen(false);
  }, []);

  const openPdf = useCallback(() => {
    if (settingsOpen) requestSettingsClose();
    setTab("pdf");
  }, [requestSettingsClose, settingsOpen]);

  const openSettingsAt = useCallback((target: SettingsNavigationTarget) => {
    if (!settingsOpen) {
      const activeElement = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      settingsReturnFocusRef.current = activeElement;
      settingsReturnFocusIdRef.current = activeElement?.id || null;
      setSettingsWorkspaceMounted(true);
      setSettingsOpen(true);
    }
    setSettingsNavigationTarget(target);
  }, [settingsOpen]);

  const openPdfEngineSettings = useCallback(() => {
    setResourceDownloadDialogOpen(false);
    openSettingsAt({ sectionId: "about", anchor: "pdf-engine", modal: "pdf-engine" });
  }, [openSettingsAt]);

  const openDictionaryAbout = useCallback(() => {
    openSettingsAt({ sectionId: "about", anchor: "dictionary-version", modal: "dictionary" });
  }, [openSettingsAt]);

  const handleDownloadActivityClick = useCallback((resource: DownloadResource) => {
    if (resource === "pdf-engine") {
      openPdfEngineSettings();
      return;
    }
    openDictionaryAbout();
  }, [openDictionaryAbout, openPdfEngineSettings]);

  settingsOpenRef.current = settingsOpen;

  const finishSettingsClose = useCallback(() => {
    if (settingsOpenRef.current) return;
    setSettingsWorkspaceMounted(false);
    const previouslyFocused = settingsReturnFocusRef.current;
    settingsReturnFocusRef.current = null;
    const fallbackFocus = settingsReturnFocusIdRef.current
      ? document.getElementById(settingsReturnFocusIdRef.current)
      : null;
    settingsReturnFocusIdRef.current = null;
    (previouslyFocused?.isConnected ? previouslyFocused : fallbackFocus ?? settingsToggleRef.current)?.focus({ preventScroll: true });
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

  const openDataTransfer = useCallback((mode: DataTransferMode) => {
    dataTransferReturnFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setDataTransferMode(mode);
    setDataTransferOverlayMounted(true);
    setDataTransferOpen(true);
  }, []);

  const requestDataTransferClose = useCallback(() => {
    setDataTransferOpen(false);
  }, []);

  const handleDataTransferClosed = useCallback(() => {
    setDataTransferOverlayMounted(false);
    const previouslyFocused = dataTransferReturnFocusRef.current;
    dataTransferReturnFocusRef.current = null;
    previouslyFocused?.focus();
  }, []);

  useEffect(() => {
    if (settingsOpen) {
      const frame = window.requestAnimationFrame(() => settingsWorkspaceRef.current?.focus({ preventScroll: true }));
      const handleKeyDown = (event: KeyboardEvent) => {
        if (event.key !== "Escape" || event.defaultPrevented) return;
        if (event.target instanceof Element && event.target.closest('[role="dialog"]')) return;
        event.preventDefault();
        requestSettingsClose();
      };
      document.addEventListener("keydown", handleKeyDown);
      return () => {
        window.cancelAnimationFrame(frame);
        document.removeEventListener("keydown", handleKeyDown);
      };
    }
    return undefined;
  }, [requestSettingsClose, settingsOpen]);

  useEffect(() => {
    if (settingsOpen || !settingsWorkspaceMounted || !reducedMotion) return;
    finishSettingsClose();
  }, [finishSettingsClose, reducedMotion, settingsOpen, settingsWorkspaceMounted]);

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
    const pdfPreflightSaveAtRequestStart = pdfPreflightSaveActiveRef.current;
    try {
      const next = await invokeCommand<AppSnapshot>("get_app_snapshot");
      setSnapshot((current) => {
        const preserveLocalPdfPreflight = pdfPreflightSaveActiveRef.current !== null
          || pdfPreflightSaveAtRequestStart !== pdfPreflightSaveActiveRef.current;
        return preserveLocalPdfPreflight
          ? {
            ...next,
            settings: {
              ...next.settings,
              pdfPreflightEnabled: current.settings.pdfPreflightEnabled,
            },
          }
          : next;
      });
      setSnapshotReady(true);
    } catch (reason) {
      setError(describeError(reason, "无法读取应用配置"));
    }
  }, []);

  const handleSourceTextChange = useCallback((value: string) => {
    sourceTextRef.current = value;
    setSourceText(value);
    setLearningResult(null);
  }, []);

  const handleLearningModeChange = useCallback(async (enabled: boolean) => {
    if (learningModeSaving) return;
    const previous = snapshot.settings.paragraphLearningModeEnabled;
    if (previous === enabled) return;
    setLearningModeSaving(true);
    setSnapshot((current) => ({
      ...current,
      settings: { ...current.settings, paragraphLearningModeEnabled: enabled },
    }));
    setError(null);
    try {
      await invokeCommand("set_paragraph_learning_mode", { enabled });
      if (!enabled) setLearningResult(null);
    } catch (reason) {
      setSnapshot((current) => ({
        ...current,
        settings: { ...current.settings, paragraphLearningModeEnabled: previous },
      }));
      setError(describeError(reason, "学习模式设置保存失败"));
    } finally {
      setLearningModeSaving(false);
    }
  }, [learningModeSaving, snapshot.settings.paragraphLearningModeEnabled]);

  const handlePdfPreflightEnabledChange = useCallback(async (enabled: boolean) => {
    if (pdfPreflightSaving) return;
    const previous = snapshot.settings.pdfPreflightEnabled;
    if (previous === enabled) return;
    const saveId = pdfPreflightSaveSequenceRef.current + 1;
    pdfPreflightSaveSequenceRef.current = saveId;
    pdfPreflightSaveActiveRef.current = saveId;
    setPdfPreflightSaving(true);
    setSnapshot((current) => ({
      ...current,
      settings: { ...current.settings, pdfPreflightEnabled: enabled },
    }));
    setError(null);
    try {
      await invokeCommand("set_pdf_preflight_enabled", { enabled });
      if (pdfPreflightSaveActiveRef.current === saveId) {
        setSnapshot((current) => ({
          ...current,
          settings: { ...current.settings, pdfPreflightEnabled: enabled },
        }));
      }
    } catch (reason) {
      if (pdfPreflightSaveActiveRef.current === saveId) {
        setSnapshot((current) => ({
          ...current,
          settings: { ...current.settings, pdfPreflightEnabled: previous },
        }));
      }
      setError(describeError(reason, "PDF 文档预检设置保存失败"));
    } finally {
      if (pdfPreflightSaveActiveRef.current === saveId) {
        pdfPreflightSaveActiveRef.current = null;
        setPdfPreflightSaving(false);
      }
    }
  }, [pdfPreflightSaving, snapshot.settings.pdfPreflightEnabled]);

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
            handleSourceTextChange(request.sourceText);
            setSourceLanguage(request.sourceLanguage);
            setTargetLanguage(request.targetLanguage);
            setTab("translate");
            setError(null);
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
  }, [handleSourceTextChange]);

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
    const requestMode = activeRequestModeRef.current;
    const requestSourceText = activeRequestSourceTextRef.current;
    const requestRawSourceText = activeRequestRawSourceTextRef.current;
    activeRequestId.current = null;
    activeRequestModeRef.current = null;
    activeRequestSourceTextRef.current = null;
    activeRequestRawSourceTextRef.current = null;
    setActiveRequestMode(null);
    const startedAt = translationStartedAt.current;
    translationStartedAt.current = null;
    switch (result.outcome) {
      case "completed":
        if (requestMode === "learning") {
          const learning = requestSourceText === null
            ? null
            : decodeParagraphLearningResult(result.learning, requestSourceText);
          if (!learning || requestRawSourceText === null || sourceTextRef.current !== requestRawSourceText) {
            setLearningResult(null);
            setTranslatedText("");
            setTranslationSummary(null);
            setError(requestRawSourceText !== null && sourceTextRef.current !== requestRawSourceText
              ? "原文在翻译完成前已修改，学习结果已丢弃，请重新翻译。"
              : "学习模式返回的结果无法与原文对应，请重试。");
            setStatus("failed");
            return;
          }
          setLearningResult(learning);
        } else {
          setLearningResult(null);
        }
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
        setLearningResult(null);
        setTranslationSummary(null);
        setError(null);
        setStatus("idle");
        break;
      case "failed":
        setLearningResult(null);
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
        if (activeRequestModeRef.current === "plain") {
          setTranslatedText((current) => current + event.content);
        }
        break;
      case "completed":
        applyTranslationResult(event.requestId, {
          outcome: "completed",
          content: event.content,
          cacheHit: event.cacheHit,
          learning: event.learning,
          message: null,
        });
        break;
      case "cancelled":
        applyTranslationResult(event.requestId, {
          outcome: "cancelled",
          content: null,
          cacheHit: false,
          learning: null,
          message: null,
        });
        break;
      case "failed":
        applyTranslationResult(event.requestId, {
          outcome: "failed",
          content: null,
          cacheHit: false,
          learning: null,
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
          const event = decodeTranslationEvent(name, payload, activeRequestSourceTextRef.current ?? undefined);
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

  const handlePdfEngineActivityEvent = useCallback((event: PdfEngineEvent) => {
    if (event.type === "prepareStarted") {
      if (!event.operationId) return;
      clearResourceDownloadTerminal("pdf-engine");
      activePdfEngineOperationId.current = event.operationId;
      dispatchDownloadActivity({
        type: "started",
        resource: "pdf-engine",
        operationId: event.operationId,
        phase: "index",
        message: "正在准备 PDF Engine",
      });
      return;
    }
    if (event.type === "prepareProgress") {
      const operationId = event.progress.operationId;
      if (!operationId) return;
      if (activePdfEngineOperationId.current && activePdfEngineOperationId.current !== operationId) return;
      dispatchDownloadActivity({
        type: "progress",
        resource: "pdf-engine",
        operationId,
        phase: event.progress.stage,
        stagePercent: normalizeStagePercent(event.progress.current, event.progress.total, event.progress.fraction),
        message: event.progress.message,
      });
      return;
    }
    if (event.type === "prepareCompleted") {
      if (!event.operationId) return;
      if (activePdfEngineOperationId.current && activePdfEngineOperationId.current !== event.operationId) return;
      dispatchDownloadActivity({
        type: "completed",
        resource: "pdf-engine",
        operationId: event.operationId,
        message: "PDF Engine 已准备完成",
      });
      markResourceDownloadTerminal("pdf-engine", "completed");
      activePdfEngineOperationId.current = null;
      scheduleDownloadActivityRemoval("pdf-engine", event.operationId, 2600);
      return;
    }
    if (event.type === "prepareFailed") {
      if (!event.operationId) return;
      if (activePdfEngineOperationId.current && activePdfEngineOperationId.current !== event.operationId) return;
      dispatchDownloadActivity({
        type: "failed",
        resource: "pdf-engine",
        operationId: event.operationId,
        error: event.message,
      });
      markResourceDownloadTerminal("pdf-engine", "failed");
      activePdfEngineOperationId.current = null;
      scheduleDownloadActivityRemoval("pdf-engine", event.operationId, 9000);
    }
  }, [clearResourceDownloadTerminal, markResourceDownloadTerminal, scheduleDownloadActivityRemoval]);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const initialisePdfEngineListeners = async () => {
      const results = await Promise.allSettled(PDF_ENGINE_EVENT_NAMES.map((name) => (
        listenTo<unknown>(name, (payload) => {
          if (disposed) return;
          const event = decodePdfEngineEvent(name, payload);
          if (event) handlePdfEngineActivityEvent(event);
        })
      )));
      for (const result of results) {
        if (result.status !== "fulfilled") continue;
        if (disposed) result.value();
        else unlisteners.push(result.value);
      }
    };
    void initialisePdfEngineListeners();
    return () => {
      disposed = true;
      unlisteners.splice(0).forEach((unlisten) => unlisten());
    };
  }, [handlePdfEngineActivityEvent]);

  const handleDictionaryEvent = useCallback((event: DictionaryUpdateEvent) => {
    if (event.type === "started") {
      clearResourceDownloadTerminal("dictionary");
      dispatchDownloadActivity({
        type: "started",
        resource: "dictionary",
        operationId: event.operationId,
        phase: "download",
        message: "正在下载词典",
      });
    } else if (event.type === "downloadProgress") {
      dispatchDownloadActivity({
        type: "progress",
        resource: "dictionary",
        operationId: event.operationId,
        phase: "download",
        stagePercent: normalizeStagePercent(event.downloadedBytes, event.totalBytes),
      });
    } else if (event.type === "verifyProgress") {
      dispatchDownloadActivity({
        type: "progress",
        resource: "dictionary",
        operationId: event.operationId,
        phase: "verify",
        stagePercent: normalizeStagePercent(event.current, event.total),
      });
    } else if (event.type === "extractProgress") {
      dispatchDownloadActivity({
        type: "progress",
        resource: "dictionary",
        operationId: event.operationId,
        phase: "extract",
        stagePercent: normalizeStagePercent(event.current, event.total),
      });
    } else if (event.type === "completed") {
      dispatchDownloadActivity({
        type: "completed",
        resource: "dictionary",
        operationId: event.operationId,
        message: "词典已准备完成",
      });
    } else {
      dispatchDownloadActivity({
        type: "failed",
        resource: "dictionary",
        operationId: event.operationId,
        error: event.message,
      });
    }

    if (event.type === "started") {
      if (activeDictionaryOperationId.current && activeDictionaryOperationId.current !== event.operationId) return;
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
        markResourceDownloadTerminal("dictionary", "completed");
        scheduleDownloadActivityRemoval("dictionary", event.operationId, 2600);
        void refreshSnapshot();
        break;
      case "failed":
        activeDictionaryOperationId.current = null;
        setDictionaryProgress(null);
        setSnapshot((current) => ({
          ...current,
          dictionary: { ...current.dictionary, status: "failed", error: event.message },
        }));
        markResourceDownloadTerminal("dictionary", "failed");
        scheduleDownloadActivityRemoval("dictionary", event.operationId, 9000);
        break;
    }
  }, [clearResourceDownloadTerminal, markResourceDownloadTerminal, refreshSnapshot, scheduleDownloadActivityRemoval]);

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
          source: request.source,
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
  const resourceDownloadActivity = resourceDownloadPrompt
    ? selectDownloadActivity(downloadActivityState, resourceDownloadPrompt.resource)
    : null;
  const resourceDownloadStatus: ResourceDownloadDialogStatus = resourceDownloadActivity?.status
    ?? (resourceDownloadPrompt ? resourceDownloadTerminalState[resourceDownloadPrompt.resource] : undefined)
    ?? (resourceDownloadPrompt?.resource === "dictionary" && dictionaryProgress !== null
      ? "running"
      : resourceDownloadPrompt?.resource === "dictionary" && snapshot.dictionary.status === "updating"
        ? "running"
        : resourceDownloadPrompt?.resource === "dictionary" && snapshot.dictionary.status === "failed"
          ? "failed"
          : "missing");
  const resourceDownloadError = resourceDownloadActivity?.error
    ?? (resourceDownloadPrompt?.resource === "dictionary" && snapshot.dictionary.status === "failed"
      ? snapshot.dictionary.error
      : null);
  const downloadActivities = listDownloadActivities(downloadActivityState);

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
    const requestMode: TranslationRequestMode = snapshot.settings.paragraphLearningModeEnabled ? "learning" : "plain";
    const requestId = crypto.randomUUID();
    activeRequestId.current = requestId;
    activeRequestModeRef.current = requestMode;
    activeRequestSourceTextRef.current = text;
    activeRequestRawSourceTextRef.current = sourceText;
    setActiveRequestMode(requestMode);
    setError(null);
    setLearningResult(null);
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
          learningMode: requestMode === "learning",
        },
      });
      if (activeRequestId.current !== requestId) return;
      const result = decodeTranslationCommandResult(rawResult, text);
      if (!result) {
        activeRequestId.current = null;
        activeRequestModeRef.current = null;
        activeRequestSourceTextRef.current = null;
        activeRequestRawSourceTextRef.current = null;
        setActiveRequestMode(null);
        setLearningResult(null);
        translationStartedAt.current = null;
        setError("翻译命令返回了无法识别的终态。");
        setStatus("failed");
        return;
      }
      applyTranslationResult(requestId, result);
    } catch (reason) {
      if (activeRequestId.current !== requestId) return;
      activeRequestId.current = null;
      activeRequestModeRef.current = null;
      activeRequestSourceTextRef.current = null;
      activeRequestRawSourceTextRef.current = null;
      setActiveRequestMode(null);
      setLearningResult(null);
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
        activeRequestModeRef.current = null;
        activeRequestSourceTextRef.current = null;
        activeRequestRawSourceTextRef.current = null;
        setActiveRequestMode(null);
        setLearningResult(null);
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
  };

  const handleSettingsSaved = useCallback((next: AppSnapshot) => {
    setSnapshot((current) => ({
      ...next,
      settings: {
        ...next.settings,
        // The PDF switch has its own command and may still be optimistically
        // saving while the settings workspace refreshes its snapshot.
        pdfPreflightEnabled: current.settings.pdfPreflightEnabled,
      },
    }));
  }, []);

  const openPersonalDictionary = useCallback(() => {
    setTab("personal");
  }, []);
  const usesBoundedListLayout = tab === "personal" || tab === "glossary";
  const usesInternalScrollLayout = tab === "pdf";

  return (
    <div className="app-shell">
      <div className="main-surface">
      <header className={`app-chrome ${settingsOpen ? "is-settings-open" : ""}`} onMouseDown={handleTitlebarMouseDown}>
        <div className="brand-lockup">
          <img className="brand-logo" src={liltLogo} alt="" />
          <span className="brand-name">Lilt</span>
        </div>
        {!settingsOpen && <ModeSwitcher activeTab={tab === "personal" ? "dictionary" : tab} onChange={setTab} />}
        <div className="chrome-actions" data-no-drag>
          <button ref={settingsToggleRef} className={`chrome-icon-button ${settingsOpen ? "is-active" : ""}`} type="button" onClick={settingsOpen ? requestSettingsClose : openSettings} aria-label={settingsOpen ? "返回 Lilt" : "打开设置"} aria-expanded={settingsOpen} title={settingsOpen ? "返回 Lilt" : "设置"}>{settingsOpen ? <ArrowLeft size={16} /> : <Settings size={16} />}</button>
          <div className="window-controls" data-no-drag>
            <button className="window-control" type="button" onClick={minimizeMainWindow} aria-label="最小化窗口" title="最小化"><Minus size={14} /></button>
            <button className="window-control" type="button" onClick={() => void toggleMainWindowMaximized()} aria-label={mainWindowMaximized ? "还原窗口" : "最大化窗口"} title={mainWindowMaximized ? "还原" : "最大化"}>{mainWindowMaximized ? <Minimize2 size={13} /> : <Maximize2 size={13} />}</button>
            <button className="window-control window-control-close" type="button" onClick={closeMainWindow} aria-label="关闭窗口" title="关闭"><X size={14} /></button>
          </div>
        </div>
      </header>

        {settingsWorkspaceMounted && (
          <div
            className={`settings-workspace ${settingsOpen ? "settings-workspace-entering" : "settings-workspace-exiting"}`}
            ref={settingsWorkspaceRef}
            role="region"
            aria-labelledby="settings-workspace-title"
            tabIndex={-1}
            onAnimationEnd={(event) => {
              if (event.target !== event.currentTarget) return;
              finishSettingsClose();
            }}
          >
            <SettingsView
              snapshot={snapshot}
              dictionaryProgress={dictionaryProgress}
              dictionaryEventsError={dictionaryEventsError}
              pdfEngine={pdfEngine}
              navigationTarget={settingsNavigationTarget}
              onDictionaryUpdate={handleDictionaryUpdate}
              onSaved={handleSettingsSaved}
              onToast={showAppToast}
              onCheckForUpdates={() => void checkForUpdates("manual")}
              releaseCheckPending={releaseCheckPending}
            />
          </div>
        )}
      <main inert={settingsOpen} className={`main-content ${tab === "translate" ? "translate-main-content" : ""} ${usesBoundedListLayout ? "bounded-list-main-content" : ""} ${usesInternalScrollLayout ? "pdf-main-content" : ""}`}>
        <div className={`pdf-persistent-host ${!settingsOpen && tab === "pdf" ? "is-active" : "is-inactive"}`}>
          <PdfView
            active={!settingsOpen && tab === "pdf"}
            pdfEngine={pdfEngine}
            pdfPreflightEnabled={snapshot.settings.pdfPreflightEnabled}
            pdfPreflightPageLimit={snapshot.settings.pdfPreflightPageLimit}
            pdfPreflightSaving={pdfPreflightSaving}
            onPdfPreflightEnabledChange={(enabled) => { void handlePdfPreflightEnabledChange(enabled); }}
            onResourceDownloadPrompt={openResourceDownloadPrompt}
            onOpenPdfEngineSettings={openPdfEngineSettings}
            onPdfTranslationFloatingChange={setPdfTranslationFloating}
          />
        </div>
        {!settingsOpen && tab !== "pdf" && (
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
                  activeRequestMode={activeRequestMode}
                  learningModeEnabled={snapshot.settings.paragraphLearningModeEnabled}
                  learningModeSaving={learningModeSaving}
                  learningResult={learningResult}
                  translationSummary={translationSummary}
                  eventsReady={translationEventsReady}
                  onSourceTextChange={handleSourceTextChange}
                  onSourceLanguageChange={setSourceLanguage}
                  onTargetLanguageChange={setTargetLanguage}
                  onLearningModeChange={(enabled) => { void handleLearningModeChange(enabled); }}
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
                  snapshotReady={snapshotReady}
                  targetLanguage={targetLanguage}
                  wordExample={wordExample}
                  onUpdate={handleDictionaryUpdate}
                  onHistoryChanged={handleDictionaryHistoryChanged}
                  onSnapshotChanged={refreshSnapshot}
                  onWordExampleRequested={(request) => { void handleWordExampleRequested(request); }}
                  onWordExampleCancelled={() => { void cancelWordExampleRequest(); }}
                  personalDictionary={snapshot.personalDictionary}
                  onResourceDownloadPrompt={openResourceDownloadPrompt}
                  openRequest={dictionaryOpenRequest}
                  onOpenRequestHandled={() => setDictionaryOpenRequest(null)}
                  onPersonalDictionaryChanged={handlePersonalDictionaryChanged}
                  onOpenPersonalDictionary={openPersonalDictionary}
                  onOpenDictionaryAbout={openDictionaryAbout}
                />
              )}
              {tab === "personal" && (
                <PersonalDictionaryView
                  entries={snapshot.personalDictionary}
                  onOpen={openPersonalWord}
                  onRemove={(entry) => { void removePersonalWord(entry); }}
                  onExport={() => openDataTransfer("personalExport")}
                  onBack={() => setTab("dictionary")}
                />
              )}
              {tab === "glossary" && (
                <GlossaryView
                  terms={snapshot.glossaryTerms}
                  onChanged={() => void refreshSnapshot()}
                  onImport={() => openDataTransfer("glossaryImport")}
                  onExport={() => openDataTransfer("glossaryExport")}
                />
              )}
            </div>
          </PageTransition>
        )}
      </main>
      </div>
      {historyOverlayMounted && (
        <AnimatedOverlay
          className="history-overlay"
          open={historyOpen}
          onClosed={handleHistoryClosed}
          onBackdropClick={(event) => {
            if (event.target === event.currentTarget) requestHistoryClose();
          }}
        >
          <div className="history-dialog" ref={historyDialogRef} role="dialog" aria-modal="true" aria-labelledby="history-dialog-title" tabIndex={-1}>
            <div className="history-dialog-heading">
              <div>
                <span className="history-dialog-eyebrow">HISTORY</span>
                <strong id="history-dialog-title">翻译历史</strong>
              </div>
              <button className="icon-button" type="button" onClick={requestHistoryClose} aria-label="关闭翻译历史" title="关闭翻译历史"><X size={17} /></button>
            </div>
            <div className="history-dialog-scroll">
              <HistoryContent history={snapshot.history} />
            </div>
          </div>
        </AnimatedOverlay>
      )}
      {dataTransferOverlayMounted && (
        <DataTransferDialog
          mode={dataTransferMode}
          open={dataTransferOpen}
          entryCount={snapshot.personalDictionary.length}
          glossaryEntryCount={snapshot.glossaryTerms.length}
          onImported={() => { void refreshSnapshot(); }}
          onRequestClose={requestDataTransferClose}
          onClosed={handleDataTransferClosed}
        />
      )}
      {resourceDownloadDialogMounted && resourceDownloadPrompt && (
        <ResourceDownloadDialog
          open={resourceDownloadDialogOpen}
          resource={resourceDownloadPrompt.resource}
          title={resourceDownloadPrompt.title}
          description={resourceDownloadPrompt.description}
          status={resourceDownloadStatus}
          phase={resourceDownloadActivity?.phase ?? null}
          stagePercent={resourceDownloadActivity?.stagePercent ?? null}
          overallPercent={resourceDownloadActivity?.overallPercent ?? null}
          message={resourceDownloadActivity?.message ?? null}
          error={resourceDownloadError}
          startLabel={resourceDownloadPrompt.startLabel}
          failedLabel={resourceDownloadPrompt.failedLabel}
          onStart={startResourceDownload}
          onRequestClose={requestResourceDownloadClose}
          onClosed={handleResourceDownloadClosed}
        />
      )}
      {closeDialogMounted && (
        <CloseBehaviorDialog
          open={closeDialogOpen}
          onResolved={requestCloseDialog}
          onClosed={handleCloseDialogClosed}
        />
      )}
      {releaseNoticeMounted && releaseNotice && (
        <ReleaseNoticeDialog
          open={releaseNoticeOpen}
          release={releaseNotice}
          currentVersion={APP_VERSION}
          updateStatus={releaseUpdateStatus}
          downloadedBytes={releaseDownloadBytes}
          downloadTotal={releaseDownloadTotal}
          updateError={releaseUpdateError}
          onInstall={() => void handleReleaseUpdate()}
          onRequestClose={requestReleaseNoticeClose}
          onClosed={handleReleaseNoticeClosed}
        />
      )}
      <div className="error-toast-anchor">
        <FeedbackMessage message={error ?? translationEventsError} kind="error" as="div" className="error-toast toast-message" />
        <FeedbackMessage message={appToast?.kind === "error" ? appToast.message : null} kind="error" as="div" className="error-toast toast-message" />
        <FeedbackMessage message={appToast?.kind === "notice" ? appToast.message : null} kind="notice" as="div" className="notice-toast toast-message" />
        <FeedbackMessage message={releaseCheckMessage} kind="notice" as="div" className="notice-toast toast-message" />
      </div>
      <DownloadActivityStack
        activities={downloadActivities}
        pdfTranslation={pdfTranslationFloating}
        onPdfTranslationClick={openPdf}
        onResourceClick={handleDownloadActivityClick}
      />
    </div>
  );
}

function DataTransferDialog({
  mode,
  open: isOpen,
  entryCount,
  glossaryEntryCount,
  onImported,
  onRequestClose,
  onClosed,
}: {
  mode: DataTransferMode;
  open: boolean;
  entryCount: number;
  glossaryEntryCount: number;
  onImported: () => void;
  onRequestClose: () => void;
  onClosed: () => void;
}) {
  const [status, setStatus] = useState<DataTransferStatus>("selecting");
  const [error, setError] = useState<string | null>(null);
  const [exportResult, setExportResult] = useState<PersonalDictionaryExportResult | null>(null);
  const [glossaryExportResult, setGlossaryExportResult] = useState<GlossaryExportResult | null>(null);
  const [importResult, setImportResult] = useState<GlossaryImportResult | null>(null);
  const dialogRef = useRef<HTMLDivElement | null>(null);
  const startedRef = useRef(false);
  const busy = status === "selecting" || status === "processing";

  const startTransfer = useCallback(async () => {
    setError(null);
    setExportResult(null);
    setGlossaryExportResult(null);
    setImportResult(null);
    if (mode === "personalExport" && entryCount === 0) {
      setStatus("empty");
      return;
    }
    if (mode === "glossaryExport" && glossaryEntryCount === 0) {
      setStatus("empty");
      return;
    }

    setStatus("selecting");
    try {
      const selected = mode === "glossaryImport"
        ? await open({
          directory: false,
          multiple: false,
          filters: [{ name: "CSV 文件", extensions: ["csv"] }],
        })
        : await save({
          defaultPath: mode === "personalExport" ? "lilt-personal-dictionary.txt" : "lilt-glossary.csv",
          filters: [{ name: mode === "personalExport" ? "TXT 文件" : "CSV 文件", extensions: [mode === "personalExport" ? "txt" : "csv"] }],
        });
      const filePath = Array.isArray(selected) ? selected[0] : selected;
      if (!filePath) {
        setStatus("cancelled");
        return;
      }

      setStatus("processing");
      if (mode === "personalExport") {
        const rawResult = await invokeCommand<unknown>("export_personal_dictionary", { filePath });
        const result = decodePersonalDictionaryExportResult(rawResult);
        if (!result) throw new Error("导出命令返回了无法识别的结果。");
        setExportResult(result);
      } else if (mode === "glossaryExport") {
        const rawResult = await invokeCommand<unknown>("export_glossary", { filePath });
        const result = decodeGlossaryExportResult(rawResult);
        if (!result) throw new Error("导出命令返回了无法识别的结果。");
        setGlossaryExportResult(result);
      } else {
        const rawResult = await invokeCommand<unknown>("import_glossary", { filePath });
        const result = decodeGlossaryImportResult(rawResult);
        if (!result) throw new Error("导入命令返回了无法识别的结果。");
        setImportResult(result);
        onImported();
      }
      setStatus("success");
    } catch (reason) {
      setError(describeError(
        reason,
        mode === "personalExport" ? "个人词典导出失败" : mode === "glossaryImport" ? "术语表导入失败" : "术语表导出失败",
      ));
      setStatus("error");
    }
  }, [entryCount, glossaryEntryCount, mode, onImported]);

  useEffect(() => {
    if (!isOpen) {
      startedRef.current = false;
      return;
    }
    if (startedRef.current) return;
    startedRef.current = true;
    const frame = window.requestAnimationFrame(() => {
      dialogRef.current?.focus();
      void startTransfer();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [isOpen, startTransfer]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || busy) return;
      event.preventDefault();
      onRequestClose();
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [busy, isOpen, onRequestClose]);

  const title = mode === "personalExport" ? "导出个人词典" : mode === "glossaryExport" ? "导出术语表" : "导入术语表";
  const description = mode === "personalExport"
    ? "将当前个人词典按列表顺序保存为 UTF-8 TXT。"
    : mode === "glossaryExport"
      ? "将原文和译文保存为 UTF-8 CSV，不包含备注。"
      : "读取 UTF-8 CSV，导入原文和译文，已有备注不会改变。";
  const retryable = status === "cancelled" || status === "error";

  return (
    <AnimatedOverlay
      className="modal-backdrop data-transfer-backdrop"
      open={isOpen}
      onClosed={onClosed}
      onBackdropClick={(event) => {
        if (!busy && event.target === event.currentTarget) onRequestClose();
      }}
    >
      <div className="modal-card data-transfer-card" ref={dialogRef} role="dialog" aria-modal="true" aria-labelledby="data-transfer-dialog-title" tabIndex={-1}>
        <div className="modal-heading">
          <div>
            <strong id="data-transfer-dialog-title">{title}</strong>
            <span>{description}</span>
          </div>
          <button className="icon-button" type="button" onClick={onRequestClose} disabled={busy} aria-label="关闭" title="关闭"><X size={17} /></button>
        </div>

        <div className="data-transfer-body" aria-live="polite">
          {status === "selecting" && <p className="data-transfer-status"><LoaderCircle className="spin" size={16} />正在等待选择文件</p>}
          {status === "processing" && <p className="data-transfer-status"><LoaderCircle className="spin" size={16} />{mode === "glossaryImport" ? "正在读取并导入术语" : "正在写入文件"}</p>}
          {status === "empty" && <p className="data-transfer-status">{mode === "personalExport" ? "个人词典为空，没有可导出的内容。" : "术语表为空，没有可导出的内容。"}</p>}
          {status === "cancelled" && <p className="data-transfer-status">未选择文件，操作已取消。</p>}
          {status === "success" && mode === "personalExport" && exportResult && (
            <div className="data-transfer-result">
              <p className="notice-message">已导出 {exportResult.entryCount} 条个人词条。</p>
              <span>文件：{exportResult.fileName}</span>
            </div>
          )}
          {status === "success" && mode === "glossaryExport" && glossaryExportResult && (
            <div className="data-transfer-result">
              <p className="notice-message">已导出 {glossaryExportResult.entryCount} 条术语。</p>
              <span>文件：{glossaryExportResult.fileName}</span>
            </div>
          )}
          {status === "success" && mode === "glossaryImport" && importResult && (
            <div className="data-transfer-result">
              <p className="notice-message">术语表导入完成，共处理 {importResult.addedCount + importResult.updatedCount} 条不同原文。</p>
              <span>新增 {importResult.addedCount} 条，更新 {importResult.updatedCount} 条，跳过 {importResult.skippedCount} 行。</span>
              {importResult.skippedRows.length > 0 && (
                <div className="data-transfer-skipped">
                  <strong>异常行</strong>
                  <ul>
                    {importResult.skippedRows.map((row) => <li key={`${row.line}-${row.reason}`}>第 {row.line} 行：{row.reason}</li>)}
                  </ul>
                </div>
              )}
            </div>
          )}
          {status === "error" && <p className="error-message data-transfer-error">{error}</p>}
        </div>

        <div className="form-actions modal-actions">
          {retryable && <button className="secondary-button" type="button" onClick={() => void startTransfer()}><Download size={15} />重新选择</button>}
          <button className="primary-button" type="button" onClick={onRequestClose} disabled={busy}>关闭</button>
        </div>
      </div>
    </AnimatedOverlay>
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

function ReleaseNoticeDialog({
  open,
  release,
  currentVersion,
  updateStatus,
  downloadedBytes,
  downloadTotal,
  updateError,
  onInstall,
  onRequestClose,
  onClosed,
}: {
  open: boolean;
  release: GitHubReleaseSummary;
  currentVersion: string;
  updateStatus: ReleaseUpdateStatus;
  downloadedBytes: number;
  downloadTotal: number | null;
  updateError: string | null;
  onInstall: () => void;
  onRequestClose: () => void;
  onClosed: () => void;
}) {
  const dialogRef = useRef<HTMLDivElement | null>(null);
  const isBusy = updateStatus === "downloading" || updateStatus === "installing";
  const progress = downloadTotal === null
    ? null
    : Math.min(100, Math.round((downloadedBytes / Math.max(downloadTotal, 1)) * 100));

  useEffect(() => {
    if (!open) return;
    const frame = window.requestAnimationFrame(() => dialogRef.current?.focus());
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      onRequestClose();
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      window.cancelAnimationFrame(frame);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [onRequestClose, open]);

  return (
    <AnimatedOverlay
      className="modal-backdrop release-notice-backdrop"
      open={open}
      onClosed={onClosed}
      onBackdropClick={(event) => {
        if (event.target === event.currentTarget) onRequestClose();
      }}
    >
      <div className="modal-card release-notice-card" ref={dialogRef} role="dialog" aria-modal="true" aria-labelledby="release-notice-title" tabIndex={-1}>
        <div className="modal-heading">
          <div>
            <strong id="release-notice-title">发现新版本</strong>
            <span>GitHub Release 已发布新的稳定版本。</span>
          </div>
          <button className="icon-button" type="button" onClick={onRequestClose} disabled={isBusy} aria-label="关闭" title="关闭"><X size={17} /></button>
        </div>
        <div className="release-notice-body">
          <p>当前版本 v{currentVersion}，最新版本 {release.tagName}。</p>
          <span>官方更新器会下载并安装 Windows x64 版本，完成后自动重启应用。</span>
          <div className="release-notice-notes">{release.notes ?? "此次版本未提供 Release 说明。"}</div>
          {updateStatus === "downloading" && (
            <div className="release-notice-progress" aria-live="polite">
              <div className="release-notice-progress-label">
                <span>正在下载更新</span>
                <span>{downloadTotal === null ? formatBytes(downloadedBytes) : `${formatBytes(downloadedBytes)} / ${formatBytes(downloadTotal)}`}</span>
              </div>
              <div
                className={`release-notice-progress-track ${progress === null ? "is-indeterminate" : ""}`}
                role="progressbar"
                aria-valuemin={0}
                aria-valuemax={downloadTotal ?? undefined}
                aria-valuenow={downloadTotal === null ? undefined : downloadedBytes}
                aria-label="更新下载进度"
              >
                <span style={progress === null ? undefined : { width: `${progress}%` }} />
              </div>
            </div>
          )}
          {updateStatus === "installing" && (
            <p className="release-notice-status" aria-live="polite"><LoaderCircle className="spin" size={14} />正在安装更新，应用即将重启。</p>
          )}
          {updateStatus === "failed" && updateError && <p className="error-message release-notice-error">{updateError}</p>}
        </div>
        <div className="form-actions modal-actions">
          <button className="secondary-button" type="button" onClick={onRequestClose} disabled={isBusy}>稍后</button>
          {(updateStatus === "ready" || updateStatus === "failed") && (
            <button className="primary-button release-notice-install" type="button" onClick={onInstall} disabled={isBusy}>
              {updateStatus === "failed" ? <><Download size={15} />重试更新</> : <><Download size={15} />更新并重启</>}
            </button>
          )}
          <a className="primary-button release-notice-link" href={release.htmlUrl} target="_blank" rel="noreferrer" onClick={(event) => { event.preventDefault(); openExternalUrl(release.htmlUrl); }}><ExternalLink size={15} />查看 Release</a>
        </div>
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
      <ModeButton buttonRef={setButtonRef("pdf")} icon={<FileType2 size={15} />} label="PDF" active={activeTab === "pdf"} onClick={() => onChange("pdf")} />
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
  activeRequestMode: TranslationRequestMode | null;
  learningModeEnabled: boolean;
  learningModeSaving: boolean;
  learningResult: ParagraphLearningResult | null;
  translationSummary: TranslationSummary | null;
  eventsReady: boolean;
  onSourceTextChange: (value: string) => void;
  onSourceLanguageChange: (value: string) => void;
  onTargetLanguageChange: (value: string) => void;
  onLearningModeChange: (enabled: boolean) => void;
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
  options = LANGUAGE_OPTIONS,
}: {
  id: string;
  ariaLabel: string;
  value: string;
  onChange: (value: string) => void;
  options?: readonly (readonly [string, string])[];
}) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const buttonRef = useRef<HTMLButtonElement | null>(null);
  const optionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const [open, setOpen] = useState(false);
  const selectedIndex = Math.max(0, options.findIndex(([, code]) => code === value));
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
        ? (highlightedIndex + direction + options.length) % options.length
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
      focusOption((index + direction + options.length) % options.length);
      return;
    }
    if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      focusOption(event.key === "Home" ? 0 : options.length - 1);
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
        <span>{languageLabel(value, options)}</span>
        <ChevronDown size={13} strokeWidth={1.8} aria-hidden="true" />
      </button>
      {open && (
        <div className="translation-language-menu" id={`${id}-menu`} role="listbox" aria-label={ariaLabel}>
          {options.map(([label, optionValue], index) => (
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
  const isLearningRequest = props.activeRequestMode === "learning";
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
            <label className={`learning-mode-toggle ${props.learningModeEnabled ? "is-enabled" : ""}`} title="仅影响主段落翻译">
              <input
                type="checkbox"
                checked={props.learningModeEnabled}
                disabled={isBusy || props.learningModeSaving}
                onChange={(event) => props.onLearningModeChange(event.target.checked)}
                aria-label="段落翻译学习模式"
              />
              <span className="learning-mode-toggle-track" aria-hidden="true"><span /></span>
              <span className="learning-mode-toggle-label">学习模式</span>
              <span className="learning-mode-toggle-state">{props.learningModeSaving ? "保存中" : props.learningModeEnabled ? "已开启" : "未开启"}</span>
            </label>
          </div>
        </div>
      </div>

      <div className="translation-grid">
        <div className="translation-column">
          <div className="translation-language">
            <span>原文</span>
            <LanguageSelect id="source-language" ariaLabel="原文语言" value={props.sourceLanguage} onChange={props.onSourceLanguageChange} options={SOURCE_LANGUAGE_OPTIONS} />
          </div>
          <div className="translation-panel">
            <div className="translation-scroll-region">
              <SegmentedSourceEditor
                value={props.sourceText}
                learning={props.learningResult}
                onChange={props.onSourceTextChange}
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
                {props.translatedText ? (
                  <>
                    {props.translatedText}
                    {props.status === "streaming" && <span className="stream-caret" />}
                  </>
                ) : (
                  <>
                    {props.status === "streaming" && <span className="stream-caret is-leading" />}
                    {isLearningRequest && isBusy
                      ? <span className="learning-stream-status">正在整理学习分段……</span>
                      : <span className="empty-result">译文会显示在这里</span>}
                  </>
                )}
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

function GlossaryView({ terms, onChanged, onImport, onExport }: { terms: GlossaryTerm[]; onChanged: () => void; onImport: () => void; onExport: () => void }) {
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
      <PageTitle
        eyebrow="GLOSSARY"
        title="术语表"
        actions={<div className="button-group"><button className="secondary-button small-button" type="button" onClick={onImport}><Download size={15} />导入术语表</button><button className="secondary-button small-button" type="button" onClick={onExport}><Upload size={15} />导出术语表</button></div>}
      />
      <div className="simple-card">
        <div className="form-grid glossary-form">
          <label>原文<input value={source} onChange={(event) => setSource(event.target.value)} /></label>
          <label>译文<input value={target} onChange={(event) => setTarget(event.target.value)} /></label>
          <label className="wide-field">备注（可选）<input value={note} onChange={(event) => setNote(event.target.value)} /></label>
        </div>
        <div className="form-actions"><span className="error-message">{error}</span><button className="secondary-button" type="button" onClick={() => void addTerm()}>添加术语</button></div>
      </div>
      <div className="list-card bounded-list-card glossary-terms-card">
        <div className="list-card-heading"><strong>已添加术语</strong><span>{terms.length} 条</span></div>
        <div className="bounded-list-card-scroll">
          {terms.length === 0 ? <div className="empty-list">还没有术语</div> : (
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
        {history.length === 0 ? <div className="empty-list">完成一次段落翻译后，记录会出现在这里</div> : history.map((item) => <HistoryRow key={item.id} item={item} />)}
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
  onToast,
}: {
  prompts: Prompt[];
  currentPromptId: string;
  onChanged: () => Promise<void>;
  onToast: ShowToast;
}) {
  type PromptDraft = { selectedId: string | null; name: string; content: string; sourceLanguage: string; targetLanguage: string };
  const initialPrompt = prompts.find((prompt) => prompt.id === currentPromptId) ?? prompts[0] ?? null;
  const [selectedId, setSelectedId] = useState<string | null>(initialPrompt?.id ?? null);
  const [name, setName] = useState(initialPrompt?.name ?? "");
  const [content, setContent] = useState(initialPrompt?.content ?? "");
  const [sourceLanguage, setSourceLanguage] = useState(initialPrompt?.sourceLanguage ?? "auto");
  const [targetLanguage, setTargetLanguage] = useState(initialPrompt?.targetLanguage ?? "zh-CN");
  const [creating, setCreating] = useState(false);
  const promptDraftRef = useRef<PromptDraft>({
    selectedId: initialPrompt?.id ?? null,
    name: initialPrompt?.name ?? "",
    content: initialPrompt?.content ?? "",
    sourceLanguage: initialPrompt?.sourceLanguage ?? "auto",
    targetLanguage: initialPrompt?.targetLanguage ?? "zh-CN",
  });
  const promptSaveTimerRef = useRef<number | null>(null);
  const promptSaveVersionRef = useRef(0);
  const promptSavingRef = useRef(false);
  const promptDirtyRef = useRef(false);
  const promptDisposedRef = useRef(false);
  const pendingSelectionIdRef = useRef<string | null>(null);
  const queuedPromptSaveRef = useRef<{ draft: PromptDraft; version: number } | null>(null);

  useEffect(() => {
    if (promptDirtyRef.current) return;
    const pendingSelectionId = pendingSelectionIdRef.current;
    if (pendingSelectionId && !prompts.some((prompt) => prompt.id === pendingSelectionId)) return;
    if (pendingSelectionId) pendingSelectionIdRef.current = null;
    const selected = prompts.find((prompt) => prompt.id === selectedId)
      ?? prompts.find((prompt) => prompt.id === currentPromptId)
      ?? prompts[0]
      ?? null;
    if (!selected) {
      setSelectedId(null);
      setName("");
      setContent("");
      setSourceLanguage("auto");
      setTargetLanguage("zh-CN");
      promptDraftRef.current = { selectedId: null, name: "", content: "", sourceLanguage: "auto", targetLanguage: "zh-CN" };
      return;
    }
    if (selected.id !== selectedId) setSelectedId(selected.id);
    setName(selected.name);
    setContent(selected.content);
    setSourceLanguage(selected.sourceLanguage);
    setTargetLanguage(selected.targetLanguage);
    promptDraftRef.current = { selectedId: selected.id, name: selected.name, content: selected.content, sourceLanguage: selected.sourceLanguage, targetLanguage: selected.targetLanguage };
  }, [currentPromptId, prompts, selectedId]);

  const savePromptDraft = useCallback(async (draft: PromptDraft, version: number) => {
    if (promptSavingRef.current) {
      const queued = queuedPromptSaveRef.current;
      if (!queued || version >= queued.version) queuedPromptSaveRef.current = { draft, version };
      return;
    }
    if (!draft.selectedId || !draft.name.trim()) return;
    promptSavingRef.current = true;
    try {
      const raw = await invokeCommand<unknown>("update_prompt", {
        id: draft.selectedId,
        name: draft.name,
        content: draft.content,
        sourceLanguage: draft.sourceLanguage,
        targetLanguage: draft.targetLanguage,
      });
      const next = decodePrompt(raw);
      if (!next) throw new Error("提示词命令返回了无法识别的结果");
      if (version === promptSaveVersionRef.current) {
        promptDraftRef.current = { selectedId: next.id, name: next.name, content: next.content, sourceLanguage: next.sourceLanguage, targetLanguage: next.targetLanguage };
        promptDirtyRef.current = false;
        if (!promptDisposedRef.current) {
          setSelectedId(next.id);
          setName(next.name);
          setContent(next.content);
          setSourceLanguage(next.sourceLanguage);
          setTargetLanguage(next.targetLanguage);
        }
      }
      await onChanged();
    } catch (reason) {
      if (version === promptSaveVersionRef.current && !promptDisposedRef.current) {
        onToast(describeError(reason, "提示词自动保存失败"), "error");
      }
    } finally {
      promptSavingRef.current = false;
      const queued = queuedPromptSaveRef.current;
      queuedPromptSaveRef.current = null;
      if (queued) {
        void savePromptDraft(queued.draft, queued.version);
      } else if (version !== promptSaveVersionRef.current && promptDirtyRef.current) {
        void savePromptDraft(promptDraftRef.current, promptSaveVersionRef.current);
      }
    }
  }, [onChanged, onToast]);

  const schedulePromptSave = useCallback(() => {
    promptSaveVersionRef.current += 1;
    if (promptSaveTimerRef.current !== null) window.clearTimeout(promptSaveTimerRef.current);
    const version = promptSaveVersionRef.current;
    const draft = promptDraftRef.current;
    promptSaveTimerRef.current = window.setTimeout(() => {
      promptSaveTimerRef.current = null;
      void savePromptDraft(draft, version);
    }, 650);
  }, [savePromptDraft]);

  const updatePromptDraft = (patch: Partial<{ name: string; content: string; sourceLanguage: string; targetLanguage: string }>) => {
    const next = { ...promptDraftRef.current, ...patch };
    promptDraftRef.current = next;
    promptDirtyRef.current = true;
    setName(next.name);
    setContent(next.content);
    setSourceLanguage(next.sourceLanguage);
    setTargetLanguage(next.targetLanguage);
    schedulePromptSave();
  };

  useEffect(() => () => {
    promptDisposedRef.current = true;
    if (promptSaveTimerRef.current !== null) {
      window.clearTimeout(promptSaveTimerRef.current);
      promptSaveTimerRef.current = null;
      if (promptDirtyRef.current) void savePromptDraft(promptDraftRef.current, promptSaveVersionRef.current);
    }
  }, [savePromptDraft]);

  const selectPrompt = (prompt: Prompt) => {
    const pendingDraft = promptDraftRef.current;
    const pendingVersion = promptSaveVersionRef.current;
    if (promptSaveTimerRef.current !== null) {
      window.clearTimeout(promptSaveTimerRef.current);
      promptSaveTimerRef.current = null;
    }
    if (promptDirtyRef.current) void savePromptDraft(pendingDraft, pendingVersion);
    promptSaveVersionRef.current += 1;
    promptDirtyRef.current = false;
    pendingSelectionIdRef.current = null;
    promptDraftRef.current = { selectedId: prompt.id, name: prompt.name, content: prompt.content, sourceLanguage: prompt.sourceLanguage, targetLanguage: prompt.targetLanguage };
    setSelectedId(prompt.id);
    setName(prompt.name);
    setContent(prompt.content);
    setSourceLanguage(prompt.sourceLanguage);
    setTargetLanguage(prompt.targetLanguage);
  };

  const createPrompt = async () => {
    if (creating) return;
    const pendingDraft = promptDraftRef.current;
    const pendingVersion = promptSaveVersionRef.current;
    if (promptSaveTimerRef.current !== null) {
      window.clearTimeout(promptSaveTimerRef.current);
      promptSaveTimerRef.current = null;
    }
    if (promptDirtyRef.current) void savePromptDraft(pendingDraft, pendingVersion);
    promptSaveVersionRef.current += 1;
    promptDirtyRef.current = false;
    setCreating(true);
    try {
      const raw = await invokeCommand<unknown>("create_prompt");
      const next = decodePrompt(raw);
      if (!next) throw new Error("提示词命令返回了无法识别的结果。");
      promptSaveVersionRef.current += 1;
      promptDirtyRef.current = false;
      pendingSelectionIdRef.current = next.id;
      promptDraftRef.current = { selectedId: next.id, name: next.name, content: next.content, sourceLanguage: next.sourceLanguage, targetLanguage: next.targetLanguage };
      setSelectedId(next.id);
      setName(next.name);
      setContent(next.content);
      setSourceLanguage(next.sourceLanguage);
      setTargetLanguage(next.targetLanguage);
      await onChanged();
      onToast(`已创建${next.name}`);
    } catch (reason) {
      onToast(describeError(reason, "创建提示词失败"), "error");
    } finally {
      setCreating(false);
    }
  };

  const setDefault = async (prompt: Prompt) => {
    try {
      await invokeCommand("set_default_prompt", { id: prompt.id });
      await onChanged();
      onToast(`已将${prompt.name}设为默认提示词`);
    } catch (reason) {
      onToast(describeError(reason, "设置默认提示词失败"), "error");
    }
  };

  const remove = async (prompt: Prompt) => {
    if (prompt.isBuiltin || prompt.id === currentPromptId) return;
    if (!window.confirm(`确定删除提示词「${prompt.name}」吗？`)) return;
    try {
      await invokeCommand("delete_prompt", { id: prompt.id });
      await onChanged();
      setSelectedId(currentPromptId);
      onToast("提示词已删除");
    } catch (reason) {
      onToast(describeError(reason, "删除提示词失败"), "error");
    }
  };

  return (
    <div className="prompt-manager-card">
      <div className="card-heading settings-section-heading"><div><strong>提示词</strong></div></div>
      <div className="prompt-manager-grid">
        <div className="prompt-list">
          {prompts.map((prompt) => (
            <div className={`prompt-list-item ${prompt.id === selectedId ? "is-active" : ""} ${prompt.id === currentPromptId ? "is-default" : ""}`} key={prompt.id}>
              <button className="prompt-list-select" type="button" onClick={() => selectPrompt(prompt)} aria-pressed={prompt.id === selectedId}>
                <span><strong>{prompt.name}</strong></span>
              </button>
              <div className="prompt-list-actions">
                <button
                  className="icon-button prompt-icon-button prompt-default-button"
                  type="button"
                  onClick={() => void setDefault(prompt)}
                  disabled={prompt.id === currentPromptId}
                  title={prompt.id === currentPromptId ? "当前已是默认提示词" : `将${prompt.name}设为默认提示词`}
                  aria-label={prompt.id === currentPromptId ? "当前已是默认提示词" : `将${prompt.name}设为默认提示词`}
                >
                  <Check size={14} strokeWidth={2.2} aria-hidden="true" />
                </button>
                <button
                  className="icon-button danger-icon-button prompt-delete-button"
                  type="button"
                  onClick={() => void remove(prompt)}
                  disabled={prompt.isBuiltin || prompt.id === currentPromptId}
                  title={prompt.id === currentPromptId ? "当前默认提示词不可删除" : prompt.isBuiltin ? "内置提示词不可删除" : `删除${prompt.name}`}
                  aria-label={prompt.id === currentPromptId ? "当前默认提示词不可删除" : prompt.isBuiltin ? "内置提示词不可删除" : `删除${prompt.name}`}
                >
                  <Trash2 size={14} aria-hidden="true" />
                </button>
              </div>
            </div>
          ))}
          <button className="prompt-list-item prompt-list-create" type="button" onClick={() => void createPrompt()} disabled={creating}>
            <span><strong>新增自定义提示词</strong><small>创建空白提示词</small></span>
            <em aria-hidden="true">＋</em>
          </button>
        </div>
        <div className="prompt-editor-panel">
          <label>名称<input value={name} onChange={(event) => updatePromptDraft({ name: event.target.value })} disabled={!selectedId || creating} /></label>
          <div className="prompt-language-fields">
            <SettingsSelectField
              label="源语言"
              id={`prompt-source-language-${selectedId ?? "empty"}`}
              ariaLabel="提示词源语言"
              value={sourceLanguage}
              options={SOURCE_LANGUAGE_OPTIONS.map(([label, value]) => ({ label, value }))}
              onChange={(value) => updatePromptDraft({ sourceLanguage: value })}
              disabled={!selectedId || creating}
            />
            <SettingsSelectField
              label="目标语言"
              id={`prompt-target-language-${selectedId ?? "empty"}`}
              ariaLabel="提示词目标语言"
              value={targetLanguage}
              options={LANGUAGE_OPTIONS.map(([label, value]) => ({ label, value }))}
              onChange={(value) => updatePromptDraft({ targetLanguage: value })}
              disabled={!selectedId || creating}
            />
          </div>
          <label>内容<textarea className="prompt-editor" value={content} onChange={(event) => updatePromptDraft({ content: event.target.value })} disabled={!selectedId || creating} /></label>
        </div>
      </div>
    </div>
  );
}

interface SettingsSelectOption {
  value: string;
  label: string;
}

interface SettingsSelectProps {
  id: string;
  ariaLabel: string;
  value: string;
  options: SettingsSelectOption[];
  onChange: (value: string) => void;
  disabled?: boolean;
}

function SettingsSelect({
  id,
  ariaLabel,
  value,
  options,
  onChange,
  disabled = false,
}: SettingsSelectProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const buttonRef = useRef<HTMLButtonElement | null>(null);
  const optionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const [open, setOpen] = useState(false);
  const selectedIndex = Math.max(0, options.findIndex((option) => option.value === value));
  const [highlightedIndex, setHighlightedIndex] = useState(selectedIndex);
  const selectedOption = options[selectedIndex];

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

  useEffect(() => {
    if (disabled) setOpen(false);
  }, [disabled]);

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
    if ((event.key === "ArrowDown" || event.key === "ArrowUp") && options.length > 0) {
      event.preventDefault();
      const direction = event.key === "ArrowDown" ? 1 : -1;
      const nextIndex = open
        ? (highlightedIndex + direction + options.length) % options.length
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
      focusOption((index + direction + options.length) % options.length);
      return;
    }
    if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      focusOption(event.key === "Home" ? 0 : options.length - 1);
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
    <div className="settings-select-control" ref={containerRef}>
      <button
        className="settings-select-button"
        ref={buttonRef}
        id={id}
        type="button"
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={`${id}-menu`}
        disabled={disabled}
        onClick={() => setOpen((current) => !current)}
        onKeyDown={handleButtonKeyDown}
      >
        <span className="settings-select-button-label">{selectedOption?.label ?? value}</span>
        <ChevronDown className="settings-select-button-icon" size={15} strokeWidth={1.8} aria-hidden="true" />
      </button>
      {open && options.length > 0 && (
        <div className="settings-select-menu" id={`${id}-menu`} role="listbox" aria-label={ariaLabel}>
          {options.map((option, index) => (
            <button
              className={`settings-select-option ${index === selectedIndex ? "is-selected" : ""} ${index === highlightedIndex ? "is-highlighted" : ""}`}
              key={option.value}
              ref={(element) => { optionRefs.current[index] = element; }}
              type="button"
              role="option"
              aria-selected={index === selectedIndex}
              onMouseEnter={() => setHighlightedIndex(index)}
              onClick={() => selectValue(option.value)}
              onKeyDown={(event) => handleOptionKeyDown(event, index, option.value)}
            >
              <span>{option.label}</span>
              {index === selectedIndex && <Check size={14} strokeWidth={2} aria-hidden="true" />}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function SettingsSelectField({ label, ...props }: SettingsSelectProps & { label: string }) {
  return (
    <div className="settings-select-field">
      <span className="settings-select-label">{label}</span>
      <SettingsSelect {...props} />
    </div>
  );
}

function DictionarySettingsPanel({
  snapshot,
  dictionaryProgress,
  onDictionaryUpdate,
}: {
  snapshot: AppSnapshot;
  dictionaryProgress: DictionaryProgress | null;
  onDictionaryUpdate: () => Promise<void>;
}) {
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

  return (
    <div className="dictionary-settings-panel">
      <div className="settings-resource-status-row">
        <span className={`connection-status ${snapshot.dictionary.status === "ready" ? "connected" : ""}`}>{dictionaryStatusLabel}</span>
      </div>
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
      <div className="form-actions settings-actions"><span className="muted-text">数据版本 {snapshot.dictionary.distributionSchemaVersion ?? "—"} · SQLite {snapshot.dictionary.sqliteSchemaVersion ?? "—"}</span><button className="secondary-button" type="button" onClick={() => void onDictionaryUpdate()} disabled={dictionaryUpdating}>{dictionaryUpdating ? <LoaderCircle className="spin" size={15} /> : <BookOpen size={15} />}{snapshot.dictionary.status === "ready" ? "手动更新" : "下载词典"}</button></div>
    </div>
  );
}

function SettingsResourceModal({
  kind,
  open,
  onRequestClose,
  onClosed,
  children,
}: {
  kind: SettingsResourceModal;
  open: boolean;
  onRequestClose: () => void;
  onClosed: () => void;
  children: ReactNode;
}) {
  const dialogRef = useRef<HTMLDivElement | null>(null);
  const title = kind === "dictionary" ? "本地词典" : "PDF Engine";
  const titleId = `settings-${kind}-dialog-title`;

  useEffect(() => {
    if (!open) return;
    const frame = window.requestAnimationFrame(() => dialogRef.current?.focus());
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      onRequestClose();
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      window.cancelAnimationFrame(frame);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [onRequestClose, open]);

  return (
    <AnimatedOverlay
      className="modal-backdrop settings-resource-backdrop"
      open={open}
      onClosed={onClosed}
      onBackdropClick={(event) => {
        if (event.target === event.currentTarget) onRequestClose();
      }}
    >
      <div className="modal-card settings-resource-card" ref={dialogRef} role="dialog" aria-modal="true" aria-labelledby={titleId} tabIndex={-1}>
        <div className="modal-heading settings-resource-heading">
          <strong id={titleId}>{title}</strong>
          <button className="icon-button" type="button" onClick={onRequestClose} aria-label={`关闭${title}`} title={`关闭${title}`}><X size={17} /></button>
        </div>
        <div className="settings-resource-modal-body">{children}</div>
      </div>
    </AnimatedOverlay>
  );
}

function SettingsView({
  snapshot,
  dictionaryProgress,
  dictionaryEventsError,
  pdfEngine,
  navigationTarget,
  onDictionaryUpdate,
  onSaved,
  onToast,
  onCheckForUpdates,
  releaseCheckPending,
}: {
  snapshot: AppSnapshot;
  dictionaryProgress: DictionaryProgress | null;
  dictionaryEventsError: string | null;
  pdfEngine: PdfEngineRuntime;
  navigationTarget: SettingsNavigationTarget | null;
  onDictionaryUpdate: () => Promise<void>;
  onSaved: (snapshot: AppSnapshot) => void;
  onToast: ShowToast;
  onCheckForUpdates: () => void;
  releaseCheckPending: boolean;
}) {
  const [activeSection, setActiveSection] = useState<SettingsSectionId>("provider");
  const settingsScrollRef = useRef<HTMLDivElement | null>(null);
  const settingsSectionRefs = useRef<Partial<Record<SettingsSectionId, HTMLDivElement | null>>>({});
  const aboutDictionaryRef = useRef<HTMLButtonElement | null>(null);
  const aboutPdfEngineRef = useRef<HTMLButtonElement | null>(null);
  const [resourceModal, setResourceModal] = useState<SettingsResourceModal | null>(null);
  const [resourceModalOpen, setResourceModalOpen] = useState(false);
  const [resourceModalMounted, setResourceModalMounted] = useState(false);
  const resourceModalReturnFocusRef = useRef<HTMLElement | null>(null);
  const [baseUrl, setBaseUrl] = useState(snapshot.provider.baseUrl);
  const [modelId, setModelId] = useState(snapshot.provider.modelId);
  const [thinkingEffort, setThinkingEffort] = useState<ThinkingEffort>(snapshot.provider.thinkingEffort ?? "none");
  const [availableModels, setAvailableModels] = useState<ModelInfo[] | null>(null);
  const [apiKey, setApiKey] = useState("");
  const [settings, setSettings] = useState<AppSettings>(snapshot.settings);
  const [selectionMode, setSelectionMode] = useState(snapshot.settings.selectionMode);
  const [selectionShortcut, setSelectionShortcut] = useState(snapshot.settings.selectionShortcut);
  const [selectionShortcutListening, setSelectionShortcutListening] = useState(false);
  const [selectionStatus, setSelectionStatus] = useState<SelectionRuntimeStatus | null>(null);
  const [cacheClearing, setCacheClearing] = useState(false);
  const selectionShortcutBeforeListeningRef = useRef(snapshot.settings.selectionShortcut);
  const selectionShortcutListeningRef = useRef(false);
  const settingsDraftRef = useRef<SettingsDraft>(createSettingsDraft(snapshot));
  const saveTimerRef = useRef<number | null>(null);
  const saveVersionRef = useRef(0);
  const savingRef = useRef(false);
  const disposedRef = useRef(false);
  const saveDraft = useCallback(async (draft: SettingsDraft, version: number) => {
    if (savingRef.current) return;
    savingRef.current = true;
    try {
      const shortcut = draft.selectionShortcut.trim();
      if (!draft.modelId.trim()) throw new Error("Model ID 不能为空");
      if (!shortcut) throw new Error("快捷键不能为空");
      await invokeCommand("save_provider_config", {
        baseUrl: draft.baseUrl,
        modelId: draft.modelId,
        thinkingEffort: draft.thinkingEffort,
        apiKey: draft.apiKey || null,
      });
      await invokeCommand("save_app_settings", {
        historyRetention: draft.settings.historyRetention,
        cacheEnabled: draft.settings.cacheEnabled,
        cacheMaxBytes: draft.settings.cacheMaxBytes,
        wordAiCacheEnabled: draft.settings.wordAiCacheEnabled,
        paragraphExampleLookupEnabled: draft.settings.paragraphExampleLookupEnabled,
        pdfPreflightPageLimit: draft.settings.pdfPreflightPageLimit,
      });
      await invokeCommand("configure_selection", {
        mode: draft.selectionMode,
        shortcut,
      });
      const rawStatus = await invokeCommand<unknown>("get_selection_status");
      const nextStatus = decodeSelectionStatus(rawStatus);
      if (!disposedRef.current && nextStatus) setSelectionStatus(nextStatus);
      const next = await invokeCommand<AppSnapshot>("get_app_snapshot");
      if (version === saveVersionRef.current) {
        const nextDraft = { ...settingsDraftRef.current, apiKey: "" };
        settingsDraftRef.current = nextDraft;
        if (!disposedRef.current) {
          setApiKey("");
        }
        onSaved(next);
      }
    } catch (reason) {
      if (version === saveVersionRef.current && !disposedRef.current) {
        onToast(describeError(reason, "设置自动保存失败"), "error");
      }
    } finally {
      savingRef.current = false;
      if (version !== saveVersionRef.current) {
        void saveDraft(settingsDraftRef.current, saveVersionRef.current);
      }
    }
  }, [onSaved, onToast]);

  const scheduleSave = useCallback((nextDraft: SettingsDraft) => {
    settingsDraftRef.current = nextDraft;
    saveVersionRef.current += 1;
    if (saveTimerRef.current !== null) window.clearTimeout(saveTimerRef.current);
    const version = saveVersionRef.current;
    saveTimerRef.current = window.setTimeout(() => {
      saveTimerRef.current = null;
      void saveDraft(nextDraft, version);
    }, 650);
  }, [saveDraft]);

  const updateDraft = useCallback((update: (current: SettingsDraft) => SettingsDraft) => {
    const nextDraft = update(settingsDraftRef.current);
    settingsDraftRef.current = nextDraft;
    setBaseUrl(nextDraft.baseUrl);
    setModelId(nextDraft.modelId);
    setThinkingEffort(nextDraft.thinkingEffort);
    setApiKey(nextDraft.apiKey);
    setSettings(nextDraft.settings);
    setSelectionMode(nextDraft.selectionMode);
    setSelectionShortcut(nextDraft.selectionShortcut);
    scheduleSave(nextDraft);
  }, [scheduleSave]);

  const updateProviderDraft = useCallback((patch: Partial<Pick<SettingsDraft, "baseUrl" | "modelId" | "thinkingEffort" | "apiKey">>) => {
    updateDraft((current) => ({ ...current, ...patch }));
  }, [updateDraft]);

  const updateAppSettingsDraft = useCallback((patch: Partial<AppSettings>) => {
    updateDraft((current) => ({ ...current, settings: { ...current.settings, ...patch } }));
  }, [updateDraft]);

  const updateSelectionDraft = useCallback((patch: Partial<Pick<SettingsDraft, "selectionMode" | "selectionShortcut">>) => {
    updateDraft((current) => ({ ...current, ...patch }));
  }, [updateDraft]);

  const clearCache = useCallback(async () => {
    if (cacheClearing) return;
    setCacheClearing(true);
    try {
      await invokeCommand("clear_cache");
      const next = await invokeCommand<AppSnapshot>("get_app_snapshot");
      onSaved(next);
      onToast("缓存已清空");
    } catch (reason) {
      onToast(describeError(reason, "清空缓存失败"), "error");
    } finally {
      setCacheClearing(false);
    }
  }, [cacheClearing, onSaved, onToast]);

  const startSelectionShortcutListening = useCallback(() => {
    selectionShortcutBeforeListeningRef.current = selectionShortcut;
    selectionShortcutListeningRef.current = true;
    setSelectionShortcutListening(true);
    setSelectionShortcut("");
  }, [selectionShortcut]);

  const cancelSelectionShortcutListening = useCallback(() => {
    if (!selectionShortcutListeningRef.current) return;
    selectionShortcutListeningRef.current = false;
    setSelectionShortcutListening(false);
    setSelectionShortcut(selectionShortcutBeforeListeningRef.current);
  }, []);

  const handleSelectionShortcutKeyDown = useCallback((event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (!selectionShortcutListeningRef.current) return;
    if (event.key === "Escape") {
      event.preventDefault();
      cancelSelectionShortcutListening();
      event.currentTarget.blur();
      return;
    }
    if (event.key === "Tab") return;

    event.preventDefault();
    const nextShortcut = formatSelectionShortcut(event);
    if (!nextShortcut) return;

    selectionShortcutListeningRef.current = false;
    setSelectionShortcutListening(false);
    updateSelectionDraft({ selectionShortcut: nextShortcut });
  }, [cancelSelectionShortcutListening, updateSelectionDraft]);

  const handleSelectionModeKeyDown = useCallback((event: ReactKeyboardEvent<HTMLButtonElement>) => {
    if (!["ArrowDown", "ArrowRight", "ArrowUp", "ArrowLeft", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const currentValue = event.currentTarget.dataset.selectionMode as SelectionModeOption | undefined;
    const currentIndex = SELECTION_MODE_OPTIONS.findIndex((option) => option.value === currentValue);
    const nextIndex = event.key === "Home"
      ? 0
      : event.key === "End"
        ? SELECTION_MODE_OPTIONS.length - 1
        : (currentIndex + (["ArrowUp", "ArrowLeft"].includes(event.key) ? -1 : 1) + SELECTION_MODE_OPTIONS.length) % SELECTION_MODE_OPTIONS.length;
    const nextOption = SELECTION_MODE_OPTIONS[nextIndex];
    if (!nextOption) return;
    event.currentTarget.parentElement?.querySelector<HTMLButtonElement>(`[data-selection-mode="${nextOption.value}"]`)?.focus();
  }, []);

  const openResourceModal = useCallback((kind: SettingsResourceModal, returnFocusElement?: HTMLElement | null) => {
    resourceModalReturnFocusRef.current = returnFocusElement
      ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null);
    setResourceModal(kind);
    setResourceModalMounted(true);
    setResourceModalOpen(true);
  }, []);

  const requestResourceModalClose = useCallback(() => {
    setResourceModalOpen(false);
  }, []);

  const handleResourceModalClosed = useCallback(() => {
    setResourceModalMounted(false);
    setResourceModal(null);
    const previouslyFocused = resourceModalReturnFocusRef.current;
    resourceModalReturnFocusRef.current = null;
    previouslyFocused?.focus();
  }, []);

  const scrollToSection = useCallback((sectionId: SettingsSectionId) => {
    const scrollElement = settingsScrollRef.current;
    const sectionElement = settingsSectionRefs.current[sectionId];
    if (!scrollElement || !sectionElement) return;
    const scrollRect = scrollElement.getBoundingClientRect();
    const sectionRect = sectionElement.getBoundingClientRect();
    const top = scrollElement.scrollTop + sectionRect.top - scrollRect.top - SETTINGS_CHROME_HEIGHT;
    setActiveSection(sectionId);
    scrollElement.scrollTo({ top: Math.max(0, top), behavior: "smooth" });
  }, []);

  useEffect(() => {
    if (!navigationTarget) return undefined;
    const frame = window.requestAnimationFrame(() => {
      const scrollElement = settingsScrollRef.current;
      const targetElement = navigationTarget.anchor === "dictionary-version"
        ? aboutDictionaryRef.current
        : navigationTarget.anchor === "pdf-engine"
          ? aboutPdfEngineRef.current
          : settingsSectionRefs.current[navigationTarget.sectionId];
      if (!scrollElement || !targetElement) return;
      const scrollRect = scrollElement.getBoundingClientRect();
      const targetRect = targetElement.getBoundingClientRect();
      const top = scrollElement.scrollTop + targetRect.top - scrollRect.top - SETTINGS_CHROME_HEIGHT;
      setActiveSection(navigationTarget.sectionId);
      scrollElement.scrollTo({ top: Math.max(0, top), behavior: "smooth" });
      if (navigationTarget.modal) openResourceModal(navigationTarget.modal, targetElement);
    });
    return () => window.cancelAnimationFrame(frame);
  }, [navigationTarget, openResourceModal]);

  useEffect(() => {
    const scrollElement = settingsScrollRef.current;
    if (!scrollElement) return;
    let frame: number | null = null;

    const syncActiveSection = () => {
      frame = null;
      const scrollRect = scrollElement.getBoundingClientRect();
      const anchorTop = scrollRect.top + SETTINGS_CHROME_HEIGHT + 4;
      let nextSection: SettingsSectionId = "provider";
      let hasPassedSection = false;

      for (const section of SETTINGS_SECTIONS) {
        const element = settingsSectionRefs.current[section.id];
        if (!element) continue;
        if (element.getBoundingClientRect().top <= anchorTop) {
          nextSection = section.id;
          hasPassedSection = true;
        } else if (!hasPassedSection) {
          break;
        }
      }

      if (scrollElement.scrollTop + scrollElement.clientHeight >= scrollElement.scrollHeight - 2) {
        const lastSection = SETTINGS_SECTIONS[SETTINGS_SECTIONS.length - 1];
        if (lastSection) nextSection = lastSection.id;
      }

      setActiveSection((current) => current === nextSection ? current : nextSection);
    };

    const requestSync = () => {
      if (frame !== null) return;
      frame = window.requestAnimationFrame(syncActiveSection);
    };

    scrollElement.addEventListener("scroll", requestSync, { passive: true });
    window.addEventListener("resize", requestSync);
    requestSync();
    return () => {
      scrollElement.removeEventListener("scroll", requestSync);
      window.removeEventListener("resize", requestSync);
      if (frame !== null) window.cancelAnimationFrame(frame);
    };
  }, []);

  useEffect(() => {
    return () => {
      disposedRef.current = true;
      if (saveTimerRef.current !== null) {
        window.clearTimeout(saveTimerRef.current);
        saveTimerRef.current = null;
        void saveDraft(settingsDraftRef.current, saveVersionRef.current);
      }
    };
  }, [saveDraft]);

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
  }, [selectionMode, selectionShortcut]);

  useEffect(() => {
    if (selectionStatus?.message) onToast(selectionStatus.message, "error");
  }, [onToast, selectionStatus?.message]);

  useEffect(() => {
    if (dictionaryEventsError) onToast(dictionaryEventsError, "error");
  }, [dictionaryEventsError, onToast]);

  useEffect(() => {
    if (snapshot.dictionary.error && snapshot.dictionary.status === "failed") {
      onToast(snapshot.dictionary.error, "error");
    }
  }, [onToast, snapshot.dictionary.error, snapshot.dictionary.status]);

  useEffect(() => {
    if (resourceModal !== "pdf-engine") return;
    const engineError = pdfEngine.error ?? pdfEngine.eventsError;
    if (engineError) onToast(engineError, "error");
  }, [onToast, pdfEngine.error, pdfEngine.eventsError, resourceModal]);

  const fetchModels = async () => {
    try {
      const models = await invokeCommand<ModelInfo[]>("fetch_models", {
        baseUrl: baseUrl.trim() || null,
        apiKey: apiKey.trim() || null,
      });
      setAvailableModels(models.length > 0 ? models : null);
      if (models.length > 0 && !models.some((model) => model.id === modelId)) {
        updateProviderDraft({ modelId: models[0]?.id ?? modelId });
      }
      onToast(`模型列表已更新，共 ${models.length} 个模型`);
    } catch (reason) {
      setAvailableModels(null);
      onToast(describeError(reason, "模型列表读取失败，可手动填写 Model ID"), "error");
    }
  };

  const refreshAfterPromptChange = useCallback(async () => {
    const next = await invokeCommand<AppSnapshot>("get_app_snapshot");
    onSaved(next);
  }, [onSaved]);

  const resetCloseBehavior = async () => {
    try {
      await invokeCommand("reset_close_behavior");
      const next = await invokeCommand<AppSnapshot>("get_app_snapshot");
      const nextDraft = { ...settingsDraftRef.current, settings: { ...settingsDraftRef.current.settings, closeBehavior: next.settings.closeBehavior } };
      settingsDraftRef.current = nextDraft;
      setSettings(nextDraft.settings);
      onSaved(next);
      onToast("关闭行为已恢复为每次询问");
    } catch (reason) {
      onToast(describeError(reason, "恢复关闭行为失败"), "error");
    }
  };

  const modelOptions = availableModels
    ? [
      ...(!availableModels.some((model) => model.id === modelId) ? [{ value: modelId, label: `当前：${modelId}` }] : []),
      ...availableModels.map((model) => ({ value: model.id, label: model.label })),
    ]
    : [];
  const cacheMaxMb = Math.min(MAX_CACHE_SIZE_MB, Math.max(MIN_CACHE_SIZE_MB, Math.round(settings.cacheMaxBytes / (1024 * 1024))));
  const cacheRangePercent = ((cacheMaxMb - MIN_CACHE_SIZE_MB) / (MAX_CACHE_SIZE_MB - MIN_CACHE_SIZE_MB)) * 100;
  return (
    <section className="settings-view" aria-labelledby="settings-workspace-title">
      <aside className="settings-sidebar">
        <div className="settings-sidebar-heading">
          <h1 id="settings-workspace-title">设置</h1>
        </div>
        <nav className="settings-navigation" aria-label="设置分类">
          {SETTINGS_SECTIONS.map((section) => {
            const Icon = section.icon;
            const selected = section.id === activeSection;
            return (
              <button
                className={`settings-navigation-item ${selected ? "is-active" : ""}`}
                type="button"
                key={section.id}
                aria-current={selected ? "page" : undefined}
                onClick={() => scrollToSection(section.id)}
              >
                <Icon size={16} strokeWidth={1.8} aria-hidden="true" />
                <span><strong>{section.label}</strong></span>
              </button>
            );
          })}
        </nav>
      </aside>

      <div className="settings-content">
        <div className="settings-content-scroll" ref={settingsScrollRef}>
          <div className="settings-section" id="settings-section-provider" data-settings-section="provider" ref={(element) => { settingsSectionRefs.current.provider = element; }}>
          <div className="card-heading"><div><strong>LLM提供商</strong></div></div>
          <div className="form-grid">
            <label className="wide-field">Base URL<input value={baseUrl} onChange={(event) => updateProviderDraft({ baseUrl: event.target.value })} placeholder="https://api.openai.com/v1" /></label>
            {availableModels ? <SettingsSelectField label="Model ID" id="settings-model-id" ariaLabel="模型 ID" value={modelId} options={modelOptions} onChange={(value) => updateProviderDraft({ modelId: value })} /> : <label>Model ID<input value={modelId} onChange={(event) => updateProviderDraft({ modelId: event.target.value })} placeholder="gpt-4o-mini" /></label>}
            <SettingsSelectField label="思考强度" id="settings-thinking-effort" ariaLabel="思考强度" value={thinkingEffort} options={[{ value: "none", label: "none" }, { value: "low", label: "low" }, { value: "medium", label: "medium" }, { value: "high", label: "high" }]} onChange={(value) => updateProviderDraft({ thinkingEffort: value as ThinkingEffort })} />
            <label className="wide-field">API Key<input type="password" value={apiKey} onChange={(event) => updateProviderDraft({ apiKey: event.target.value })} placeholder={snapshot.provider.hasApiKey ? "已保存，留空表示不修改" : "保存在 Windows 凭据管理器"} autoComplete="off" /></label>
          </div>
          <div className="form-actions settings-actions"><button className="secondary-button" type="button" onClick={() => void fetchModels()}>读取模型</button></div>
          </div>

          <div className="settings-section" id="settings-section-prompt" data-settings-section="prompt" ref={(element) => { settingsSectionRefs.current.prompt = element; }}>
            <PromptManager prompts={snapshot.prompts} currentPromptId={snapshot.provider.promptId} onChanged={refreshAfterPromptChange} onToast={onToast} />
          </div>

        <div className="settings-section" id="settings-section-selection" data-settings-section="selection" ref={(element) => { settingsSectionRefs.current.selection = element; }}>
          <div className="card-heading"><div><strong>划词翻译</strong></div></div>
          <div className="form-grid selection-settings-grid">
            <div className="selection-mode-field">
              <span className="settings-select-label" id="settings-selection-mode-label">触发方式</span>
              <div className="selection-mode-options" role="group" aria-labelledby="settings-selection-mode-label">
                {SELECTION_MODE_OPTIONS.map((option) => {
                  const selected = isSelectionModeEnabled(selectionMode, option.value);
                  return (
                    <button
                      className={`selection-mode-card ${selected ? "is-selected" : ""}`}
                      type="button"
                      key={option.value}
                      aria-pressed={selected}
                      tabIndex={0}
                      data-selection-mode={option.value}
                      onClick={() => updateSelectionDraft({ selectionMode: toggleSelectionMode(selectionMode, option.value) })}
                      onKeyDown={handleSelectionModeKeyDown}
                    >
                      <span className="selection-mode-card-copy"><strong>{option.label}</strong></span>
                    </button>
                  );
                })}
              </div>
            </div>
            <label className={`selection-shortcut-field ${selectionShortcutListening ? "is-listening" : ""}`}>
              <span>快捷键</span>
              <input
                className="selection-shortcut-input"
                value={selectionShortcut}
                readOnly
                onFocus={startSelectionShortcutListening}
                onBlur={cancelSelectionShortcutListening}
                onKeyDown={handleSelectionShortcutKeyDown}
                placeholder="按任意键/组合键"
                aria-label="快捷键"
              />
            </label>
          </div>
        </div>

        <div className="settings-section" id="settings-section-pdf" data-settings-section="pdf" ref={(element) => { settingsSectionRefs.current.pdf = element; }}>
          <div className="card-heading"><div><strong>PDF 全文翻译</strong></div></div>
          <label className="setting-line">
            <span><strong>预检读取页数</strong></span>
            <input
              className="number-input"
              type="number"
              min={1}
              max={100}
              value={settings.pdfPreflightPageLimit}
              onChange={(event) => {
                const value = Number(event.target.value);
                updateAppSettingsDraft({
                  pdfPreflightPageLimit: Number.isFinite(value) ? Math.min(100, Math.max(1, Math.round(value))) : 1,
                });
              }}
              aria-label="PDF 预检读取页数"
            />
          </label>
        </div>

        <div className="settings-section" id="settings-section-local" data-settings-section="local" ref={(element) => { settingsSectionRefs.current.local = element; }}>
          <div className="card-heading"><div><strong>本地数据</strong></div></div>
          <label className="setting-line"><span><strong>翻译历史保留条数</strong></span><input className="number-input" type="number" min={1} max={1000} value={settings.historyRetention} onChange={(event) => updateAppSettingsDraft({ historyRetention: Number(event.target.value) })} /></label>
          <label className={`setting-line settings-switch-line ${settings.cacheEnabled ? "is-enabled" : ""}`}>
            <span><strong>启用段落翻译缓存</strong></span>
            <span className="settings-switch">
              <input className="settings-switch-input" type="checkbox" checked={settings.cacheEnabled} onChange={(event) => updateAppSettingsDraft({ cacheEnabled: event.target.checked })} aria-label="启用段落翻译缓存" />
              <span className="settings-switch-track" aria-hidden="true"><span /></span>
            </span>
          </label>
          <label className={`setting-line settings-switch-line ${settings.wordAiCacheEnabled ? "is-enabled" : ""}`}>
            <span><strong>缓存单词 AI 见解</strong></span>
            <span className="settings-switch">
              <input className="settings-switch-input" type="checkbox" checked={settings.wordAiCacheEnabled} onChange={(event) => updateAppSettingsDraft({ wordAiCacheEnabled: event.target.checked })} aria-label="缓存单词 AI 见解" />
              <span className="settings-switch-track" aria-hidden="true"><span /></span>
            </span>
          </label>
          <label className={`setting-line settings-switch-line ${settings.paragraphExampleLookupEnabled ? "is-enabled" : ""}`}>
            <span><strong>从段落缓存查找例句</strong></span>
            <span className="settings-switch">
              <input className="settings-switch-input" type="checkbox" checked={settings.paragraphExampleLookupEnabled} onChange={(event) => updateAppSettingsDraft({ paragraphExampleLookupEnabled: event.target.checked })} aria-label="从段落缓存查找例句" />
              <span className="settings-switch-track" aria-hidden="true"><span /></span>
            </span>
          </label>
          <label className="setting-line slider-line"><span><strong>段落缓存上限</strong></span><span className="settings-range-control"><input className="settings-range-input" type="range" min={MIN_CACHE_SIZE_MB} max={MAX_CACHE_SIZE_MB} step={16} value={cacheMaxMb} onChange={(event) => updateAppSettingsDraft({ cacheMaxBytes: Number(event.target.value) * 1024 * 1024 })} aria-label="段落缓存上限" aria-valuetext={`${cacheMaxMb} MB`} /><output className="settings-range-value" style={{ left: `${cacheRangePercent}%` }}>{cacheMaxMb} MB</output></span></label>
          <div className="setting-line cache-summary-line">
            <span>
              <strong>当前缓存</strong>
              <small>{formatBytes(snapshot.cacheStats.usageBytes)} · {snapshot.cacheStats.entryCount.toLocaleString("zh-CN")} 条</small>
            </span>
            <button className="secondary-button" type="button" onClick={() => void clearCache()} disabled={cacheClearing || (snapshot.cacheStats.usageBytes === 0 && snapshot.cacheStats.entryCount === 0)}>
              {cacheClearing ? "清空中…" : "清空缓存"}
            </button>
          </div>
        </div>

        <div className="settings-section" id="settings-section-behavior" data-settings-section="behavior" ref={(element) => { settingsSectionRefs.current.behavior = element; }}>
          <div className="card-heading"><div><strong>关闭行为</strong></div></div>
          <div className="setting-line"><span><strong>{settings.closeBehavior === "ask" ? "每次询问" : settings.closeBehavior === "tray" ? "缩小到系统托盘" : "退出程序"}</strong></span><button className="secondary-button" type="button" onClick={() => void resetCloseBehavior()} disabled={settings.closeBehavior === "ask"}>恢复每次询问</button></div>
        </div>

        <div className="settings-section settings-about-section" id="settings-section-about" data-settings-section="about" ref={(element) => { settingsSectionRefs.current.about = element; }}>
          <div className="card-heading"><div><strong>关于</strong></div></div>
          <div className="about-info-list">
            <div className="about-version-group">
              <div className="about-info-row"><span>程序版本</span><button className="about-version-button" type="button" onClick={onCheckForUpdates} disabled={releaseCheckPending} aria-busy={releaseCheckPending} aria-label={releaseCheckPending ? "正在检查更新" : "检查更新"} title={releaseCheckPending ? "正在检查更新" : "点击检查更新"}>v{APP_VERSION}</button></div>
              <div className="about-info-row"><span>本地词典版本</span><button className="about-version-button" id="settings-about-dictionary-version" ref={aboutDictionaryRef} type="button" onClick={() => openResourceModal("dictionary")} aria-haspopup="dialog" aria-label="打开本地词典准备弹窗">{snapshot.dictionary.installedRelease ?? "未安装"}</button></div>
              <div className="about-info-row"><span>PDF Engine 版本</span><button className="about-version-button" id="settings-about-pdf-engine" ref={aboutPdfEngineRef} type="button" onClick={() => openResourceModal("pdf-engine")} aria-haspopup="dialog" aria-label="打开 PDF Engine 准备弹窗">{pdfEngine.status?.engineVersion ?? (pdfEngine.statusLoading ? "检查中" : "—")}</button></div>
            </div>
            <div className="about-info-row"><span>Github</span><a href={GITHUB_URL} target="_blank" rel="noreferrer" onClick={(event) => { event.preventDefault(); openExternalUrl(GITHUB_URL); }}>TTTTTony32/Lilt</a></div>
            <div className="about-info-row"><span>开发者</span><span>Tony32 · <a href={`mailto:${DEVELOPER_EMAIL}`}>{DEVELOPER_EMAIL}</a></span></div>
          </div>
        </div>
        </div>
      </div>
      {resourceModalMounted && resourceModal && (
        <SettingsResourceModal
          kind={resourceModal}
          open={resourceModalOpen}
          onRequestClose={requestResourceModalClose}
          onClosed={handleResourceModalClosed}
        >
          {resourceModal === "dictionary" ? (
            <DictionarySettingsPanel
              snapshot={snapshot}
              dictionaryProgress={dictionaryProgress}
              onDictionaryUpdate={onDictionaryUpdate}
            />
          ) : (
            <PdfEnginePanel
              engineStatus={pdfEngine.status}
              engineStatusLoading={pdfEngine.statusLoading}
              enginePreparing={pdfEngine.preparing}
              engineProgress={pdfEngine.progress}
              onPrepareEngine={() => void pdfEngine.prepare()}
            />
          )}
        </SettingsResourceModal>
      )}
    </section>
  );
}

function PageTitle({ eyebrow, title, description, actions }: { eyebrow: string; title: string; description?: string; actions?: ReactNode }) {
  return <div className="page-heading"><div><p className="eyebrow">{eyebrow}</p><h1>{title}</h1>{description && <p className="page-description">{description}</p>}</div>{actions}</div>;
}

export default App;
