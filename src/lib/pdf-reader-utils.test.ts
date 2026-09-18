import { describe, expect, it, vi } from "vitest";
import {
  clampPdfPage,
  clampPdfPreflightPageLimit,
  collectPdfPreflightSamples,
  clampPdfZoom,
  fitPdfWidth,
  toPdfBytes,
} from "./pdf-reader-utils";

describe("PDF reader helpers", () => {
  it("normalizes binary command responses", () => {
    expect(Array.from(toPdfBytes(new Uint8Array([1, 2, 3])))).toEqual([1, 2, 3]);
    expect(Array.from(toPdfBytes(new Uint8Array([4, 5]).buffer))).toEqual([4, 5]);
    expect(Array.from(toPdfBytes([6, 7, 8]))).toEqual([6, 7, 8]);
  });

  it("clamps page numbers and zoom values", () => {
    expect(clampPdfPage(0, 5)).toBe(1);
    expect(clampPdfPage(3.6, 5)).toBe(4);
    expect(clampPdfPage(9, 5)).toBe(5);
    expect(clampPdfZoom(0.1)).toBe(0.5);
    expect(clampPdfZoom(4)).toBe(2.5);
    expect(clampPdfPreflightPageLimit(0)).toBe(1);
    expect(clampPdfPreflightPageLimit(10.6)).toBe(11);
    expect(clampPdfPreflightPageLimit(200)).toBe(100);
  });

  it("samples page text with page, sample, and character bounds", async () => {
    const getPage = vi.fn().mockImplementation(async (pageNumber: number) => {
      if (pageNumber === 2) throw new Error("没有文本层");
      return {
        getTextContent: vi.fn().mockResolvedValue({
          items: [{ str: `page ${pageNumber} text` }],
        }),
      };
    });
    const result = await collectPdfPreflightSamples(
      { numPages: 4, getPage } as never,
      3,
    );
    expect(getPage).toHaveBeenCalledTimes(3);
    expect(result.samples.map((sample) => sample.page_number)).toEqual([1, 3]);
    expect(result.samples[0]?.segment_id).toBe("page-1");
    expect(result.warning).toContain("第 2 页");
  });

  it("calculates a bounded fit-width scale", () => {
    expect(fitPdfWidth(612, 612)).toBe(1);
    expect(fitPdfWidth(900, 600)).toBe(1.5);
    expect(fitPdfWidth(100, 1000)).toBe(0.5);
  });
});
