import { describe, expect, it } from "vitest";
import { isReleaseNewerThan, parseReleaseVersion, selectLatestStableRelease, summarizeUpdaterUpdate } from "./release-version";

describe("release version tracking", () => {
  it("parses only stable vX.Y.Z tags", () => {
    expect(parseReleaseVersion("v0.5.3")).toEqual([0, 5, 3]);
    expect(parseReleaseVersion("v1.10.0")).toEqual([1, 10, 0]);
    expect(parseReleaseVersion("0.5.3")).toBeNull();
    expect(parseReleaseVersion("v0.5.3-beta.1")).toBeNull();
  });

  it("selects the greatest stable release and ignores drafts and prereleases", () => {
    const release = selectLatestStableRelease([
      { tag_name: "v0.9.0", html_url: "https://github.com/TTTTTony32/Lilt/releases/tag/v0.9.0" },
      { tag_name: "v1.0.0-rc.1", prerelease: true },
      { tag_name: "v1.1.0", draft: true },
      { tag_name: "v0.10.0", html_url: "https://github.com/TTTTTony32/Lilt/releases/tag/v0.10.0" },
    ]);

    expect(release).toMatchObject({
      tagName: "v0.10.0",
      htmlUrl: "https://github.com/TTTTTony32/Lilt/releases/tag/v0.10.0",
      version: [0, 10, 0],
    });
  });

  it("falls back to the repository release URL for malformed links", () => {
    expect(selectLatestStableRelease([{ tag_name: "v0.5.4", html_url: "https://example.com/release" }])).toMatchObject({
      htmlUrl: "https://github.com/TTTTTony32/Lilt/releases/tag/v0.5.4",
    });
  });

  it("compares the latest release with the installed version", () => {
    const release = selectLatestStableRelease([{ tag_name: "v0.5.4" }]);
    expect(release).not.toBeNull();
    expect(isReleaseNewerThan(release!, "0.5.3")).toBe(true);
    expect(isReleaseNewerThan(release!, "0.5.4")).toBe(false);
    expect(isReleaseNewerThan(release!, "0.6.0")).toBe(false);
  });

  it("summarizes updater metadata with a Release link and notes", () => {
    expect(summarizeUpdaterUpdate({
      version: "0.6.3",
      body: "修复启动问题",
      date: "2026-09-19T08:00:00Z",
    })).toEqual({
      tagName: "v0.6.3",
      htmlUrl: "https://github.com/TTTTTony32/Lilt/releases/tag/v0.6.3",
      version: [0, 6, 3],
      notes: "修复启动问题",
      date: "2026-09-19T08:00:00Z",
    });
  });

  it("rejects malformed updater metadata and supplies a fallback note", () => {
    expect(summarizeUpdaterUpdate({ version: "v0.6.3", body: "  " })).toMatchObject({
      notes: "此次版本未提供 Release 说明。",
    });
    expect(summarizeUpdaterUpdate({ version: "0.6.3-beta.1" })).toBeNull();
  });
});
