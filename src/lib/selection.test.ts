import { describe, expect, it } from "vitest";
import { isDictionarySelection, routeSelection } from "./selection";

describe("selection routing", () => {
  it("routes English word shapes to the dictionary", () => {
    expect(isDictionarySelection("Running")).toBe(true);
    expect(isDictionarySelection("mother-in-law")).toBe(true);
    expect(isDictionarySelection("don't")).toBe(true);
    expect(routeSelection("  Running  ")).toBe("dictionary");
  });

  it("routes sentences and unsupported tokens to paragraph translation", () => {
    expect(routeSelection("a useful phrase")).toBe("paragraph");
    expect(routeSelection("你好")).toBe("paragraph");
    expect(routeSelection("v2")).toBe("paragraph");
    expect(routeSelection("")).toBe("paragraph");
  });
});
