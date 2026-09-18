import { useCallback, useEffect, useRef, useState } from "react";
import { describeError } from "./errors";
import { invokeCommand, listenTo } from "./tauri";
import {
  PDF_ENGINE_EVENT_NAMES,
  decodePdfEngineEvent,
  decodePdfEngineStatus,
  type PdfEngineEvent,
  type PdfEngineProgress,
  type PdfEngineStatus,
} from "../types/contracts";

const PDF_ENGINE_PREPARE_TIMEOUT_MS = 600_000;

export interface PdfEngineRuntime {
  status: PdfEngineStatus | null;
  statusLoading: boolean;
  preparing: boolean;
  progress: PdfEngineProgress | null;
  error: string | null;
  eventsError: string | null;
  prepare: () => Promise<void>;
}

function fallbackStatus(status: PdfEngineStatus["status"], error: string | null): PdfEngineStatus {
  return {
    status,
    engineVersion: null,
    target: null,
    pythonVersion: null,
    babeldocVersion: null,
    distributionVersion: null,
    resourceSizeBytes: null,
    updating: false,
    error,
  };
}

export function usePdfEngineRuntime(): PdfEngineRuntime {
  const [status, setStatus] = useState<PdfEngineStatus | null>(null);
  const [statusLoading, setStatusLoading] = useState(true);
  const [preparing, setPreparing] = useState(false);
  const [progress, setProgress] = useState<PdfEngineProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [eventsError, setEventsError] = useState<string | null>(null);
  const disposedRef = useRef(false);
  const statusRequestRef = useRef(0);
  const prepareAttemptRef = useRef(0);
  const prepareOperationRef = useRef<string | null>(null);
  const prepareTimeoutRef = useRef<number | null>(null);
  const preparePreviousStatusRef = useRef<PdfEngineStatus | null>(null);
  const preparingRef = useRef(false);

  const clearPrepareTimeout = useCallback(() => {
    if (prepareTimeoutRef.current === null) return;
    window.clearTimeout(prepareTimeoutRef.current);
    prepareTimeoutRef.current = null;
  }, []);

  const refreshStatus = useCallback(async () => {
    const requestId = statusRequestRef.current + 1;
    statusRequestRef.current = requestId;
    setStatusLoading(true);
    setError(null);
    try {
      const raw = await invokeCommand<unknown>("get_pdf_engine_status");
      const next = decodePdfEngineStatus(raw);
      if (!next) throw new Error("PDF Engine 状态返回了无法识别的结果。");
      if (disposedRef.current || requestId !== statusRequestRef.current) return;
      setStatus(next);
      setError(next.error);
      if (next.status === "preparing") {
        preparingRef.current = true;
        setPreparing(true);
      } else {
        preparingRef.current = false;
        clearPrepareTimeout();
        setPreparing(false);
        setProgress(null);
      }
    } catch (reason) {
      if (disposedRef.current || requestId !== statusRequestRef.current) return;
      const message = describeError(reason, "无法读取 PDF Engine 状态");
      setStatus((current) => current ? { ...current, status: "invalid", error: message } : fallbackStatus("invalid", message));
      setError(message);
    } finally {
      if (!disposedRef.current && requestId === statusRequestRef.current) {
        setStatusLoading(false);
      }
    }
  }, [clearPrepareTimeout]);

  const handleEngineEvent = useCallback((event: PdfEngineEvent) => {
    if (disposedRef.current) return;
    switch (event.type) {
      case "status":
        setStatus(event.status);
        setError(event.status.error);
        if (event.status.status !== "preparing") {
          preparingRef.current = false;
          clearPrepareTimeout();
          setPreparing(false);
          setProgress(null);
        }
        break;
      case "prepareStarted":
        if (!preparingRef.current) return;
        prepareOperationRef.current = event.operationId;
        setPreparing(true);
        setProgress(null);
        setStatus((current) => current ? { ...current, status: "preparing", error: null } : current);
        break;
      case "prepareProgress":
        if (!preparingRef.current) return;
        if (prepareOperationRef.current && event.progress.operationId && prepareOperationRef.current !== event.progress.operationId) return;
        if (!prepareOperationRef.current && event.progress.operationId) {
          prepareOperationRef.current = event.progress.operationId;
        }
        setPreparing(true);
        setProgress(event.progress);
        setStatus((current) => current ? { ...current, status: "preparing", error: null } : current);
        break;
      case "prepareCompleted":
        if (!preparingRef.current) return;
        if (prepareOperationRef.current && event.operationId && prepareOperationRef.current !== event.operationId) return;
        preparingRef.current = false;
        prepareOperationRef.current = null;
        preparePreviousStatusRef.current = null;
        clearPrepareTimeout();
        setPreparing(false);
        setProgress(null);
        setError(event.status?.error ?? null);
        setStatus((current) => event.status ?? (current ? { ...current, status: "ready", error: null } : fallbackStatus("ready", null)));
        void refreshStatus();
        break;
      case "prepareFailed": {
        if (!preparingRef.current) return;
        if (prepareOperationRef.current && event.operationId && prepareOperationRef.current !== event.operationId) return;
        preparingRef.current = false;
        prepareOperationRef.current = null;
        const failedStatus = event.status;
        clearPrepareTimeout();
        setPreparing(false);
        setProgress(null);
        setError(failedStatus?.error ?? event.message);
        setStatus((current) => failedStatus ?? (current ? {
          ...current,
          status: current.status === "ready" ? "ready" : "invalid",
          error: event.message,
        } : fallbackStatus("invalid", event.message)));
        break;
      }
    }
  }, [clearPrepareTimeout, refreshStatus]);

  const prepare = useCallback(async () => {
    if (preparingRef.current || status?.status === "preparing") return;
    const attempt = prepareAttemptRef.current + 1;
    prepareAttemptRef.current = attempt;
    preparePreviousStatusRef.current = status;
    preparingRef.current = true;
    prepareOperationRef.current = null;
    clearPrepareTimeout();
    setStatusLoading(false);
    setPreparing(true);
    setProgress(null);
    setError(null);
    setStatus((current) => current ? { ...current, status: "preparing", error: null } : current);
    prepareTimeoutRef.current = window.setTimeout(() => {
      if (disposedRef.current || prepareAttemptRef.current !== attempt || !preparingRef.current) return;
      preparingRef.current = false;
      prepareOperationRef.current = null;
      prepareTimeoutRef.current = null;
      setPreparing(false);
      setProgress(null);
      const message = "PDF Engine 准备超时，请检查运行环境后重试。";
      setError(message);
      const previousStatus = preparePreviousStatusRef.current;
      preparePreviousStatusRef.current = null;
      setStatus((current) => previousStatus?.status === "ready" ? {
        ...previousStatus,
        error: message,
      } : current ? { ...current, status: "invalid", error: message } : fallbackStatus("invalid", message));
    }, PDF_ENGINE_PREPARE_TIMEOUT_MS);

    try {
      const raw = await invokeCommand<unknown>("prepare_pdf_engine");
      const next = decodePdfEngineStatus(raw);
      if (!next) throw new Error("PDF Engine 准备命令返回了无法识别的结果。");
      if (disposedRef.current || prepareAttemptRef.current !== attempt) return;
      setStatus(next);
      setError(next.status === "invalid" ? next.error : null);
      if (next.status !== "preparing") {
        preparingRef.current = false;
        prepareOperationRef.current = null;
        preparePreviousStatusRef.current = null;
        clearPrepareTimeout();
        setPreparing(false);
        setProgress(null);
      }
    } catch (reason) {
      if (disposedRef.current || prepareAttemptRef.current !== attempt) return;
      preparingRef.current = false;
      prepareOperationRef.current = null;
      clearPrepareTimeout();
      setPreparing(false);
      setProgress(null);
      const message = describeError(reason, "准备 PDF Engine 失败");
      setError(message);
      const previousStatus = preparePreviousStatusRef.current;
      preparePreviousStatusRef.current = null;
      setStatus((current) => previousStatus?.status === "ready" ? {
        ...previousStatus,
        error: message,
      } : current ? { ...current, status: "invalid", error: message } : fallbackStatus("invalid", message));
    }
  }, [clearPrepareTimeout, status]);

  useEffect(() => {
    disposedRef.current = false;
    return () => {
      disposedRef.current = true;
      clearPrepareTimeout();
    };
  }, [clearPrepareTimeout]);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    setEventsError(null);

    const initialiseListeners = async () => {
      const results = await Promise.allSettled(PDF_ENGINE_EVENT_NAMES.map((name) => (
        listenTo<unknown>(name, (payload) => {
          if (disposed || disposedRef.current) return;
          const event = decodePdfEngineEvent(name, payload);
          if (event) handleEngineEvent(event);
        })
      )));

      for (const result of results) {
        if (result.status !== "fulfilled") continue;
        if (disposed) result.value();
        else unlisteners.push(result.value);
      }
      if (disposed) return;

      const failure = results.find((result) => result.status === "rejected");
      if (failure?.status === "rejected") {
        setEventsError(describeError(failure.reason, "PDF Engine 事件监听初始化失败。"));
      }
    };

    void initialiseListeners();
    return () => {
      disposed = true;
      unlisteners.splice(0).forEach((unlisten) => unlisten());
    };
  }, [handleEngineEvent]);

  useEffect(() => {
    void refreshStatus();
  }, [refreshStatus]);

  return {
    status,
    statusLoading,
    preparing,
    progress,
    error,
    eventsError,
    prepare,
  };
}
