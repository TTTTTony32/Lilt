import { describe, expect, it } from "vitest";
import { parsePdfPageRange } from "./pdf-page-range";

describe("parsePdfPageRange", () => {
  it("treats an empty value as all pages", () => {
    expect(parsePdfPageRange("", 10)).toEqual({ pages: null, error: null });
    expect(parsePdfPageRange("  ", 10)).toEqual({ pages: null, error: null });
  });

  it("normalizes a single page and an inclusive range", () => {
    expect(parsePdfPageRange("003", 10)).toEqual({ pages: "3", error: null });
    expect(parsePdfPageRange(" 3 - 7 ", 10)).toEqual({ pages: "3-7", error: null });
  });

  it("rejects invalid page numbers and ranges", () => {
    expect(parsePdfPageRange("0", 10).error).toContain("正整数");
    expect(parsePdfPageRange("-1", 10).error).toContain("单个页码");
    expect(parsePdfPageRange("7-3", 10).error).toContain("起始页");
    expect(parsePdfPageRange("3,5", 10).error).toContain("单个页码");
    expect(parsePdfPageRange("3.5", 10).error).toContain("单个页码");
  });

  it("rejects a range outside the loaded document", () => {
    expect(parsePdfPageRange("3-11", 10).error).toContain("10 页");
  });
});
