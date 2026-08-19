import { describe, expect, it } from "vitest";
import {
  calculateOverallPercent,
  downloadActivityReducer,
  initialDownloadActivityState,
  listDownloadActivities,
  normalizeStagePercent,
  selectDownloadActivity,
} from "./download-activity";

describe("download activity progress", () => {
  it("maps each resource phase into the total progress range", () => {
    expect(calculateOverallPercent("dictionary", "download", 50)).toBe(35);
    expect(calculateOverallPercent("dictionary", "verify", 100)).toBe(85);
    expect(calculateOverallPercent("dictionary", "extract", 40)).toBe(91);
    expect(calculateOverallPercent("pdf-engine", "index", null)).toBe(0);
    expect(calculateOverallPercent("pdf-engine", "download", 50)).toBe(40);
    expect(calculateOverallPercent("pdf-engine", "extract", 100)).toBe(100);
  });

  it("normalizes fractions, percentages, and byte counters", () => {
    expect(normalizeStagePercent(null, null, 0.85)).toBe(85);
    expect(normalizeStagePercent(null, null, 85)).toBe(85);
    expect(normalizeStagePercent(25, 100)).toBe(25);
    expect(normalizeStagePercent(1, 0)).toBeNull();
  });
});

describe("download activity reducer", () => {
  it("keeps dictionary and PDF Engine operations visible at the same time", () => {
    let state = initialDownloadActivityState;
    state = downloadActivityReducer(state, {
      type: "started",
      resource: "dictionary",
      operationId: "dictionary-1",
      phase: "download",
    });
    state = downloadActivityReducer(state, {
      type: "started",
      resource: "pdf-engine",
      operationId: "pdf-1",
      phase: "index",
    });
    state = downloadActivityReducer(state, {
      type: "progress",
      resource: "dictionary",
      operationId: "dictionary-1",
      phase: "download",
      stagePercent: 50,
    });

    expect(listDownloadActivities(state)).toHaveLength(2);
    expect(selectDownloadActivity(state, "dictionary")?.overallPercent).toBe(35);
    expect(selectDownloadActivity(state, "pdf-engine")?.phase).toBe("index");
  });

  it("keeps stage progress separate from the total progress used by the fill bar", () => {
    const state = downloadActivityReducer(
      downloadActivityReducer(initialDownloadActivityState, {
        type: "started",
        resource: "dictionary",
        operationId: "dictionary-1",
        phase: "extract",
      }),
      {
        type: "progress",
        resource: "dictionary",
        operationId: "dictionary-1",
        phase: "extract",
        stagePercent: 40,
      },
    );
    const activity = selectDownloadActivity(state, "dictionary");

    expect(activity?.stagePercent).toBe(40);
    expect(activity?.overallPercent).toBe(91);
  });

  it("ignores stale events after the current operation changes", () => {
    let state = downloadActivityReducer(initialDownloadActivityState, {
      type: "started",
      resource: "dictionary",
      operationId: "dictionary-1",
      phase: "download",
    });
    state = downloadActivityReducer(state, {
      type: "completed",
      resource: "dictionary",
      operationId: "dictionary-1",
    });
    state = downloadActivityReducer(state, {
      type: "started",
      resource: "dictionary",
      operationId: "dictionary-2",
      phase: "download",
    });
    const beforeStaleEvent = state;
    state = downloadActivityReducer(state, {
      type: "progress",
      resource: "dictionary",
      operationId: "dictionary-1",
      phase: "extract",
      stagePercent: 100,
    });

    expect(state).toBe(beforeStaleEvent);
    expect(selectDownloadActivity(state, "dictionary")?.operationId).toBe("dictionary-2");
  });

  it("does not allow two active operations for one resource to overwrite each other", () => {
    let state = downloadActivityReducer(initialDownloadActivityState, {
      type: "started",
      resource: "pdf-engine",
      operationId: "pdf-1",
      phase: "download",
    });
    state = downloadActivityReducer(state, {
      type: "started",
      resource: "pdf-engine",
      operationId: "pdf-2",
      phase: "download",
    });

    expect(listDownloadActivities(state)).toHaveLength(1);
    expect(selectDownloadActivity(state, "pdf-engine")?.operationId).toBe("pdf-1");
  });
});
