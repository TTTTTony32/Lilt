import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { FileType2, Upload } from "lucide-react";
import { describeError } from "./lib/errors";
import { type PdfFile, validatePdfPath } from "./lib/pdf";
import { invokeCommand, listenTo } from "./lib/tauri";
import { PdfEnginePanel } from "./PdfEnginePanel";
import {
  PDF_ENGINE_EVENT_NAMES,
  PDF_JOB_EVENT_NAMES,
  decodePdfEngineEvent,
  decodePdfEngineStatus,
  decodePdfJobEvent,
  decodePdfTranslationCancelResult,
  decodePdfTranslationStartResult,
  type PdfEngineEvent,
  type PdfEngineProgress,
  type PdfEngineStatus,
  type PdfJobEvent,
  type PdfJobUiState,
} from "./types/contracts";

const PdfReader = lazy(() => import("./PdfReader"));

const MULTIPLE_FILES_ERROR = "当前只支持单个 PDF 文件，请一次拖放一个文件。";
const INVALID_FILE_ERROR = "请选择 PDF 文件，文件扩展名必须为 .pdf。";
const EMPTY_PATH_ERROR = "未找到 PDF 文件路径，请重试。";
const PDF_ENGINE_PREPARE_TIMEOUT_MS = 600_000;
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
  };
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

export default function PdfView() {
  const [selectedFile, setSelectedFile] = useState<PdfFile | null>(null);
  const [readerReloadToken, setReaderReloadToken] = useState(0);
  const [dragging, setDragging] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [engineStatus, setEngineStatus] = useState<PdfEngineStatus | null>(null);
  const [engineStatusLoading, setEngineStatusLoading] = useState(false);
  const [enginePreparing, setEnginePreparing] = useState(false);
  const [engineProgress, setEngineProgress] = useState<PdfEngineProgress | null>(null);
  const [engineError, setEngineError] = useState<string | null>(null);
  const [engineEventsError, setEngineEventsError] = useState<string | null>(null);
  const [jobEventsReady, setJobEventsReady] = useState(false);
  const [jobEventsError, setJobEventsError] = useState<string | null>(null);
  const [pdfJob, setPdfJob] = useState<PdfJobUiState>(() => emptyPdfJob());
  const pdfJobRef = useRef<PdfJobUiState>(emptyPdfJob());
  const disposedRef = useRef(false);
  const engineStatusRequestRef = useRef(0);
  const prepareAttemptRef = useRef(0);
  const prepareOperationRef = useRef<string | null>(null);
  const prepareTimeoutRef = useRef<number | null>(null);
  const preparePreviousStatusRef = useRef<PdfEngineStatus | null>(null);
  const preparingRef = useRef(false);
  const startAttemptRef = useRef<number | null>(null);
  const startAttemptSequenceRef = useRef(0);
  const activeTaskIdRef = useRef<string | null>(null);
  const cancelTimeoutRef = useRef<number | null>(null);

  const updatePdfJob = useCallback((updater: (current: PdfJobUiState) => PdfJobUiState) => {
    setPdfJob((current) => {
      const next = updater(current);
      pdfJobRef.current = next;
      return next;
    });
  }, []);

  const clearPrepareTimeout = useCallback(() => {
    if (prepareTimeoutRef.current === null) return;
    window.clearTimeout(prepareTimeoutRef.current);
    prepareTimeoutRef.current = null;
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

  const refreshEngineStatus = useCallback(async () => {
    const requestId = engineStatusRequestRef.current + 1;
    engineStatusRequestRef.current = requestId;
    setEngineStatusLoading(true);
    setEngineError(null);
    try {
      const raw = await invokeCommand<unknown>("get_pdf_engine_status");
      const next = decodePdfEngineStatus(raw);
      if (!next) throw new Error("PDF Engine 状态返回了无法识别的结果。");
      if (disposedRef.current || requestId !== engineStatusRequestRef.current) return;
      setEngineStatus(next);
      setEngineError(next.error);
      if (next.status === "preparing") {
        preparingRef.current = true;
        setEnginePreparing(true);
      } else {
        preparingRef.current = false;
        clearPrepareTimeout();
        setEnginePreparing(false);
        setEngineProgress(null);
      }
    } catch (reason) {
      if (disposedRef.current || requestId !== engineStatusRequestRef.current) return;
      const message = describeError(reason, "无法读取 PDF Engine 状态");
      setEngineStatus((current) => current ? { ...current, status: "invalid", error: message } : {
        status: "invalid",
        engineVersion: null,
        target: null,
        pythonVersion: null,
        babeldocVersion: null,
        distributionVersion: null,
        resourceSizeBytes: null,
        updating: false,
        error: message,
      });
      setEngineError(message);
    } finally {
      if (!disposedRef.current && requestId === engineStatusRequestRef.current) {
        setEngineStatusLoading(false);
      }
    }
  }, [clearPrepareTimeout]);

  const handleEngineEvent = useCallback((event: PdfEngineEvent) => {
    if (disposedRef.current) return;
    switch (event.type) {
      case "status":
        setEngineStatus(event.status);
        setEngineError(event.status.error);
        if (event.status.status !== "preparing") {
          preparingRef.current = false;
          clearPrepareTimeout();
          setEnginePreparing(false);
          setEngineProgress(null);
        }
        break;
      case "prepareStarted":
        if (!preparingRef.current) return;
        prepareOperationRef.current = event.operationId;
        setEnginePreparing(true);
        setEngineProgress(null);
        setEngineStatus((current) => current ? { ...current, status: "preparing", error: null } : current);
        break;
      case "prepareProgress":
        if (!preparingRef.current) return;
        if (prepareOperationRef.current && event.progress.operationId && prepareOperationRef.current !== event.progress.operationId) return;
        if (!prepareOperationRef.current && event.progress.operationId) {
          prepareOperationRef.current = event.progress.operationId;
        }
        setEnginePreparing(true);
        setEngineProgress(event.progress);
        setEngineStatus((current) => current ? { ...current, status: "preparing", error: null } : current);
        break;
      case "prepareCompleted":
        if (!preparingRef.current) return;
        if (prepareOperationRef.current && event.operationId && prepareOperationRef.current !== event.operationId) return;
        preparingRef.current = false;
        prepareOperationRef.current = null;
        preparePreviousStatusRef.current = null;
        clearPrepareTimeout();
        setEnginePreparing(false);
        setEngineProgress(null);
        setEngineError(event.status?.error ?? null);
        setEngineStatus((current) => event.status ?? (current ? { ...current, status: "ready", error: null } : {
          status: "ready",
          engineVersion: null,
          target: null,
          pythonVersion: null,
          babeldocVersion: null,
          distributionVersion: null,
          resourceSizeBytes: null,
          updating: false,
          error: null,
        }));
        void refreshEngineStatus();
        break;
      case "prepareFailed": {
        if (!preparingRef.current) return;
        if (prepareOperationRef.current && event.operationId && prepareOperationRef.current !== event.operationId) return;
        preparingRef.current = false;
        prepareOperationRef.current = null;
        const failedStatus = event.status;
        clearPrepareTimeout();
        setEnginePreparing(false);
        setEngineProgress(null);
        setEngineError(failedStatus?.error ?? event.message);
        setEngineStatus((current) => failedStatus ?? (current ? {
          ...current,
          status: current.status === "ready" ? "ready" : "invalid",
          error: event.message,
        } : {
          status: "invalid",
          engineVersion: null,
          target: null,
          pythonVersion: null,
          babeldocVersion: null,
          distributionVersion: null,
          resourceSizeBytes: null,
          updating: false,
          error: event.message,
        }));
        break;
      }
    }
  }, [clearPrepareTimeout, refreshEngineStatus]);

  const preparePdfEngine = useCallback(async () => {
    if (preparingRef.current || engineStatus?.status === "preparing") return;
    const attempt = prepareAttemptRef.current + 1;
    prepareAttemptRef.current = attempt;
    preparePreviousStatusRef.current = engineStatus;
    preparingRef.current = true;
    prepareOperationRef.current = null;
    clearPrepareTimeout();
    setEngineStatusLoading(false);
    setEnginePreparing(true);
    setEngineProgress(null);
    setEngineError(null);
    setEngineStatus((current) => current ? { ...current, status: "preparing", error: null } : current);
    prepareTimeoutRef.current = window.setTimeout(() => {
      if (disposedRef.current || prepareAttemptRef.current !== attempt || !preparingRef.current) return;
      preparingRef.current = false;
      prepareOperationRef.current = null;
      prepareTimeoutRef.current = null;
      setEnginePreparing(false);
      setEngineProgress(null);
      const message = "PDF Engine 准备超时，请检查运行环境后重试。";
      setEngineError(message);
      const previousStatus = preparePreviousStatusRef.current;
      preparePreviousStatusRef.current = null;
      setEngineStatus((current) => previousStatus?.status === "ready" ? {
        ...previousStatus,
        error: message,
      } : current ? { ...current, status: "invalid", error: message } : {
        status: "invalid",
        engineVersion: null,
        target: null,
        pythonVersion: null,
        babeldocVersion: null,
        distributionVersion: null,
        resourceSizeBytes: null,
        updating: false,
        error: message,
      });
    }, PDF_ENGINE_PREPARE_TIMEOUT_MS);

    try {
      const raw = await invokeCommand<unknown>("prepare_pdf_engine");
      const next = decodePdfEngineStatus(raw);
      if (!next) throw new Error("PDF Engine 准备命令返回了无法识别的结果。");
      if (disposedRef.current || prepareAttemptRef.current !== attempt) return;
      setEngineStatus(next);
      setEngineError(next.status === "invalid" ? next.error : null);
      if (next.status !== "preparing") {
        preparingRef.current = false;
        prepareOperationRef.current = null;
        preparePreviousStatusRef.current = null;
        clearPrepareTimeout();
        setEnginePreparing(false);
        setEngineProgress(null);
      }
    } catch (reason) {
      if (disposedRef.current || prepareAttemptRef.current !== attempt) return;
      preparingRef.current = false;
      prepareOperationRef.current = null;
      clearPrepareTimeout();
      setEnginePreparing(false);
      setEngineProgress(null);
      const message = describeError(reason, "准备 PDF Engine 失败");
      setEngineError(message);
      const previousStatus = preparePreviousStatusRef.current;
      preparePreviousStatusRef.current = null;
      setEngineStatus((current) => previousStatus?.status === "ready" ? {
        ...previousStatus,
        error: message,
      } : current ? { ...current, status: "invalid", error: message } : {
        status: "invalid",
        engineVersion: null,
        target: null,
        pythonVersion: null,
        babeldocVersion: null,
        distributionVersion: null,
        resourceSizeBytes: null,
        updating: false,
        error: message,
      });
    }
  }, [clearPrepareTimeout, engineStatus]);

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
      case "started":
        updatePdfJob((current) => ({
          ...current,
          taskId: event.taskId,
          status: current.status === "cancelling" ? "cancelling" : "running",
          stage: current.stage ?? "worker_starting",
          workerVersion: event.workerVersion,
          message: null,
          code: null,
        }));
        break;
      case "stage":
        updatePdfJob((current) => ({
          ...current,
          taskId: event.taskId,
          status: current.status === "cancelling" ? "cancelling" : "running",
          stage: event.stage,
          message: null,
        }));
        break;
      case "progress":
        updatePdfJob((current) => ({
          ...current,
          taskId: event.taskId,
          status: current.status === "cancelling" ? "cancelling" : "running",
          stage: event.progress.stage,
          progress: event.progress,
          message: event.progress.message ?? current.message,
        }));
        break;
      case "tokenUsage":
        updatePdfJob((current) => ({ ...current, tokenUsage: event.usage }));
        break;
      case "warning":
        updatePdfJob((current) => ({
          ...current,
          warnings: current.warnings.includes(event.message) ? current.warnings : [...current.warnings, event.message],
        }));
        break;
      case "finished": {
        clearPdfTaskRefs();
        const translatedFile = validatePdfPath(event.outputPdf);
        if (translatedFile) {
          setSelectedFile(translatedFile);
          setError(null);
        } else {
          setError("翻译已完成，但输出 PDF 路径无效。请从任务面板检查输出文件。");
        }
        updatePdfJob((current) => ({
          ...current,
          taskId: event.taskId,
          status: "completed",
          progress: current.progress ? { ...current.progress, fraction: 1 } : current.progress,
          stage: "finished",
          outputPdf: event.outputPdf,
          outputMode: event.outputMode,
          pageCount: event.pageCount,
          warnings: [...current.warnings, ...event.warnings.filter((warning) => !current.warnings.includes(warning))],
          message: null,
          code: null,
        }));
        break;
      }
      case "cancelled":
        clearPdfTaskRefs();
        updatePdfJob((current) => ({
          ...current,
          taskId: event.taskId,
          status: "cancelled",
          message: event.reason ?? "PDF 翻译已取消",
          code: null,
          outputPdf: null,
        }));
        break;
      case "failed":
        clearPdfTaskRefs();
        updatePdfJob((current) => ({
          ...current,
          taskId: event.taskId,
          status: "failed",
          code: event.code,
          message: event.message,
          outputPdf: null,
        }));
        break;
    }
  }, [clearPdfTaskRefs, matchesPdfTask, updatePdfJob]);

  const startPdfTranslation = useCallback(async () => {
    if (!selectedFile) return;
    if (!jobEventsReady) {
      updatePdfJob((current) => ({ ...current, status: "failed", message: jobEventsError ?? "PDF 任务事件监听尚未就绪，请稍后再试。" }));
      return;
    }
    if (engineStatus?.status !== "ready") {
      updatePdfJob((current) => ({ ...current, status: "failed", message: engineError ?? "PDF Engine 尚未就绪，请先准备运行环境。" }));
      return;
    }
    if (activeTaskIdRef.current || startAttemptRef.current !== null || pdfJobRef.current.status === "starting") return;

    const attempt = startAttemptSequenceRef.current + 1;
    startAttemptSequenceRef.current = attempt;
    startAttemptRef.current = attempt;
    updatePdfJob(() => ({ ...emptyPdfJob(), status: "starting", message: null }));
    try {
      const commandPromise = invokeCommand<unknown>("start_pdf_translation", {
        filePath: selectedFile.path,
        pdfOptions: {
          source_language: "en",
          target_language: "zh-CN",
          output_mode: "bilingual",
        },
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
      updatePdfJob((current) => ({
        ...current,
        status: "failed",
        code: "start_failed",
        message: describeError(reason, "启动 PDF 翻译失败"),
      }));
    }
  }, [clearPdfTaskRefs, engineError, engineStatus?.status, jobEventsError, jobEventsReady, selectedFile, updatePdfJob]);

  const cancelPdfTranslation = useCallback(async () => {
    const taskId = activeTaskIdRef.current;
    if (!taskId || pdfJobRef.current.status === "cancelling") return;
    updatePdfJob((current) => ({ ...current, status: "cancelling", message: null }));
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
      }));
    }, PDF_TRANSLATION_CANCEL_TIMEOUT_MS);

    try {
      const raw = await invokeCommand<unknown>("cancel_pdf_translation", { taskId });
      const result = decodePdfTranslationCancelResult(raw);
      if (disposedRef.current || activeTaskIdRef.current !== taskId) return;
      if (result === null) throw new Error("取消命令返回了无法识别的状态。");
      if (!result) {
        clearPdfTaskRefs();
        updatePdfJob((current) => ({ ...current, taskId, status: "cancelled", message: "PDF 翻译任务已结束。", code: null }));
      }
    } catch (reason) {
      if (disposedRef.current || activeTaskIdRef.current !== taskId) return;
      clearPdfTaskRefs();
      updatePdfJob((current) => ({
        ...current,
        taskId,
        status: "failed",
        code: "cancel_failed",
        message: describeError(reason, "取消 PDF 翻译失败"),
      }));
    }
  }, [clearCancelTimeout, clearPdfTaskRefs, updatePdfJob]);

  useEffect(() => {
    disposedRef.current = false;
    return () => {
      disposedRef.current = true;
      const taskId = activeTaskIdRef.current;
      clearPrepareTimeout();
      clearPdfTaskRefs();
      if (taskId) void invokeCommand("cancel_pdf_translation", { taskId }).catch(() => undefined);
    };
  }, [clearPdfTaskRefs, clearPrepareTimeout]);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    setEngineEventsError(null);
    setJobEventsError(null);
    setJobEventsReady(false);

    const initialiseListeners = async () => {
      const engineResults = await Promise.allSettled(PDF_ENGINE_EVENT_NAMES.map((name) => (
        listenTo<unknown>(name, (payload) => {
          if (disposed || disposedRef.current) return;
          const event = decodePdfEngineEvent(name, payload);
          if (event) handleEngineEvent(event);
        })
      )));
      const jobResults = await Promise.allSettled(PDF_JOB_EVENT_NAMES.map((name) => (
        listenTo<unknown>(name, (payload) => {
          if (disposed || disposedRef.current) return;
          const event = decodePdfJobEvent(name, payload);
          if (event) handlePdfJobEvent(event);
        })
      )));

      for (const result of [...engineResults, ...jobResults]) {
        if (result.status !== "fulfilled") continue;
        if (disposed) result.value();
        else unlisteners.push(result.value);
      }
      if (disposed) return;

      const engineFailure = engineResults.find((result) => result.status === "rejected");
      if (engineFailure?.status === "rejected") {
        setEngineEventsError(describeError(engineFailure.reason, "PDF Engine 事件监听初始化失败。"));
      }
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
  }, [handleEngineEvent, handlePdfJobEvent]);

  useEffect(() => {
    void refreshEngineStatus();
  }, [refreshEngineStatus]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;

    const initialiseDragDrop = async () => {
      try {
        const next = await getCurrentWebview().onDragDropEvent((event) => {
          if (disposed) return;
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
  }, [handleDrop]);

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

  const openOutputInReader = useCallback((path: string) => {
    const file = validatePdfPath(path);
    if (!file) {
      setError("翻译输出文件不存在，无法在软件内打开。请检查输出目录。");
      return;
    }
    setError(null);
    setSelectedFile(file);
    setReaderReloadToken((current) => current + 1);
  }, []);

  return (
    <section className={`page-section pdf-page ${selectedFile ? "pdf-reader-page" : ""}`}>
      {!selectedFile && (
        <div className="page-heading">
          <div className="page-title-block">
            <p className="eyebrow">PDF TRANSLATION</p>
            <h1>PDF 全文翻译</h1>
            <p className="page-description">导入单个 PDF 文件，准备开始全文翻译。</p>
          </div>
        </div>
      )}

      {selectedFile ? (
        <>
          {dragging && <div className="pdf-reader-drop-notice"><Upload size={14} />松开以替换当前 PDF</div>}
          <Suspense fallback={<div className="pdf-reader-shell"><div className="pdf-reader-state"><span>正在加载 PDF 阅读器</span></div></div>}>
            <PdfReader
              key={`${selectedFile.path}-${readerReloadToken}`}
              file={selectedFile}
              onReplace={() => void chooseFile()}
              onRemove={removeFile}
              job={pdfJob}
              jobEventsReady={jobEventsReady}
              jobEventsError={jobEventsError}
              translationEnabled={engineStatus?.status === "ready" && jobEventsReady}
              onStartTranslation={() => void startPdfTranslation()}
              onCancelTranslation={() => void cancelPdfTranslation()}
              onOpenOutputDirectory={(path) => void openOutputDirectory(path)}
              onOpenOutputInReader={openOutputInReader}
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
              <p>当前支持单个 PDF 文件，也可以使用文件选择器导入。</p>
              <button className="secondary-button" type="button" onClick={() => void chooseFile()}>
                <Upload size={15} />
                选择 PDF 文件
              </button>
            </div>
          </div>

          <div className="pdf-message-area" aria-live="polite">
            {error && <p className="error-message" role="alert">{error}</p>}
          </div>

          <div className="simple-card pdf-stage-card">
            <PdfEnginePanel
              engineStatus={engineStatus}
              engineStatusLoading={engineStatusLoading}
              enginePreparing={enginePreparing}
              engineProgress={engineProgress}
              engineError={engineError ?? engineEventsError}
              onPrepareEngine={() => void preparePdfEngine()}
            />
          </div>
        </>
      )}
    </section>
  );
}
