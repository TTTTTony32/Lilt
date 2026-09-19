import { describe, expect, it } from "vitest";
import { canStartPdfTranslation } from "./pdf-task-presentation";

describe("canStartPdfTranslation", () => {
  it("allows a ready idle task to start", () => {
    expect(canStartPdfTranslation(true, true, false, false)).toBe(true);
  });

  it("blocks starting while the preflight setting is being saved", () => {
    expect(canStartPdfTranslation(true, true, false, true)).toBe(false);
  });

  it("blocks unavailable readers and busy tasks", () => {
    expect(canStartPdfTranslation(false, true, false, false)).toBe(false);
    expect(canStartPdfTranslation(true, false, false, false)).toBe(false);
    expect(canStartPdfTranslation(true, true, true, false)).toBe(false);
  });
});
