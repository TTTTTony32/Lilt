export interface PdfPageRangeSuccess {
  pages: string | null;
  error: null;
}

export interface PdfPageRangeFailure {
  pages: null;
  error: string;
}

export type PdfPageRangeResult = PdfPageRangeSuccess | PdfPageRangeFailure;

function invalidPageNumber(): PdfPageRangeFailure {
  return { pages: null, error: "页码必须是从 1 开始的正整数。" };
}

export function parsePdfPageRange(input: string, totalPages: number | null = null): PdfPageRangeResult {
  const value = input.trim();
  if (!value) return { pages: null, error: null };

  const match = /^(\d+)(?:\s*-\s*(\d+))?$/.exec(value);
  if (!match) {
    return { pages: null, error: "请输入单个页码或页码范围，例如 3 或 3-7。" };
  }

  const start = Number(match[1]);
  const end = match[2] === undefined ? start : Number(match[2]);
  if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || start < 1 || end < 1) {
    return invalidPageNumber();
  }
  if (start > end) {
    return { pages: null, error: "页码范围的起始页不能大于结束页。" };
  }
  if (totalPages !== null && totalPages > 0 && end > totalPages) {
    return { pages: null, error: `PDF 共 ${totalPages} 页，页码不能超过文档总页数。` };
  }

  return {
    pages: match[2] === undefined ? String(start) : `${start}-${end}`,
    error: null,
  };
}
