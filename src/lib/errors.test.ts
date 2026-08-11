import { describe, expect, it } from "vitest";
import { describeError } from "./errors";

describe("describeError", () => {
  it("keeps string errors returned by Tauri commands", () => {
    expect(describeError("Provider 认证失败", "fallback")).toBe("Provider 认证失败");
  });

  it("reads message properties from structured command errors", () => {
    expect(describeError({ message: "请求超时" }, "fallback")).toBe("请求超时");
  });

  it("uses the fallback for unknown values", () => {
    expect(describeError({ detail: "unknown" }, "fallback")).toBe("fallback");
  });
});
