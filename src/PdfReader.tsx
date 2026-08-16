import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ChevronLeft,
  ChevronRight,
  FileType2,
  Maximize2,
  RotateCcw,
  Trash2,
  Upload,
  ZoomIn,
  ZoomOut,
  LoaderCircle,
} from "lucide-react";
import { describeError } from "./lib/errors";
import { type PdfFile } from "./lib/pdf";
import {
  DEFAULT_PDF_PAGE_HEIGHT,
  DEFAULT_PDF_PAGE_WIDTH,
  fitPdfWidth,
  clampPdfPage,
  clampPdfZoom,
  MAX_PDF_ZOOM,
  MIN_PDF_ZOOM,
  toPdfBytes,
} from "./lib/pdf-reader-utils";
import {
  loadPdfDocument,
  type PDFDocumentLoadingTask,
  type PDFDocumentProxy,
  type PDFPageProxy,
  type RenderTask,
} from "./lib/pdf-reader";
import { invokeCommand } from "./lib/tauri";
import type { PdfEngineProgress, PdfEngineStatus, PdfJobUiState } from "./types/contracts";

type PdfReaderStatus = "loading" | "ready" | "error";
type PdfZoomMode = "fit-width" | "manual";
type PdfPageRenderStatus = "idle" | "loading" | "ready" | "error";

interface PdfReaderProps {
  file: PdfFile;
  onReplace: () => void;
  onRemove: () => void;
  engineStatus: PdfEngineStatus | null;
  engineStatusLoading: boolean;
  enginePreparing: boolean;
  engineProgress: PdfEngineProgress | null;
  engineError: string | null;
  job: PdfJobUiState;
  jobEventsReady: boolean;
  jobEventsError: string | null;
  translationEnabled: boolean;
  onPrepareEngine: () => void;
  onStartTranslation: () => void;
  onCancelTranslation: () => void;
}

interface PdfPageProps {
  pdfDocument: PDFDocumentProxy;
  pageNumber: number;
  zoomMode: PdfZoomMode;
  manualZoom: number;
  availableWidth: number;
  scrollRoot: HTMLDivElement | null;
  onElementChange: (pageNumber: number, element: HTMLDivElement | null) => void;
}

interface PageDimensions {
  width: number;
  height: number;
}

function formatStageName(stage: string): string {
  return stage
    .replace(/[_-]+/g, " ")
    .replace(/\b\w/g, (character) => character.toUpperCase());
}

function formatPdfStage(stage: string | null): string {
  if (!stage) return "等待 Worker";
  const normalized = stage.toLowerCase().replace(/[-\s]+/g, "_");
  const label = formatStageName(stage);
  if (normalized.includes("babeldoc") || /parse|layout|render|typeset|output|finish/.test(normalized)) {
    return `BabelDOC · ${label}`;
  }
  return `Worker · ${label}`;
}

function progressPercent(progress: { fraction: number | null; current: number | null; total: number | null } | null): number | null {
  if (!progress) return null;
  if (progress.fraction !== null) return Math.max(0, Math.min(100, Math.round(progress.fraction * 100)));
  if (progress.current !== null && progress.total !== null && progress.total > 0) {
    return Math.max(0, Math.min(100, Math.round((progress.current / progress.total) * 100)));
  }
  return null;
}

function formatBytes(value: number | null): string | null {
  if (value === null || !Number.isFinite(value) || value < 0) return null;
  if (value < 1024) return `${value} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let size = value;
  let unit = -1;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${size >= 10 ? size.toFixed(0) : size.toFixed(1)} ${units[unit]}`;
}

function engineStatusLabel(status: PdfEngineStatus | null, loading: boolean, preparing: boolean): string {
  if (preparing || status?.status === "preparing" || status?.updating) return status?.updating ? "更新中" : "准备中";
  if (loading) return "检查中";
  if (status?.status === "missing") return "未准备";
  if (status?.status === "ready") return "可用";
  if (status?.status === "invalid") return "失败";
  return "未准备";
}

function jobStatusLabel(status: PdfJobUiState["status"]): string {
  switch (status) {
    case "starting": return "正在启动 Worker";
    case "running": return "翻译进行中";
    case "cancelling": return "正在取消";
    case "completed": return "翻译完成";
    case "cancelled": return "已取消";
    case "failed": return "翻译失败";
    default: return "等待翻译";
  }
}

interface PdfTaskPanelProps {
  readerStatus: PdfReaderStatus;
  engineStatus: PdfEngineStatus | null;
  engineStatusLoading: boolean;
  enginePreparing: boolean;
  engineProgress: PdfEngineProgress | null;
  engineError: string | null;
  job: PdfJobUiState;
  jobEventsReady: boolean;
  jobEventsError: string | null;
  translationEnabled: boolean;
  onPrepareEngine: () => void;
  onStartTranslation: () => void;
  onCancelTranslation: () => void;
}

function PdfTaskPanel({
  readerStatus,
  engineStatus,
  engineStatusLoading,
  enginePreparing,
  engineProgress,
  engineError,
  job,
  jobEventsReady,
  jobEventsError,
  translationEnabled,
  onPrepareEngine,
  onStartTranslation,
  onCancelTranslation,
}: PdfTaskPanelProps) {
  const jobBusy = job.status === "starting" || job.status === "running" || job.status === "cancelling";
  const canStart = translationEnabled && readerStatus === "ready" && !jobBusy;
  const engineProgressValue = progressPercent(engineProgress);
  const jobProgressValue = progressPercent(job.progress);
  const engineDetails = [
    engineStatus?.engineVersion ? `Engine ${engineStatus.engineVersion}` : null,
    engineStatus?.babeldocVersion ? `BabelDOC ${engineStatus.babeldocVersion}` : null,
    engineStatus?.pythonVersion ? `Python ${engineStatus.pythonVersion}` : null,
    engineStatus?.distributionVersion ? `资源 ${engineStatus.distributionVersion}` : null,
    formatBytes(engineStatus?.resourceSizeBytes ?? null),
    engineStatus?.target ?? null,
  ].filter((detail): detail is string => detail !== null);

  return (
    <div className="pdf-task-panel">
      <div className="pdf-task-panel-section pdf-task-engine-section">
        <div className="pdf-task-panel-heading">
          <div>
            <span className="pdf-task-panel-kicker">PDF ENGINE</span>
            <strong>{engineStatusLabel(engineStatus, engineStatusLoading, enginePreparing)}</strong>
          </div>
          <button
            className="secondary-button small-button"
            type="button"
            onClick={onPrepareEngine}
            disabled={engineStatusLoading || enginePreparing || jobBusy}
          >
            {enginePreparing ? "准备中" : engineStatus?.status === "ready" ? "重新准备" : "准备 Engine"}
          </button>
        </div>
        {engineDetails.length > 0 && <p className="pdf-task-panel-meta">{engineDetails.join(" · ")}</p>}
        {(enginePreparing || engineProgress) && (
          <div className="pdf-task-progress-block">
            <div className="pdf-task-progress-label">
              <span>{engineProgress?.message ?? (engineProgress ? formatPdfStage(engineProgress.stage) : "正在准备运行环境")}</span>
              {engineProgressValue !== null && <span>{engineProgressValue}%</span>}
            </div>
            <div className="pdf-task-progress-track" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={engineProgressValue ?? undefined}>
              {engineProgressValue !== null && <span style={{ width: `${engineProgressValue}%` }} />}
            </div>
          </div>
        )}
        {engineError && <p className="pdf-task-error" role="alert">{engineError}</p>}
      </div>

      <div className="pdf-task-panel-section pdf-task-job-section">
        <div className="pdf-task-panel-heading">
          <div>
            <span className="pdf-task-panel-kicker">PDF TRANSLATION</span>
            <strong>{jobStatusLabel(job.status)}</strong>
          </div>
          <div className="pdf-task-panel-actions">
            {jobBusy && job.status !== "cancelling" && (
              <button className="secondary-button small-button" type="button" onClick={onCancelTranslation} disabled={!job.taskId}>
                取消任务
              </button>
            )}
            {job.status === "cancelling" && <span className="pdf-task-action-status">正在等待取消确认</span>}
            {!jobBusy && (
              <button className="primary-button small-button" type="button" onClick={onStartTranslation} disabled={!canStart}>
                {job.status === "completed" ? "再次翻译" : "开始翻译"}
              </button>
            )}
          </div>
        </div>
        {!jobEventsReady && <p className="pdf-task-error">{jobEventsError ?? "PDF 任务事件监听尚未就绪。"}</p>}
        {job.stage && <p className="pdf-task-panel-stage">{formatPdfStage(job.stage)}</p>}
        {job.progress && (
          <div className="pdf-task-progress-block">
            <div className="pdf-task-progress-label">
              <span>{job.progress.message ?? formatPdfStage(job.progress.stage)}</span>
              {jobProgressValue !== null && <span>{jobProgressValue}%</span>}
            </div>
            <div className="pdf-task-progress-track" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={jobProgressValue ?? undefined}>
              {jobProgressValue !== null && <span style={{ width: `${jobProgressValue}%` }} />}
            </div>
          </div>
        )}
        {job.message && <p className={job.status === "failed" ? "pdf-task-error" : "pdf-task-panel-message"} role={job.status === "failed" ? "alert" : undefined}>{job.message}</p>}
        {job.code && <p className="pdf-task-panel-meta">错误代码：{job.code}</p>}
        {job.status === "completed" && job.outputPdf && (
          <div className="pdf-task-output">
            <strong>输出已生成</strong>
            <span>{job.outputPdf}</span>
            <small>{[job.outputMode, job.pageCount === null ? null : `${job.pageCount} 页`].filter((item): item is string => item !== null).join(" · ")}</small>
          </div>
        )}
        {job.warnings.length > 0 && (
          <ul className="pdf-task-warnings">
            {job.warnings.map((warning) => <li key={warning}>{warning}</li>)}
          </ul>
        )}
        {job.tokenUsage && (
          <p className="pdf-task-panel-meta">
            Token：{job.tokenUsage.totalTokens ?? "—"}
          </p>
        )}
      </div>
    </div>
  );
}

function PdfPage({
  pdfDocument,
  pageNumber,
  zoomMode,
  manualZoom,
  availableWidth,
  scrollRoot,
  onElementChange,
}: PdfPageProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const documentRef = useRef<PDFDocumentProxy | null>(null);
  const renderSerialRef = useRef(Promise.resolve());
  const [isNearViewport, setIsNearViewport] = useState(pageNumber === 1);
  const [loadedPage, setLoadedPage] = useState<PDFPageProxy | null>(null);
  const [dimensions, setDimensions] = useState<PageDimensions>({
    width: DEFAULT_PDF_PAGE_WIDTH,
    height: DEFAULT_PDF_PAGE_HEIGHT,
  });
  const [renderStatus, setRenderStatus] = useState<PdfPageRenderStatus>("idle");

  const scale = zoomMode === "fit-width"
    ? fitPdfWidth(availableWidth, dimensions.width)
    : clampPdfZoom(manualZoom);
  const viewportWidth = dimensions.width * scale;
  const viewportHeight = dimensions.height * scale;

  const setContainer = useCallback((element: HTMLDivElement | null) => {
    containerRef.current = element;
    onElementChange(pageNumber, element);
  }, [onElementChange, pageNumber]);

  useEffect(() => () => onElementChange(pageNumber, null), [onElementChange, pageNumber]);

  useEffect(() => {
    const element = containerRef.current;
    if (!element) return;
    if (!scrollRoot) return;

    const observer = new IntersectionObserver(
      ([entry]) => setIsNearViewport(Boolean(entry?.isIntersecting)),
      { root: scrollRoot, rootMargin: "1000px 0px", threshold: 0 },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [scrollRoot]);

  useEffect(() => {
    let active = true;
    setLoadedPage(null);
    documentRef.current = null;
    setRenderStatus("idle");
    setDimensions({ width: DEFAULT_PDF_PAGE_WIDTH, height: DEFAULT_PDF_PAGE_HEIGHT });

    if (!isNearViewport) return () => { active = false; };

    const loadPage = async () => {
      try {
        const page = await pdfDocument.getPage(pageNumber);
        if (!active) {
          page.cleanup();
          return;
        }
        const viewport = page.getViewport({ scale: 1 });
        setLoadedPage(page);
        documentRef.current = pdfDocument;
        setDimensions({ width: viewport.width, height: viewport.height });
        setRenderStatus("idle");
      } catch {
        if (active) setRenderStatus("error");
      }
    };

    void loadPage();
    return () => {
      active = false;
      documentRef.current = null;
    };
  }, [isNearViewport, pageNumber, pdfDocument]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!isNearViewport || !loadedPage || documentRef.current !== pdfDocument || !canvas) return;

    let active = true;
    let renderTask: RenderTask | null = null;
    setRenderStatus("loading");
    const previousRender = renderSerialRef.current;

    const renderPage = async () => {
      try {
        await previousRender.catch(() => undefined);
        if (!active) return;
        const viewport = loadedPage.getViewport({ scale });
        const deviceScale = Math.max(1, Math.min(window.devicePixelRatio || 1, 2.5));
        const pixelWidth = Math.max(1, Math.floor(viewport.width * deviceScale));
        const pixelHeight = Math.max(1, Math.floor(viewport.height * deviceScale));
        canvas.width = pixelWidth;
        canvas.height = pixelHeight;
        canvas.style.width = `${viewport.width}px`;
        canvas.style.height = `${viewport.height}px`;

        const context = canvas.getContext("2d", { alpha: false });
        if (!context) throw new Error("无法创建 PDF 画布");
        context.save();
        context.fillStyle = "#ffffff";
        context.fillRect(0, 0, pixelWidth, pixelHeight);
        context.restore();

        renderTask = loadedPage.render({
          canvas: null,
          canvasContext: context,
          viewport,
          transform: deviceScale === 1 ? undefined : [deviceScale, 0, 0, deviceScale, 0, 0],
          background: "#ffffff",
        });
        await renderTask.promise;
        if (active) setRenderStatus("ready");
      } catch {
        if (active) setRenderStatus("error");
      }
    };

    renderSerialRef.current = renderPage().catch(() => undefined);
    return () => {
      active = false;
      renderTask?.cancel();
    };
  }, [isNearViewport, loadedPage, pdfDocument, scale]);

  return (
    <div
      ref={setContainer}
      className={`pdf-page-shell ${renderStatus === "loading" ? "is-rendering" : ""}`}
      style={{ width: viewportWidth, minHeight: viewportHeight, height: viewportHeight }}
      data-page-number={pageNumber}
    >
      <canvas ref={canvasRef} className="pdf-page-canvas" aria-label={`第 ${pageNumber} 页`} />
      {renderStatus === "idle" && <span className="pdf-page-placeholder" aria-hidden="true" />}
      {renderStatus === "loading" && (
        <span className="pdf-page-status" aria-label="正在渲染页面">
          <LoaderCircle className="spin" size={16} />
        </span>
      )}
      {renderStatus === "error" && (
        <span className="pdf-page-error" role="img" aria-label="页面渲染失败">页面渲染失败</span>
      )}
    </div>
  );
}

export default function PdfReader({
  file,
  onReplace,
  onRemove,
  engineStatus,
  engineStatusLoading,
  enginePreparing,
  engineProgress,
  engineError,
  job,
  jobEventsReady,
  jobEventsError,
  translationEnabled,
  onPrepareEngine,
  onStartTranslation,
  onCancelTranslation,
}: PdfReaderProps) {
  const [status, setStatus] = useState<PdfReaderStatus>("loading");
  const [error, setError] = useState<string | null>(null);
  const [pdfDocument, setPdfDocument] = useState<PDFDocumentProxy | null>(null);
  const [loadingProgress, setLoadingProgress] = useState<number | null>(null);
  const [retryToken, setRetryToken] = useState(0);
  const [currentPage, setCurrentPage] = useState(1);
  const [pageInput, setPageInput] = useState("1");
  const [zoomMode, setZoomMode] = useState<PdfZoomMode>("fit-width");
  const [manualZoom, setManualZoom] = useState(1);
  const [availableWidth, setAvailableWidth] = useState(0);
  const [scrollRoot, setScrollRoot] = useState<HTMLDivElement | null>(null);
  const loadingTaskRef = useRef<PDFDocumentLoadingTask | null>(null);
  const documentRef = useRef<PDFDocumentProxy | null>(null);
  const pageElementsRef = useRef(new Map<number, HTMLDivElement>());

  const handlePageElementChange = useCallback((pageNumber: number, element: HTMLDivElement | null) => {
    if (element) pageElementsRef.current.set(pageNumber, element);
    else pageElementsRef.current.delete(pageNumber);
  }, []);

  useEffect(() => {
    let active = true;
    setStatus("loading");
    setError(null);
    setPdfDocument(null);
    setLoadingProgress(null);
    setCurrentPage(1);
    setPageInput("1");
    setZoomMode("fit-width");
    pageElementsRef.current.clear();

    const loadDocument = async () => {
      try {
        const response = await invokeCommand<unknown>("read_pdf_bytes", { filePath: file.path });
        const bytes = toPdfBytes(response);
        if (bytes.byteLength === 0) throw new Error("PDF 文件为空");
        const loadingTask = loadPdfDocument(bytes);
        loadingTaskRef.current = loadingTask;
        loadingTask.onProgress = ({ loaded, total }: { loaded: number; total: number }) => {
          if (!active || !total) return;
          setLoadingProgress(Math.min(100, Math.round((loaded / total) * 100)));
        };
        const document = await loadingTask.promise;
        if (!active) {
          await document.cleanup();
          return;
        }
        documentRef.current = document;
        setPdfDocument(document);
        setStatus("ready");
      } catch (reason) {
        if (!active) return;
        setStatus("error");
        setError(describeError(reason, "读取 PDF 失败"));
      }
    };

    void loadDocument();
    return () => {
      active = false;
      const loadingTask = loadingTaskRef.current;
      const document = documentRef.current;
      loadingTaskRef.current = null;
      documentRef.current = null;
      setPdfDocument(null);
      void loadingTask?.destroy().catch(() => undefined);
      void document?.cleanup().catch(() => undefined);
    };
  }, [file.path, retryToken]);

  useEffect(() => {
    if (!scrollRoot) return;
    const updateWidth = () => setAvailableWidth(Math.max(0, scrollRoot.clientWidth - 52));
    updateWidth();
    const observer = new ResizeObserver(updateWidth);
    observer.observe(scrollRoot);
    return () => observer.disconnect();
  }, [scrollRoot]);

  useEffect(() => {
    if (!scrollRoot || !pdfDocument) return;
    let animationFrame = 0;
    const updateCurrentPage = () => {
      animationFrame = 0;
      const rootRect = scrollRoot.getBoundingClientRect();
      const targetY = rootRect.top + rootRect.height * 0.35;
      let candidate = 1;
      let nearestDistance = Number.POSITIVE_INFINITY;
      pageElementsRef.current.forEach((element, pageNumber) => {
        const rect = element.getBoundingClientRect();
        if (rect.top <= targetY && rect.bottom >= targetY) {
          candidate = pageNumber;
          nearestDistance = 0;
          return;
        }
        const distance = Math.min(Math.abs(rect.top - targetY), Math.abs(rect.bottom - targetY));
        if (distance < nearestDistance) {
          nearestDistance = distance;
          candidate = pageNumber;
        }
      });
      setCurrentPage((current) => current === candidate ? current : candidate);
    };
    const handleScroll = () => {
      if (animationFrame === 0) animationFrame = requestAnimationFrame(updateCurrentPage);
    };
    scrollRoot.addEventListener("scroll", handleScroll, { passive: true });
    updateCurrentPage();
    return () => {
      scrollRoot.removeEventListener("scroll", handleScroll);
      if (animationFrame) cancelAnimationFrame(animationFrame);
    };
  }, [pdfDocument, scrollRoot]);

  useEffect(() => {
    setPageInput(String(currentPage));
  }, [currentPage]);

  const totalPages = pdfDocument?.numPages ?? 0;
  const zoomLabel = zoomMode === "fit-width" ? "适合宽度" : `${Math.round(manualZoom * 100)}%`;

  const goToPage = useCallback((page: number) => {
    const nextPage = clampPdfPage(page, totalPages);
    const element = pageElementsRef.current.get(nextPage);
    if (element) {
      element.scrollIntoView({ behavior: "smooth", block: "start" });
      setCurrentPage(nextPage);
      setPageInput(String(nextPage));
    }
  }, [totalPages]);

  const submitPage = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const page = Number.parseInt(pageInput, 10);
    if (Number.isFinite(page)) goToPage(page);
    else setPageInput(String(currentPage));
  };

  const changeZoom = (delta: number) => {
    setZoomMode("manual");
    setManualZoom((current) => clampPdfZoom(Math.round((current + delta) * 20) / 20));
  };

  const resetReader = () => {
    setZoomMode("manual");
    setManualZoom(1);
  };

  const setReaderScrollRoot = useCallback((element: HTMLDivElement | null) => {
    setScrollRoot(element);
  }, []);

  const pageNumbers = useMemo(() => (
    totalPages > 0 ? Array.from({ length: totalPages }, (_, index) => index + 1) : []
  ), [totalPages]);

  return (
    <section className="pdf-reader-shell" aria-label="PDF 阅读器">
      <div className="pdf-reader-toolbar">
        <div className="pdf-reader-file" title={file.fileName}>
          <span className="pdf-reader-file-icon" aria-hidden="true"><FileType2 size={16} /></span>
          <span>{file.fileName}</span>
        </div>
        {status === "ready" && (
          <div className="pdf-reader-controls" aria-label="阅读器控制">
            <div className="pdf-page-control">
              <button className="pdf-toolbar-button" type="button" onClick={() => goToPage(currentPage - 1)} disabled={currentPage <= 1} aria-label="上一页" title="上一页"><ChevronLeft size={15} /></button>
              <form onSubmit={submitPage}>
                <input
                  value={pageInput}
                  onChange={(event) => setPageInput(event.target.value)}
                  aria-label="当前页码"
                  inputMode="numeric"
                />
                <span>/ {totalPages}</span>
              </form>
              <button className="pdf-toolbar-button" type="button" onClick={() => goToPage(currentPage + 1)} disabled={currentPage >= totalPages} aria-label="下一页" title="下一页"><ChevronRight size={15} /></button>
            </div>
            <div className="pdf-zoom-control">
              <button className="pdf-toolbar-button" type="button" onClick={() => changeZoom(-0.1)} disabled={zoomMode === "manual" && manualZoom <= MIN_PDF_ZOOM} aria-label="缩小" title="缩小"><ZoomOut size={15} /></button>
              <span>{zoomLabel}</span>
              <button className="pdf-toolbar-button" type="button" onClick={() => changeZoom(0.1)} disabled={zoomMode === "manual" && manualZoom >= MAX_PDF_ZOOM} aria-label="放大" title="放大"><ZoomIn size={15} /></button>
              <button className={`pdf-toolbar-button ${zoomMode === "manual" && manualZoom === 1 ? "is-active" : ""}`} type="button" onClick={resetReader} aria-label="重置缩放" title="重置缩放"><RotateCcw size={14} /></button>
              <button className={`pdf-toolbar-button ${zoomMode === "fit-width" ? "is-active" : ""}`} type="button" onClick={() => setZoomMode("fit-width")} aria-label="适合宽度" title="适合宽度"><Maximize2 size={14} /></button>
            </div>
          </div>
        )}
        <div className="pdf-reader-actions">
          <button className="pdf-toolbar-button" type="button" onClick={onReplace} aria-label="替换 PDF" title="替换 PDF"><Upload size={15} /></button>
          <button className="pdf-toolbar-button danger-icon-button" type="button" onClick={onRemove} aria-label="移除 PDF" title="移除 PDF"><Trash2 size={15} /></button>
        </div>
      </div>

      <PdfTaskPanel
        readerStatus={status}
        engineStatus={engineStatus}
        engineStatusLoading={engineStatusLoading}
        enginePreparing={enginePreparing}
        engineProgress={engineProgress}
        engineError={engineError}
        job={job}
        jobEventsReady={jobEventsReady}
        jobEventsError={jobEventsError}
        translationEnabled={translationEnabled}
        onPrepareEngine={onPrepareEngine}
        onStartTranslation={onStartTranslation}
        onCancelTranslation={onCancelTranslation}
      />

      {status === "loading" && (
        <div className="pdf-reader-state" role="status">
          <LoaderCircle className="spin" size={19} />
          <span>正在读取 PDF{loadingProgress === null ? "" : ` · ${loadingProgress}%`}</span>
        </div>
      )}
      {status === "error" && (
        <div className="pdf-reader-state pdf-reader-error-state" role="alert">
          <span>{error ?? "PDF 读取失败"}</span>
          <div className="button-group">
            <button className="secondary-button small-button" type="button" onClick={() => setRetryToken((current) => current + 1)}>重试</button>
            <button className="secondary-button small-button" type="button" onClick={onReplace}>替换文件</button>
          </div>
        </div>
      )}
      {status === "ready" && pdfDocument && (
        <div ref={setReaderScrollRoot} className="pdf-reader-scroll">
          <div className="pdf-document-pages">
            {pageNumbers.map((pageNumber) => (
              <PdfPage
                key={`${file.path}-${retryToken}-${pageNumber}`}
                pdfDocument={pdfDocument}
                pageNumber={pageNumber}
                zoomMode={zoomMode}
                manualZoom={manualZoom}
                availableWidth={availableWidth}
                scrollRoot={scrollRoot}
                onElementChange={handlePageElementChange}
              />
            ))}
          </div>
        </div>
      )}
    </section>
  );
}
