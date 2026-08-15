import * as pdfjsLib from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import type {
  PDFDocumentLoadingTask,
  PDFDocumentProxy,
  PDFPageProxy,
  RenderTask,
} from "pdfjs-dist";

pdfjsLib.GlobalWorkerOptions.workerSrc = workerUrl;

export type { PDFDocumentLoadingTask, PDFDocumentProxy, PDFPageProxy, RenderTask };

export function loadPdfDocument(data: Uint8Array): PDFDocumentLoadingTask {
  return pdfjsLib.getDocument({ data });
}
