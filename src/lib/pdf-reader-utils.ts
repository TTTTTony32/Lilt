import type { PDFDocumentProxy } from "pdfjs-dist";

export const DEFAULT_PDF_PAGE_WIDTH = 612;
export const DEFAULT_PDF_PAGE_HEIGHT = 792;
export const MIN_PDF_ZOOM = 0.5;
export const MAX_PDF_ZOOM = 2.5;
export const DEFAULT_PDF_PREFLIGHT_PAGE_LIMIT = 10;
export const MIN_PDF_PREFLIGHT_PAGE_LIMIT = 1;
export const MAX_PDF_PREFLIGHT_PAGE_LIMIT = 100;
export const MAX_PDF_PREFLIGHT_SAMPLES = 12;
export const MAX_PDF_PREFLIGHT_SAMPLE_CHARS = 24_000;

export interface PdfPreflightSample {
  segment_id: string;
  source_text: string;
  placeholders: string[];
  page_number: number;
}

export interface PdfPreflightSampleCollection {
  samples: PdfPreflightSample[];
  warning: string | null;
}

export function toPdfBytes(value: unknown): Uint8Array {
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  }
  if (Array.isArray(value) && value.every((item) => Number.isInteger(item) && item >= 0 && item <= 255)) {
    return Uint8Array.from(value);
  }
  throw new Error("PDF 文件数据格式无效");
}
export function clampPdfPage(page: number, totalPages: number): number {
  if (totalPages <= 0) return 1;
  return Math.min(Math.max(Math.round(page), 1), totalPages);
}

export function clampPdfZoom(zoom: number): number {
  return Math.min(Math.max(zoom, MIN_PDF_ZOOM), MAX_PDF_ZOOM);
}

export function clampPdfPreflightPageLimit(pageLimit: number): number {
  const normalized = Number.isFinite(pageLimit)
    ? Math.round(pageLimit)
    : DEFAULT_PDF_PREFLIGHT_PAGE_LIMIT;
  return Math.min(
    Math.max(normalized, MIN_PDF_PREFLIGHT_PAGE_LIMIT),
    MAX_PDF_PREFLIGHT_PAGE_LIMIT,
  );
}

export async function collectPdfPreflightSamples(
  pdfDocument: PDFDocumentProxy,
  pageLimit: number,
): Promise<PdfPreflightSampleCollection> {
  const samples: PdfPreflightSample[] = [];
  const pagesToRead = Math.min(clampPdfPreflightPageLimit(pageLimit), pdfDocument.numPages);
  let remainingChars = MAX_PDF_PREFLIGHT_SAMPLE_CHARS;
  let warning: string | null = null;

  for (let pageNumber = 1; pageNumber <= pagesToRead; pageNumber += 1) {
    if (samples.length >= MAX_PDF_PREFLIGHT_SAMPLES || remainingChars <= 0) break;

    try {
      const page = await pdfDocument.getPage(pageNumber);
      const textContent = await page.getTextContent();
      const sourceText = textContent.items
        .map((item) => ("str" in item ? item.str : ""))
        .join(" ")
        .replace(/\s+/g, " ")
        .trim();
      if (!sourceText) continue;

      const boundedText = Array.from(sourceText).slice(0, remainingChars).join("");
      samples.push({
        segment_id: `page-${pageNumber}`,
        source_text: boundedText,
        placeholders: [],
        page_number: pageNumber,
      });
      remainingChars -= Array.from(boundedText).length;
    } catch (error) {
      if (!warning) {
        const detail = error instanceof Error && error.message ? `：${error.message}` : "";
        warning = `第 ${pageNumber} 页文本提取失败${detail}，将继续执行翻译。`;
      }
    }
  }

  return { samples, warning };
}

export function fitPdfWidth(containerWidth: number, pageWidth: number): number {
  if (containerWidth <= 0 || pageWidth <= 0) return 1;
  return clampPdfZoom(containerWidth / pageWidth);
}
