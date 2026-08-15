import { describe, expect, it } from "vitest";
import {
  clampPdfPage,
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
  });

  it("calculates a bounded fit-width scale", () => {
    expect(fitPdfWidth(612, 612)).toBe(1);
    expect(fitPdfWidth(900, 600)).toBe(1.5);
    expect(fitPdfWidth(100, 1000)).toBe(0.5);
  });
});
