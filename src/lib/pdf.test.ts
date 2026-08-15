import { describe, expect, it } from "vitest";
import { extractFileName, isPdfPath, validatePdfPath } from "./pdf";

describe("PDF path helpers", () => {
  it("accepts PDF extensions without regard to case", () => {
    expect(isPdfPath(String.raw`C:\Documents\Paper.PDF`)).toBe(true);
    expect(validatePdfPath(" /workspace/paper.pDf ")).toEqual({
      path: "/workspace/paper.pDf",
      fileName: "paper.pDf",
    });
  });

  it("extracts a filename from Windows and POSIX paths", () => {
    expect(extractFileName(String.raw`C:\Documents\paper.pdf`)).toBe("paper.pdf");
    expect(extractFileName("/workspace/research/paper.pdf")).toBe("paper.pdf");
  });

  it("rejects empty paths, directories, and non-PDF files", () => {
    expect(isPdfPath("")).toBe(false);
    expect(isPdfPath("   ")).toBe(false);
    expect(isPdfPath("C:\\Documents\\")).toBe(false);
    expect(isPdfPath("/workspace/research/paper.docx")).toBe(false);
    expect(validatePdfPath("/workspace/research/paper.docx")).toBeNull();
  });
});
