import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { FileType2, Upload } from "lucide-react";
import { describeError } from "./lib/errors";
import { type PdfFile, validatePdfPath } from "./lib/pdf";
import { invokeCommand, listenTo } from "./lib/tauri";
import type { ResourceDownloadPromptRequest } from "./lib/download-activity";
import type { FloatingProgressCardData } from "./components/FloatingProgressCard";
import type { PdfEngineRuntime } from "./lib/usePdfEngineRuntime";
import {
  createEmptyPdfPreflightState,
  markPdfPreflightRunning,
  mergePdfPreflightState,
  reducePdfPreflightEvent,
  reducePdfPreflightWarning,
} from "./lib/pdf-preflight";
import { appendPdfJobLogMessage, reducePdfJobLog } from "./lib/pdf-job-log";
import {
  formatPdfTaskProgressDetail,
  isPdfJobBusy,
  progressPercent,
} from "./lib/pdf-task-presentation";
import type { PdfPreflightSample } from "./lib/pdf-reader-utils";
import {
  PDF_JOB_EVENT_NAMES,
  decodePdfJobEvent,
  decodePdfTranslationCancelResult,
  decodePdfTranslationStartResult,
  type PdfQualityDiagnostic,
  type PdfJobEvent,
  type PdfJobUiState,
} from "./types/contracts";

const PdfReader = lazy(() => import("./PdfReader"));

const MULTIPLE_FILES_ERROR = "当前只支持单个 PDF 文件，请一次拖放一个文件。";
const INVALID_FILE_ERROR = "请选择 PDF 文件，文件扩展名必须为 .pdf。";
const EMPTY_PATH_ERROR = "未找到 PDF 文件路径，请重试。";
const PDF_TRANSLATION_START_TIMEOUT_MS = 30_000;
const PDF_TRANSLATION_CANCEL_TIMEOUT_MS = 10_000;

function emptyPdfJob(): PdfJobUiState {
  return {
    taskId: null,
    status: "idle",
    stage: null,
    progress: null,
    workerVersion: null,
    outputPdf: null,
    outputMode: null,
    pageCount: null,
    warnings: [],
    tokenUsage: null,
    code: null,
    message: null,
    logs: [],
    preflight: createEmptyPdfPreflightState(),
    documentContext: null,
    diagnostics: [],
  };
}

function appendUniqueStrings(current: string[], additions: string[]): string[] {
  return additions.reduce((result, item) => result.includes(item) ? result : [...result, item], current);
}

function diagnosticKey(diagnostic: PdfQualityDiagnostic): string {
  return [
    diagnostic.ruleId ?? "",
    diagnostic.message,
    diagnostic.segmentId ?? "",
    diagnostic.pageNumber ?? "",
    diagnostic.translationRequestId ?? "",
  ].join("|");
}

function appendUniqueDiagnostics(current: PdfQualityDiagnostic[], additions: PdfQualityDiagnostic[]): PdfQualityDiagnostic[] {
  const keys = new Set(current.map(diagnosticKey));
  return additions.reduce((result, diagnostic) => {
    const key = diagnosticKey(diagnostic);
    if (keys.has(key)) return result;
    keys.add(key);
    return [...result, diagnostic];
  }, current);
}

function mergePdfJobEventMetadata(current: PdfJobUiState, event: PdfJobEvent): PdfJobUiState {
  let next = current;
  const hasPreflightMetadata = "preflight" in event && Boolean(event.preflight);
  if (hasPreflightMetadata) {
    next = mergePdfPreflightState(next, event.preflight!);
  }
  if ("documentContext" in event && event.documentContext && (!hasPreflightMetadata || next !== current)) {
    next = { ...next, documentContext: event.documentContext };
  }
  if ("diagnostics" in event && event.diagnostics && (!hasPreflightMetadata || next !== current)) {
    next = { ...next, diagnostics: appendUniqueDiagnostics(next.diagnostics ?? [], event.diagnostics) };
  }
  return next;
}

function appendPdfJobEventLog(current: PdfJobUiState, event: PdfJobEvent): PdfJobUiState {
  const logs = reducePdfJobLog(current.logs, event);
  return logs === current.logs ? current : { ...current, logs };
}

function withTimeout<T>(promise: Promise<T>, timeoutMs: number, message: string): Promise<T> {
  let timeoutId: number | null = null;
  const timeoutPromise = new Promise<never>((_, reject) => {
    timeoutId = window.setTimeout(() => reject(new Error(message)), timeoutMs);
  });
  return Promise.race([promise, timeoutPromise]).finally(() => {
    if (timeoutId !== null) window.clearTimeout(timeoutId);
  });
}

interface PdfViewProps {
  active: boolean;
  pdfEngine: PdfEngineRuntime;
  pdfPreflightEnabled: boolean;
  pdfPreflightPageLimit: number;
  pdfPreflightSaving: boolean;
  onPdfPreflightEnabledChange: (enabled: boolean) => void;
  onResourceDownloadPrompt: (request: ResourceDownloadPromptRequest) => void;
  onOpenPdfEngineSettings: () => void;
  onPdfTranslationFloatingChange: (state: PdfTranslationFloatingState | null) => void;
}

export type PdfTranslationFloatingState = FloatingProgressCardData;

export default function PdfView({
  active,
  pdfEngine,
  pdfPreflightEnabled,
  pdfPreflightPageLimit,
  pdfPreflightSaving,
  onPdfPreflightEnabledChange,
  onResourceDownloadPrompt,
  onOpenPdfEngineSettings,
  onPdfTranslationFloatingChange,
}: PdfViewProps) {
  const [selectedFile, setSelectedFile] = useState<PdfFile | null>(null);
  const [dragging, setDragging] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const {
    status: engineStatus,
    statusLoading: engineStatusLoading,
    error: engineError,
  } = pdfEngine;
  const [jobEventsReady, setJobEventsReady] = useState(false);
  const [jobEventsError, setJobEventsError] = useState<string | null>(null);
  const [pdfJob, setPdfJob] = useState<PdfJobUiState>(() => emptyPdfJob());
  const pdfJobRef = useRef<PdfJobUiState>(emptyPdfJob());
  const disposedRef = useRef(false);
  const enginePromptShownRef = useRef(false);
  const startAttemptRef = useRef<number | null>(null);
  const startAttemptSequenceRef = useRef(0);
  const activeTaskIdRef = useRef<string | null>(null);
  const cancelTimeoutRef = useRef<number | null>(null);
  const taskbarProgressKeyRef = useRef<string | null>(null);

  const updatePdfJob = useCallback((updater: (current: PdfJobUiState) => PdfJobUiState) => {
    setPdfJob((current) => {
      const next = updater(current);
      pdfJobRef.current = next;
      return next;
    });
  }, []);

  const clearCancelTimeout = useCallback(() => {
    if (cancelTimeoutRef.current === null) return;
    window.clearTimeout(cancelTimeoutRef.current);
    cancelTimeoutRef.current = null;
  }, []);

  const clearPdfTaskRefs = useCallback(() => {
    activeTaskIdRef.current = null;
    startAttemptRef.current = null;
    clearCancelTimeout();
  }, [clearCancelTimeout]);

  const resetPdfJob = useCallback(() => {
    clearPdfTaskRefs();
    const next = emptyPdfJob();
    pdfJobRef.current = next;
    setPdfJob(next);
  }, [clearPdfTaskRefs]);

  const abandonPdfTask = useCallback(() => {
    const taskId = activeTaskIdRef.current;
    clearPdfTaskRefs();
    resetPdfJob();
    if (taskId) {
      void invokeCommand("cancel_pdf_translation", { taskId }).catch(() => undefined);
    }
  }, [clearPdfTaskRefs, resetPdfJob]);

  const acceptPath = useCallback((path: string | undefined) => {
    if (!path?.trim()) {
      setError(EMPTY_PATH_ERROR);
      return;
    }

    const file = validatePdfPath(path);
    if (!file) {
      setError(INVALID_FILE_ERROR);
      return;
    }

    abandonPdfTask();
    setSelectedFile(file);
    setError(null);
  }, [abandonPdfTask]);

  const handleDrop = useCallback((paths: string[]) => {
    setDragging(false);
    if (paths.length !== 1) {
      setError(MULTIPLE_FILES_ERROR);
      return;
    }
    acceptPath(paths[0]);
  }, [acceptPath]);

  const matchesPdfTask = useCallback((taskId: string): boolean => {
    const activeTaskId = activeTaskIdRef.current;
    if (activeTaskId) return activeTaskId === taskId;
    if (startAttemptRef.current !== null && pdfJobRef.current.status === "starting") {
      activeTaskIdRef.current = taskId;
      return true;
    }
    return false;
  }, []);

  const handlePdfJobEvent = useCallback((event: PdfJobEvent) => {
    if (disposedRef.current || !matchesPdfTask(event.taskId)) return;
    switch (event.type) {
      case "preflightStarted":
      case "preflightActivity":
      case "preflightCompleted":
      case "preflightDegraded":
      case "preflightFailed":
        updatePdfJob((current) => {
          const next = reducePdfPreflightEvent(current, event);
          return next === current ? current : appendPdfJobEventLog(next, event);
        });
        break;
      case "diagnostic":
        updatePdfJob((current) => appendPdfJobEventLog(mergePdfJobEventMetadata({
          ...current,
          diagnostics: appendUniqueDiagnostics(current.diagnostics ?? [], [event.diagnostic]),
        }, event), event));
        break;
      case "started":
        updatePdfJob((current) => appendPdfJobEventLog(mergePdfJobEventMetadata({
          ...current,
          taskId: event.taskId,
          status: current.status === "cancelling" ? "cancelling" : "running",
          stage: current.stage ?? "worker_starting",
          workerVersion: event.workerVersion,
          message: null,
          code: null,
        }, event), event));
        break;
      case "stage":
        updatePdfJob((current) => {
          const next = mergePdfJobEventMetadata({
            ...current,
            taskId: event.taskId,
            status: current.status === "cancelling" ? "cancelling" : "running",
            stage: event.stage,
            message: null,
          }, event);
          const nextWithPreflight = /preflight/i.test(event.stage) ? markPdfPreflightRunning(next) : next;
          return appendPdfJobEventLog(nextWithPreflight, event);
        });
        break;
      case "progress":
        updatePdfJob((current) => {
          const next = mergePdfJobEventMetadata({
            ...current,
            taskId: event.taskId,
            status: current.status === "cancelling" ? "cancelling" : "running",
            stage: event.progress.stage,
            progress: event.progress,
            message: event.progress.message ?? current.message,
          }, event);
          const nextWithPreflight = /preflight/i.test(event.progress.stage) ? markPdfPreflightRunning(next) : next;
          return appendPdfJobEventLog(nextWithPreflight, event);
        });
        break;
      case "tokenUsage":
        updatePdfJob((current) => mergePdfJobEventMetadata({ ...current, tokenUsage: event.usage }, event));
        break;
      case "warning":
        updatePdfJob((current) => {
          if (event.code.toLowerCase().includes("preflight")) {
            const next = reducePdfPreflightWarning(current, event);
            return next === current ? current : appendPdfJobEventLog(next, event);
          }
          return appendPdfJobEventLog((() => {
            const next = mergePdfJobEventMetadata({
              ...current,
              warnings: appendUniqueStrings(current.warnings, [event.message]),
            }, event);
            return event.diagnostic
              ? { ...next, diagnostics: appendUniqueDiagnostics(next.diagnostics ?? [], [event.diagnostic]) }
              : next;
          })(), event);
        });
        break;
      case "finished": {
        clearPdfTaskRefs();
        const translatedFile = validatePdfPath(event.outputPdf);
        const outputPathError = "翻译已完成，但输出 PDF 路径无效。请从任务面板检查输出文件。";
        if (translatedFile) {
          setSelectedFile(translatedFile);
          setError(null);
        } else {
          setError(outputPathError);
        }
        updatePdfJob((current) => {
          const next = appendPdfJobEventLog(mergePdfJobEventMetadata({
            ...current,
            taskId: event.taskId,
            status: "completed",
            progress: current.progress ? { ...current.progress, fraction: 1 } : current.progress,
            stage: "finished",
            outputPdf: event.outputPdf,
            outputMode: event.outputMode,
            pageCount: event.pageCount,
            warnings: appendUniqueStrings(current.warnings, event.warnings),
            message: null,
            code: null,
          }, event), event);
          return translatedFile
            ? next
            : {
              ...next,
              code: "invalid_output_path",
              message: outputPathError,
              logs: appendPdfJobLogMessage(next.logs, "result", "error", `${outputPathError} · 错误代码：invalid_output_path`),
            };
        });
        break;
      }
      case "cancelled":
        clearPdfTaskRefs();
        updatePdfJob((current) => appendPdfJobEventLog(mergePdfJobEventMetadata({
          ...current,
          taskId: event.taskId,
          status: "cancelled",
          message: event.reason ?? "PDF 翻译已取消",
          code: null,
          outputPdf: null,
        }, event), event));
        break;
      case "failed":
        clearPdfTaskRefs();
        updatePdfJob((current) => {
          const preflightState = event.code.toLowerCase().includes("preflight")
            ? reducePdfPreflightWarning(current, {
              type: "warning",
              taskId: event.taskId,
              code: event.code,
              message: event.message,
            })
            : current;
          const next = mergePdfJobEventMetadata({
            ...preflightState,
            taskId: event.taskId,
            status: "failed",
            code: event.code,
            message: event.message,
            outputPdf: null,
          }, event);
          return appendPdfJobEventLog(next, event);
        });
        break;
    }
  }, [clearPdfTaskRefs, matchesPdfTask, updatePdfJob]);

  const startPdfTranslation = useCallback(async (
    samples: PdfPreflightSample[] = [],
    preflightWarning: string | null = null,
    pages: string | null = null,
  ) => {
    if (!selectedFile) return;
    if (pdfPreflightSaving) return;
    if (!jobEventsReady) {
      updatePdfJob((current) => {
        const message = jobEventsError ?? "PDF 任务事件监听尚未就绪，请稍后再试。";
        return {
          ...current,
          status: "failed",
          code: "events_not_ready",
          message,
          logs: appendPdfJobLogMessage(current.logs, "result", "error", `${message} · 错误代码：events_not_ready`),
        };
      });
      return;
    }
    if (engineStatus?.status !== "ready") {
      updatePdfJob((current) => {
        const message = engineError ?? "PDF Engine 尚未就绪，请先准备运行环境。";
        return {
          ...current,
          status: "failed",
          code: "engine_not_ready",
          message,
          logs: appendPdfJobLogMessage(current.logs, "result", "error", `${message} · 错误代码：engine_not_ready`),
        };
      });
      return;
    }
    if (activeTaskIdRef.current || startAttemptRef.current !== null || isPdfJobBusy(pdfJobRef.current.status)) return;

    const attempt = startAttemptSequenceRef.current + 1;
    startAttemptSequenceRef.current = attempt;
    startAttemptRef.current = attempt;
    let initialLogs = appendPdfJobLogMessage([], "system", "info", "状态：正在启动 Worker · 准备 PDF 翻译任务");
    initialLogs = appendPdfJobLogMessage(
      initialLogs,
      "preflight",
      pdfPreflightEnabled ? "info" : "warning",
      pdfPreflightEnabled
        ? `文档预检已启用，将读取前 ${pdfPreflightPageLimit} 页`
        : "文档预检已关闭，将跳过预检请求",
    );
    if (preflightWarning) {
      initialLogs = appendPdfJobLogMessage(initialLogs, "warning", "warning", preflightWarning);
    }
    initialLogs = appendPdfJobLogMessage(
      initialLogs,
      "system",
      "info",
      pages ? `翻译页面：${pages}` : "翻译页面：全部页面",
    );
    updatePdfJob(() => ({ ...emptyPdfJob(), status: "starting", message: null, logs: initialLogs }));
    try {
      const pdfOptions = {
        source_language: "en",
        target_language: "zh-CN",
        output_mode: "bilingual",
        metadata: { file_name: selectedFile.fileName },
        preflight_enabled: pdfPreflightEnabled,
        preflight_page_limit: pdfPreflightPageLimit,
        samples: pdfPreflightEnabled ? samples : [],
        ...(pages ? { pages } : {}),
      };
      const commandPromise = invokeCommand<unknown>("start_pdf_translation", {
        filePath: selectedFile.path,
        pdfOptions,
      });
      void commandPromise.then((lateRaw) => {
        if (!disposedRef.current && startAttemptRef.current === attempt) return;
        const lateResult = decodePdfTranslationStartResult(lateRaw);
        if (lateResult) void invokeCommand("cancel_pdf_translation", { taskId: lateResult.taskId }).catch(() => undefined);
      }).catch(() => undefined);
      const raw = await withTimeout(commandPromise, PDF_TRANSLATION_START_TIMEOUT_MS, "启动 PDF 翻译超时，请检查 Worker 运行环境。");
      const result = decodePdfTranslationStartResult(raw);
      if (!result) throw new Error("PDF 翻译启动命令返回了无法识别的任务 ID。");
      if (disposedRef.current) {
        void invokeCommand("cancel_pdf_translation", { taskId: result.taskId }).catch(() => undefined);
        return;
      }
      if (startAttemptRef.current !== attempt) {
        void invokeCommand("cancel_pdf_translation", { taskId: result.taskId }).catch(() => undefined);
        return;
      }
      if (activeTaskIdRef.current && activeTaskIdRef.current !== result.taskId) {
        throw new Error("PDF 翻译启动返回了不匹配的任务 ID。");
      }
      activeTaskIdRef.current = result.taskId;
      updatePdfJob((current) => current.status === "starting"
        ? { ...current, taskId: result.taskId, status: "running", stage: current.stage ?? "worker_starting" }
        : current);
    } catch (reason) {
      if (disposedRef.current || startAttemptRef.current !== attempt) return;
      clearPdfTaskRefs();
      updatePdfJob((current) => {
        const message = describeError(reason, "启动 PDF 翻译失败");
        return {
          ...current,
          status: "failed",
          code: "start_failed",
          message,
          logs: appendPdfJobLogMessage(current.logs, "result", "error", `${message} · 错误代码：start_failed`),
        };
      });
    }
  }, [clearPdfTaskRefs, engineError, engineStatus?.status, jobEventsError, jobEventsReady, pdfPreflightEnabled, pdfPreflightPageLimit, pdfPreflightSaving, selectedFile, updatePdfJob]);

  const cancelPdfTranslation = useCallback(async () => {
    const taskId = activeTaskIdRef.current;
    const pendingStartAttempt = startAttemptRef.current;
    if ((!taskId && pendingStartAttempt === null) || pdfJobRef.current.status === "cancelling") return;
    if (!taskId) {
      clearPdfTaskRefs();
      updatePdfJob((current) => ({
        ...current,
        status: "cancelled",
        message: "PDF 翻译启动已取消",
        code: null,
        logs: appendPdfJobLogMessage(current.logs, "result", "warning", "PDF 翻译启动已取消。"),
      }));
      return;
    }
    updatePdfJob((current) => ({
      ...current,
      status: "cancelling",
      message: null,
      logs: appendPdfJobLogMessage(current.logs, "system", "warning", "状态：正在取消翻译"),
    }));
    clearCancelTimeout();
    cancelTimeoutRef.current = window.setTimeout(() => {
      if (disposedRef.current || activeTaskIdRef.current !== taskId || pdfJobRef.current.status !== "cancelling") return;
      clearPdfTaskRefs();
      updatePdfJob((current) => ({
        ...current,
        taskId,
        status: "cancelled",
        message: "取消确认超时，Worker 已终止。",
        code: "cancel_timeout",
        logs: appendPdfJobLogMessage(current.logs, "result", "warning", "取消确认超时，Worker 已终止 · 错误代码：cancel_timeout"),
      }));
    }, PDF_TRANSLATION_CANCEL_TIMEOUT_MS);

    try {
      const raw = await invokeCommand<unknown>("cancel_pdf_translation", { taskId });
      const result = decodePdfTranslationCancelResult(raw);
      if (disposedRef.current || activeTaskIdRef.current !== taskId) return;
      if (result === null) throw new Error("取消命令返回了无法识别的状态。");
      if (!result) {
        clearPdfTaskRefs();
        updatePdfJob((current) => ({
          ...current,
          taskId,
          status: "cancelled",
          message: "PDF 翻译任务已结束。",
          code: null,
          logs: appendPdfJobLogMessage(current.logs, "result", "warning", "PDF 翻译任务已结束。"),
        }));
      }
    } catch (reason) {
      if (disposedRef.current || activeTaskIdRef.current !== taskId) return;
      clearPdfTaskRefs();
      updatePdfJob((current) => {
        const message = describeError(reason, "取消 PDF 翻译失败");
        return {
          ...current,
          taskId,
          status: "failed",
          code: "cancel_failed",
          message,
          logs: appendPdfJobLogMessage(current.logs, "result", "error", `${message} · 错误代码：cancel_failed`),
        };
      });
    }
  }, [clearCancelTimeout, clearPdfTaskRefs, updatePdfJob]);

  useEffect(() => {
    disposedRef.current = false;
    return () => {
      disposedRef.current = true;
      const taskId = activeTaskIdRef.current;
      clearPdfTaskRefs();
      if (taskId) void invokeCommand("cancel_pdf_translation", { taskId }).catch(() => undefined);
    };
  }, [clearPdfTaskRefs]);

  useEffect(() => {
    const percentage = progressPercent(pdfJob.progress);
    const busy = isPdfJobBusy(pdfJob.status);
    const state = busy ? (percentage === null ? "indeterminate" : "normal") : "none";
    const value = percentage ?? 0;
    const key = `${state}:${value}`;
    if (taskbarProgressKeyRef.current === key) return;
    taskbarProgressKeyRef.current = key;
    void invokeCommand("set_main_taskbar_progress", {
      state,
      value,
    }).catch(() => undefined);
  }, [pdfJob.progress, pdfJob.status]);

  useEffect(() => () => {
    void invokeCommand("set_main_taskbar_progress", { state: "none", value: 0 }).catch(() => undefined);
  }, []);

  useEffect(() => {
    if (!active) {
      setDragging(false);
    }
  }, [active]);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    setJobEventsError(null);
    setJobEventsReady(false);

    const initialiseListeners = async () => {
      const jobResults = await Promise.allSettled(PDF_JOB_EVENT_NAMES.map((name) => (
        listenTo<unknown>(name, (payload) => {
          if (disposed || disposedRef.current) return;
          const event = decodePdfJobEvent(name, payload);
          if (event) handlePdfJobEvent(event);
        })
      )));

      for (const result of jobResults) {
        if (result.status !== "fulfilled") continue;
        if (disposed) result.value();
        else unlisteners.push(result.value);
      }
      if (disposed) return;

      const jobFailure = jobResults.find((result) => result.status === "rejected");
      if (jobFailure?.status === "rejected") {
        unlisteners.splice(0).forEach((unlisten) => unlisten());
        setJobEventsError(describeError(jobFailure.reason, "PDF 任务事件监听初始化失败。"));
        setJobEventsReady(false);
      } else {
        setJobEventsReady(true);
      }
    };

    void initialiseListeners();
    return () => {
      disposed = true;
      unlisteners.splice(0).forEach((unlisten) => unlisten());
    };
  }, [handlePdfJobEvent]);

  useEffect(() => {
    if (!active || engineStatusLoading || !engineStatus || enginePromptShownRef.current || (engineStatus.status !== "missing" && engineStatus.status !== "invalid")) return;
    enginePromptShownRef.current = true;
    onResourceDownloadPrompt({
      resource: "pdf-engine",
      title: "准备 PDF Engine",
      description: "PDF 全文翻译依赖本地 PDF Engine，首次使用需要准备运行环境",
      startLabel: "前往关于",
      failedLabel: "前往关于",
      onStart: onOpenPdfEngineSettings,
    });
  }, [active, engineStatus, engineStatusLoading, onOpenPdfEngineSettings, onResourceDownloadPrompt]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;

    const initialiseDragDrop = async () => {
      try {
        const next = await getCurrentWebview().onDragDropEvent((event) => {
          if (disposed || !active) return;
          switch (event.payload.type) {
            case "enter":
            case "over":
              setDragging(true);
              break;
            case "leave":
              setDragging(false);
              break;
            case "drop":
              handleDrop(event.payload.paths);
              break;
          }
        });
        if (disposed) next();
        else unlisten = next;
      } catch (reason) {
        if (!disposed) setError(describeError(reason, "PDF 拖放入口初始化失败"));
      }
    };

    void initialiseDragDrop();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [active, handleDrop]);

  const chooseFile = useCallback(async () => {
    setError(null);
    try {
      const selected = await open({
        directory: false,
        multiple: false,
        filters: [{ name: "PDF 文件", extensions: ["pdf"] }],
      });
      if (selected === null) return;

      const paths = Array.isArray(selected) ? selected : [selected];
      if (paths.length !== 1) {
        setError(MULTIPLE_FILES_ERROR);
        return;
      }
      acceptPath(paths[0]);
    } catch (reason) {
      setError(describeError(reason, "打开 PDF 文件选择器失败"));
    }
  }, [acceptPath]);

  const removeFile = useCallback(() => {
    abandonPdfTask();
    setSelectedFile(null);
    setDragging(false);
    setError(null);
  }, [abandonPdfTask]);

  const openOutputDirectory = useCallback(async (path: string) => {
    try {
      await invokeCommand("reveal_pdf_file", { filePath: path });
      setError(null);
    } catch (reason) {
      setError(describeError(reason, "打开 PDF 文件目录失败"));
    }
  }, []);

  const pdfJobBusy = isPdfJobBusy(pdfJob.status);
  const progressValue = progressPercent(pdfJob.progress);
  const selectedFilePath = selectedFile?.path;
  const progressDetail = formatPdfTaskProgressDetail(
    pdfJob,
    pdfJob.preflight ?? createEmptyPdfPreflightState(),
    pdfPreflightEnabled,
  );

  useEffect(() => {
    if (!active && selectedFilePath && pdfJobBusy) {
      onPdfTranslationFloatingChange({
        status: "running",
        statusText: `PDF 全文翻译 · ${progressDetail}`,
        progress: progressValue,
      });
    } else {
      onPdfTranslationFloatingChange(null);
    }
  }, [active, onPdfTranslationFloatingChange, pdfJobBusy, progressDetail, progressValue, selectedFilePath]);

  useEffect(() => () => {
    onPdfTranslationFloatingChange(null);
  }, [onPdfTranslationFloatingChange]);

  return (
    <>
      <section className={`page-section pdf-page ${selectedFile ? "pdf-reader-page" : ""} ${active ? "" : "pdf-page-persisted-inactive"}`} aria-hidden={!active}>
      {!selectedFile && (
        <div className="page-heading">
          <div className="page-title-block">
            <p className="eyebrow">PDF TRANSLATION</p>
            <div className="page-title-line pdf-page-title-line">
              <h1>PDF 全文翻译</h1>
              <button
                id="pdf-engine-status-entry"
                className={`pdf-engine-status-bubble pdf-engine-status-bubble-button ${engineStatus?.status === "ready" ? "is-ready" : "is-unavailable"}`}
                type="button"
                onClick={onOpenPdfEngineSettings}
                aria-label="打开 PDF Engine 设置"
                aria-live="polite"
              >
                {engineStatus?.status === "ready" ? "PDF引擎可用" : "PDF引擎不可用"}
              </button>
            </div>
          </div>
        </div>
      )}

      {selectedFile ? (
        <>
          {dragging && <div className="pdf-reader-drop-notice"><Upload size={14} />松开以替换当前 PDF</div>}
          <Suspense fallback={<div className="pdf-reader-shell"><div className="pdf-reader-state"><span>正在加载 PDF 阅读器</span></div></div>}>
            <PdfReader
              file={selectedFile}
              onReplace={() => void chooseFile()}
              onRemove={removeFile}
              job={pdfJob}
              jobEventsReady={jobEventsReady}
              jobEventsError={jobEventsError}
              translationEnabled={engineStatus?.status === "ready" && jobEventsReady}
              pdfPreflightEnabled={pdfPreflightEnabled}
              pdfPreflightPageLimit={pdfPreflightPageLimit}
              pdfPreflightSaving={pdfPreflightSaving}
              onPdfPreflightEnabledChange={onPdfPreflightEnabledChange}
              onStartTranslation={(samples, warning, pages) => void startPdfTranslation(samples, warning, pages)}
              onCancelTranslation={() => void cancelPdfTranslation()}
              onOpenOutputDirectory={(path) => void openOutputDirectory(path)}
            />
          </Suspense>
          {error && <p className="error-message pdf-reader-external-error" role="alert">{error}</p>}
        </>
      ) : (
        <>
          <div className={`pdf-import-card ${dragging ? "is-dragging" : ""}`}>
            <div className="pdf-drop-zone" aria-live="polite">
              <div className="pdf-drop-icon" aria-hidden="true"><FileType2 size={25} strokeWidth={1.6} /></div>
              <strong>{dragging ? "松开以导入 PDF" : "拖放 PDF 文件到这里"}</strong>
              <p>当前支持单个 PDF 文件，也可以使用文件选择器导入</p>
              <button className="secondary-button" type="button" onClick={() => void chooseFile()}>
                <Upload size={15} />
                选择 PDF 文件
              </button>
            </div>
          </div>

          <div className="pdf-message-area" aria-live="polite">
            {error && <p className="error-message" role="alert">{error}</p>}
          </div>

        </>
      )}
      </section>
    </>
  );
}
