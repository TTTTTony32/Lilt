export interface PdfFile {
  path: string;
  fileName: string;
}

export function extractFileName(path: string): string {
  const normalizedPath = path.trim();
  const separatorIndex = Math.max(normalizedPath.lastIndexOf("/"), normalizedPath.lastIndexOf("\\"));
  return normalizedPath.slice(separatorIndex + 1);
}

export function isPdfPath(path: string): boolean {
  const normalizedPath = path.trim();
  if (!normalizedPath) return false;

  const fileName = extractFileName(normalizedPath);
  return fileName.length > ".pdf".length && fileName.toLowerCase().endsWith(".pdf");
}

export function validatePdfPath(path: string): PdfFile | null {
  const normalizedPath = path.trim();
  if (!isPdfPath(normalizedPath)) return null;

  return {
    path: normalizedPath,
    fileName: extractFileName(normalizedPath),
  };
}
