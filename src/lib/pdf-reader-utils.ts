export const DEFAULT_PDF_PAGE_WIDTH = 612;
export const DEFAULT_PDF_PAGE_HEIGHT = 792;
export const MIN_PDF_ZOOM = 0.5;
export const MAX_PDF_ZOOM = 2.5;

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

export function fitPdfWidth(containerWidth: number, pageWidth: number): number {
  if (containerWidth <= 0 || pageWidth <= 0) return 1;
  return clampPdfZoom(containerWidth / pageWidth);
}
