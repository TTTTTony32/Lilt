export interface GitHubReleaseSummary {
  tagName: string;
  htmlUrl: string;
  version: [number, number, number];
}

type ReleaseVersion = [number, number, number];

const VERSION_TAG_PATTERN = /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const RELEASE_TAG_URL_PREFIX = "https://github.com/TTTTTony32/Lilt/releases/tag/";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

export function parseReleaseVersion(tagName: string): ReleaseVersion | null {
  const match = VERSION_TAG_PATTERN.exec(tagName.trim());
  if (!match) return null;
  return [Number(match[1]), Number(match[2]), Number(match[3])];
}

export function compareReleaseVersions(left: ReleaseVersion, right: ReleaseVersion): number {
  if (left[0] !== right[0]) return left[0] > right[0] ? 1 : -1;
  if (left[1] !== right[1]) return left[1] > right[1] ? 1 : -1;
  if (left[2] !== right[2]) return left[2] > right[2] ? 1 : -1;
  return 0;
}

export function selectLatestStableRelease(payload: unknown): GitHubReleaseSummary | null {
  if (!Array.isArray(payload)) return null;

  const candidates = payload.flatMap((item): GitHubReleaseSummary[] => {
    if (!isRecord(item) || item.draft === true || item.prerelease === true) return [];
    const tagName = typeof item.tag_name === "string" ? item.tag_name.trim() : "";
    const version = parseReleaseVersion(tagName);
    if (!version) return [];

    const htmlUrl = typeof item.html_url === "string" && item.html_url.startsWith(RELEASE_TAG_URL_PREFIX)
      ? item.html_url
      : `${RELEASE_TAG_URL_PREFIX}${encodeURIComponent(tagName)}`;
    return [{ tagName, htmlUrl, version }];
  });

  candidates.sort((left, right) => compareReleaseVersions(right.version, left.version));
  return candidates[0] ?? null;
}

export function isReleaseNewerThan(latest: GitHubReleaseSummary, currentVersion: string): boolean {
  const normalizedCurrentVersion = currentVersion.trim().startsWith("v")
    ? currentVersion.trim()
    : `v${currentVersion.trim()}`;
  const current = parseReleaseVersion(normalizedCurrentVersion);
  return current !== null && compareReleaseVersions(latest.version, current) > 0;
}
