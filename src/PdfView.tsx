import { lazy, Suspense, useCallback, useEffect, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { FileType2, Upload } from "lucide-react";
import { describeError } from "./lib/errors";
import { type PdfFile, validatePdfPath } from "./lib/pdf";

const PdfReader = lazy(() => import("./PdfReader"));

const MULTIPLE_FILES_ERROR = "当前只支持单个 PDF 文件，请一次拖放一个文件。";
const INVALID_FILE_ERROR = "请选择 PDF 文件，文件扩展名必须为 .pdf。";
const EMPTY_PATH_ERROR = "未找到 PDF 文件路径，请重试。";

export default function PdfView() {
  const [selectedFile, setSelectedFile] = useState<PdfFile | null>(null);
  const [dragging, setDragging] = useState(false);
  const [error, setError] = useState<string | null>(null);

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

    setSelectedFile(file);
    setError(null);
  }, []);

  const handleDrop = useCallback((paths: string[]) => {
    setDragging(false);
    if (paths.length !== 1) {
      setError(MULTIPLE_FILES_ERROR);
      return;
    }
    acceptPath(paths[0]);
  }, [acceptPath]);

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
    setSelectedFile(null);
    setDragging(false);
    setError(null);
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
            <PdfReader file={selectedFile} onReplace={() => void chooseFile()} onRemove={removeFile} />
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
            <div className="card-heading">
              <div>
                <strong>PDF 阅读器</strong>
                <span>选择文件后在当前窗口连续阅读。</span>
              </div>
              <span className="connection-status">第二阶段</span>
            </div>
            <p className="pdf-stage-description">阅读器只读取当前会话中选定的文件，不保存 PDF 内容，也不会创建翻译任务。</p>
          </div>
        </>
      )}
    </section>
  );
}
