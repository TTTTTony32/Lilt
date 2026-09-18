import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ChevronLeft,
  ChevronRight,
  ChevronDown,
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
  collectPdfPreflightSamples,
  type PdfPreflightSample,
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
import { createEmptyPdfPreflightState } from "./lib/pdf-preflight";
import type {
  DocumentContext,
  PdfJobLogKind,
  PdfJobUiState,
  PdfPreflightState,
  PdfQualityDiagnostic,
} from "./types/contracts";

type PdfReaderStatus = "loading" | "ready" | "error";
type PdfZoomMode = "fit-width" | "manual";
type PdfPageRenderStatus = "idle" | "loading" | "ready" | "error";

interface PdfReaderProps {
  file: PdfFile;
  onReplace: () => void;
  onRemove: () => void;
  job: PdfJobUiState;
  jobEventsReady: boolean;
  jobEventsError: string | null;
  translationEnabled: boolean;
  pdfPreflightEnabled: boolean;
  pdfPreflightPageLimit: number;
  onPdfPreflightEnabledChange: (enabled: boolean) => void;
  onStartTranslation: (samples: PdfPreflightSample[], warning: string | null) => void;
  onCancelTranslation: () => void;
  onOpenOutputDirectory: (path: string) => void;
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

function formatPdfTaskProgressStage(stage: string | null): string {
  if (!stage) return "等待翻译";
  const normalized = stage.toLowerCase().replace(/[-\s]+/g, "_");
  if (normalized.includes("preflight")) return "预检";
  if (/(translate|translation|segment)/.test(normalized)) return "分段翻译中";
  if (/(assemble|render|output|finish)/.test(normalized)) return "生成 PDF";
  return formatPdfStage(stage);
}

function formatPdfTaskProgressDetail(
  job: PdfJobUiState,
  preflight: PdfPreflightState,
  preflightEnabled: boolean,
): string {
  if (preflightEnabled && preflight.status === "running") {
    const phase = preflight.responsePhase === "waiting"
      ? "等待模型响应"
      : preflight.responsePhase === "thinking"
        ? "模型思考中"
        : preflight.responsePhase === "streaming"
          ? "生成预检结果"
          : "分析文档";
    return `预检·${phase}`;
  }
  if (job.progress) {
    const stage = formatPdfTaskProgressStage(job.progress.stage);
    if (job.progress.current !== null && job.progress.total !== null) {
      return `${stage}·${job.progress.current}/${job.progress.total}`;
    }
    return job.progress.message ? `${stage}·${job.progress.message}` : stage;
  }
  return jobStatusLabel(job.status);
}

function pdfJobLogKindLabel(kind: PdfJobLogKind): string {
  switch (kind) {
    case "system": return "系统";
    case "stage": return "阶段";
    case "progress": return "进度";
    case "preflight": return "预检";
    case "warning": return "警告";
    case "quality": return "质检";
    case "result": return "结果";
  }
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

function preflightStatusLabel(preflight: PdfPreflightState): string {
  switch (preflight.status) {
    case "running":
      switch (preflight.responsePhase) {
        case "waiting": return "等待模型响应";
        case "thinking": return "模型思考中";
        case "streaming": return "正在生成预检结果";
        default: return "正在分析文档";
      }
    case "completed": return "预检完成";
    case "degraded": return "已降级应用";
    case "failed": return "预检失败，继续翻译";
    default: return "等待预检";
  }
}

function diagnosticSeverityLabel(severity: PdfQualityDiagnostic["severity"]): string {
  switch (severity) {
    case "error": return "错误";
    case "warning": return "警告";
    default: return "提示";
  }
}

function contextTitle(context: DocumentContext | null): string {
  return context?.title?.trim() || "未识别标题";
}

function PdfContextSummary({ context }: { context: DocumentContext }) {
  const metadata = [
    context.documentType ? `类型：${context.documentType}` : null,
    context.domain ? `领域：${context.domain}` : null,
    `术语 ${context.keyTerms.length}`,
    `缩写 ${context.abbreviations.length}`,
    context.headings.length > 0 ? `标题层级 ${context.headings.length}` : null,
  ].filter((item): item is string => item !== null);

  return (
    <details className="pdf-task-context-details">
      <summary>查看文档上下文 · {contextTitle(context)}</summary>
      <div className="pdf-task-context-summary">
        <p className="pdf-task-panel-meta">{metadata.join(" · ")}</p>
        {context.abstract && <p className="pdf-task-panel-message">{context.abstract}</p>}
        {context.translationNotes.length > 0 && (
          <div>
            <strong>翻译注意事项</strong>
            <ul className="pdf-task-warnings">
              {context.translationNotes.map((note) => <li key={note}>{note}</li>)}
            </ul>
          </div>
        )}
        {context.keyTerms.length > 0 && (
          <p className="pdf-task-panel-meta">任务术语：{context.keyTerms.map((term) => term.target ? `${term.source} → ${term.target}` : term.source).join("、")}</p>
        )}
        {context.abbreviations.length > 0 && (
          <p className="pdf-task-panel-meta">任务缩写：{context.abbreviations.map((item) => item.expanded ? `${item.abbreviation}（${item.expanded}）` : item.abbreviation).join("、")}</p>
        )}
      </div>
    </details>
  );
}

function PdfQualityDiagnostics({ diagnostics }: { diagnostics: PdfQualityDiagnostic[] }) {
  if (diagnostics.length === 0) return null;
  return (
    <div className="pdf-task-panel-section pdf-task-diagnostics-section">
      <div className="pdf-task-panel-heading">
        <div>
          <span className="pdf-task-panel-kicker">QUALITY CONTROL</span>
          <strong>质量诊断 · {diagnostics.length}</strong>
        </div>
      </div>
      <details>
        <summary>查看诊断明细</summary>
        <ul className="pdf-task-warnings">
          {diagnostics.map((diagnostic, index) => {
            const location = [
              diagnostic.pageNumber === null ? null : `第 ${diagnostic.pageNumber} 页`,
              diagnostic.segmentId ? `段落 ${diagnostic.segmentId}` : null,
            ].filter((item): item is string => item !== null).join(" · ");
            return (
              <li key={`${diagnosticKeyForRender(diagnostic)}-${index}`}>
                <strong>{diagnosticSeverityLabel(diagnostic.severity)}</strong>
                {diagnostic.ruleId && ` · ${diagnostic.ruleId}`}：{diagnostic.message}
                {location && <small> · {location}</small>}
              </li>
            );
          })}
        </ul>
      </details>
    </div>
  );
}

function diagnosticKeyForRender(diagnostic: PdfQualityDiagnostic): string {
  return [diagnostic.ruleId ?? "diagnostic", diagnostic.message, diagnostic.segmentId ?? "", diagnostic.pageNumber ?? ""].join("-");
}

interface PdfTaskPanelProps {
  readerStatus: PdfReaderStatus;
  job: PdfJobUiState;
  jobEventsReady: boolean;
  jobEventsError: string | null;
  translationEnabled: boolean;
  pdfPreflightEnabled: boolean;
  pdfPreflightPageLimit: number;
  preflightSampling: boolean;
  onPdfPreflightEnabledChange: (enabled: boolean) => void;
  onStartTranslation: () => void;
  onCancelTranslation: () => void;
  onOpenOutputDirectory: (path: string) => void;
}

function PdfTaskPanel({
  readerStatus,
  job,
  jobEventsReady,
  jobEventsError,
  translationEnabled,
  pdfPreflightEnabled,
  pdfPreflightPageLimit,
  preflightSampling,
  onPdfPreflightEnabledChange,
  onStartTranslation,
  onCancelTranslation,
  onOpenOutputDirectory,
}: PdfTaskPanelProps) {
  const jobBusy = job.status === "starting" || job.status === "running" || job.status === "cancelling";
  const canStart = translationEnabled && readerStatus === "ready" && !jobBusy;
  const jobProgressValue = progressPercent(job.progress);
  const preflight = job.preflight ?? createEmptyPdfPreflightState();
  const context = job.documentContext ?? preflight.context;
  const diagnostics = job.diagnostics ?? [];
  const [detailsOpen, setDetailsOpen] = useState(true);
  const detailsId = "pdf-task-panel-details";
  const progressDetail = formatPdfTaskProgressDetail(job, preflight, pdfPreflightEnabled);
  const preflightLabel = pdfPreflightEnabled
    ? `${preflightStatusLabel(preflight)} · 前 ${pdfPreflightPageLimit} 页`
    : "未启用";

  return (
    <div className="pdf-task-panel">
      <div className="pdf-task-panel-topbar">
        <button className="primary-button small-button" type="button" onClick={onStartTranslation} disabled={!canStart}>
          {job.status === "completed" ? "再次翻译" : "开始翻译"}
        </button>
        <div className="pdf-task-panel-progress" aria-live="polite">
          <div className="pdf-task-panel-heading">
            <div>
              <span className="pdf-task-panel-kicker">PDF TRANSLATION</span>
              <strong>{jobStatusLabel(job.status)}</strong>
            </div>
          </div>
          <div className="pdf-task-progress-block">
            <div className="pdf-task-progress-track" role="progressbar" aria-label="PDF 翻译进度" aria-valuemin={0} aria-valuemax={100} aria-valuenow={jobProgressValue ?? undefined} aria-valuetext={progressDetail}>
              {jobProgressValue !== null && <span style={{ width: `${jobProgressValue}%` }} />}
            </div>
            <div className="pdf-task-progress-label">
              <span>{progressDetail}</span>
              {jobProgressValue !== null && <span>{jobProgressValue}%</span>}
            </div>
          </div>
        </div>
        <div className="pdf-task-panel-topbar-actions">
          <label className="pdf-preflight-toggle">
            <input
              type="checkbox"
              checked={pdfPreflightEnabled}
              disabled={jobBusy || preflightSampling}
              onChange={(event) => onPdfPreflightEnabledChange(event.target.checked)}
            />
            <span>预检</span>
          </label>
          <button
            className="pdf-task-details-toggle"
            type="button"
            aria-expanded={detailsOpen}
            aria-controls={detailsId}
            aria-label={detailsOpen ? "收起 PDF 任务详情" : "展开 PDF 任务详情"}
            title={detailsOpen ? "收起任务详情" : "展开任务详情"}
            onClick={() => setDetailsOpen((open) => !open)}
          >
            <ChevronDown size={17} aria-hidden="true" className={detailsOpen ? "is-open" : ""} />
          </button>
        </div>
      </div>
      {detailsOpen && (
        <div className="pdf-task-panel-details" id={detailsId}>
          <div className="pdf-task-panel-section pdf-task-job-section">
            <div className="pdf-task-panel-heading">
              <div>
                <span className="pdf-task-panel-kicker">TRANSLATION DETAILS</span>
                <strong>任务详情</strong>
              </div>
              <div className="pdf-task-panel-actions">
                {jobBusy && job.status !== "cancelling" && (
                  <button className="secondary-button small-button" type="button" onClick={onCancelTranslation} disabled={!job.taskId}>
                    停止翻译
                  </button>
                )}
                {job.status === "cancelling" && <span className="pdf-task-action-status">正在停止翻译</span>}
              </div>
            </div>
            {!jobEventsReady && <p className="pdf-task-error">{jobEventsError ?? "PDF 任务事件监听尚未就绪。"}</p>}
            {job.stage && <p className="pdf-task-panel-stage">{formatPdfStage(job.stage)}</p>}
            {job.message && <p className={job.status === "failed" ? "pdf-task-error" : "pdf-task-panel-message"} role={job.status === "failed" ? "alert" : undefined}>{job.message}</p>}
            {job.code && <p className="pdf-task-panel-meta">错误代码：{job.code}</p>}
          </div>

          <div className="pdf-task-panel-section pdf-task-log-section">
            <div className="pdf-task-panel-heading">
              <div>
                <span className="pdf-task-panel-kicker">TASK LOG</span>
                <strong>任务日志 · {job.logs.length}</strong>
              </div>
            </div>
            <div className="pdf-task-log" role="log" aria-live="polite" aria-label="PDF 任务日志">
              {job.logs.length === 0 ? (
                <p className="pdf-task-panel-meta">暂无任务日志。</p>
              ) : (
                <ol>
                  {job.logs.map((entry) => (
                    <li key={entry.seq} className={`pdf-task-log-entry is-${entry.level}`}>
                      <span className="pdf-task-log-kind">{pdfJobLogKindLabel(entry.kind)}</span>
                      <span>{entry.message}</span>
                    </li>
                  ))}
                </ol>
              )}
            </div>
          </div>

          <div className="pdf-task-panel-section pdf-task-output-section">
            {job.status === "completed" && job.outputPdf ? (
              <div className="pdf-task-output">
                <strong>输出已生成</strong>
                <button className="pdf-task-output-path" type="button" onClick={() => onOpenOutputDirectory(job.outputPdf!)} title="打开文件所在目录">
                  {job.outputPdf}
                </button>
                <small>{[job.outputMode, job.pageCount === null ? null : `${job.pageCount} 页`].filter((item): item is string => item !== null).join(" · ")}</small>
              </div>
            ) : null}
            {job.warnings.length > 0 && (
              <ul className="pdf-task-warnings">
                {job.warnings.map((warning) => <li key={warning}>{warning}</li>)}
              </ul>
            )}
            {job.tokenUsage && <p className="pdf-task-panel-meta">Token：{job.tokenUsage.totalTokens ?? "—"}</p>}
          </div>

          <div className="pdf-task-panel-section pdf-task-preflight-section" aria-live="polite">
            <div className="pdf-task-panel-heading">
              <div>
                <span className="pdf-task-panel-kicker">DOCUMENT PREFLIGHT</span>
                <strong>{preflightLabel}</strong>
              </div>
              {preflight.applied && <span className="connection-status connected">自动应用</span>}
            </div>
            {preflight.message && <p className={preflight.status === "failed" ? "pdf-task-error" : "pdf-task-panel-message"}>{preflight.message}</p>}
            {!pdfPreflightEnabled && preflight.status === "idle" ? (
              <p className="pdf-task-panel-meta">文档预检未启用，不会发送预检请求，PDF 翻译仍可继续。</p>
            ) : context ? (
              <>
                <p className="pdf-task-panel-meta">文档上下文：{contextTitle(context)} · 术语 {context.keyTerms.length} · 缩写 {context.abbreviations.length}</p>
                <PdfContextSummary context={context} />
              </>
            ) : preflight.status === "running" ? (
              <p className="pdf-task-panel-meta">正在整理标题、摘要、术语和缩写，完成后会自动用于段落翻译。</p>
            ) : (
              <p className="pdf-task-panel-meta">预检结果会自动应用；没有上下文时继续使用普通 PDF 翻译。</p>
            )}
            {(preflight.contextHash || preflight.schemaVersion !== null) && (
              <p className="pdf-task-panel-meta">
                {preflight.schemaVersion === null ? null : `上下文 v${preflight.schemaVersion}`}
                {preflight.contextHash ? ` · ${preflight.contextHash}` : null}
              </p>
            )}
          </div>
          <PdfQualityDiagnostics diagnostics={diagnostics} />
        </div>
      )}
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
  job,
  jobEventsReady,
  jobEventsError,
  translationEnabled,
  pdfPreflightEnabled,
  pdfPreflightPageLimit,
  onPdfPreflightEnabledChange,
  onStartTranslation,
  onCancelTranslation,
  onOpenOutputDirectory,
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
  const [preflightSampling, setPreflightSampling] = useState(false);
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
    setPreflightSampling(false);
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

  const handleStartTranslation = useCallback(async () => {
    if (!pdfPreflightEnabled) {
      onStartTranslation([], null);
      return;
    }
    if (!pdfDocument) {
      onStartTranslation([], "PDF 文档尚未加载完成，预检将跳过文本采样并继续翻译。");
      return;
    }
    setPreflightSampling(true);
    try {
      const result = await collectPdfPreflightSamples(pdfDocument, pdfPreflightPageLimit);
      onStartTranslation(result.samples, result.warning);
    } catch (reason) {
      const detail = reason instanceof Error && reason.message ? `：${reason.message}` : "";
      onStartTranslation([], `PDF 文本采样失败${detail}，将继续执行翻译。`);
    } finally {
      setPreflightSampling(false);
    }
  }, [onStartTranslation, pdfDocument, pdfPreflightEnabled, pdfPreflightPageLimit]);

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
        job={job}
        jobEventsReady={jobEventsReady}
        jobEventsError={jobEventsError}
        translationEnabled={translationEnabled && !preflightSampling}
        pdfPreflightEnabled={pdfPreflightEnabled}
        pdfPreflightPageLimit={pdfPreflightPageLimit}
        preflightSampling={preflightSampling}
        onPdfPreflightEnabledChange={onPdfPreflightEnabledChange}
        onStartTranslation={() => void handleStartTranslation()}
        onCancelTranslation={onCancelTranslation}
        onOpenOutputDirectory={onOpenOutputDirectory}
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
