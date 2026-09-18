import { describe, expect, it } from "vitest";
import { appendPdfJobLogMessage, reducePdfJobLog } from "./pdf-job-log";
import type { DocumentContext, PdfJobEvent, PdfJobLogEntry, PdfPreflightState } from "../types/contracts";

const preflight: PdfPreflightState = {
  requestId: "preflight-1",
  responsePhase: "waiting",
  status: "running",
  schemaVersion: null,
  context: null,
  contextHash: null,
  warnings: [],
  applied: false,
  message: null,
};

describe("PDF task log projection", () => {
  it("keeps stage, preflight, quality, warning, and terminal events in order", () => {
    const events: PdfJobEvent[] = [
      { type: "started", taskId: "task-1", workerVersion: "worker-1" },
      { type: "stage", taskId: "task-1", stage: "preparing" },
      {
        type: "preflightStarted",
        taskId: "task-1",
        preflightRequestId: "preflight-1",
        preflight,
      },
      {
        type: "preflightDegraded",
        taskId: "task-1",
        preflightRequestId: "preflight-1",
        preflight: { ...preflight, status: "degraded", message: "未收到完整预检结果" },
      },
      {
        type: "warning",
        taskId: "task-1",
        code: "document_warning",
        message: "部分页面没有文本层",
      },
      {
        type: "diagnostic",
        taskId: "task-1",
        diagnostic: {
          severity: "warning",
          ruleId: "placeholder_missing",
          message: "占位符数量不一致",
          taskId: "task-1",
          translationRequestId: null,
          segmentId: "p1-s1",
          pageNumber: 1,
        },
      },
      {
        type: "finished",
        taskId: "task-1",
        outputPdf: "C:/output.pdf",
        outputMode: "bilingual",
        pageCount: 2,
        warnings: [],
      },
    ];

    const logs = events.reduce(reducePdfJobLog, []);
    expect(logs.map((entry) => entry.kind)).toEqual([
      "system",
      "stage",
      "preflight",
      "preflight",
      "warning",
      "quality",
      "result",
    ]);
    expect(logs[0]?.message).toContain("状态：翻译进行中");
    expect(logs.find((entry) => entry.kind === "quality")?.message).toBe("警告 · placeholder_missing · 占位符数量不一致 · 第 1 页 · 段落 p1-s1");
    expect(logs.at(-1)?.message).toBe("翻译完成 · 2 页");
  });

  it("records the preflight result and document context summary in the task log", () => {
    const context: DocumentContext = {
      schemaVersion: 2,
      title: "A PDF title",
      abstract: "A short abstract",
      documentType: "research paper",
      domain: "linguistics",
      headings: ["Introduction", "Results"],
      keyTerms: [{ source: "source term", target: "目标术语", sourceKind: null, confidence: 0.9, note: null }],
      abbreviations: [{ abbreviation: "PDF", expanded: "Portable Document Format", target: null, confidence: null }],
      translationNotes: ["保留公式格式"],
      contextHash: "context-hash",
    };
    const logs = reducePdfJobLog([], {
      type: "preflightCompleted",
      taskId: "task-1",
      preflightRequestId: "preflight-1",
      preflight: {
        ...preflight,
        status: "completed",
        applied: true,
        responsePhase: null,
        schemaVersion: context.schemaVersion,
        context,
        contextHash: context.contextHash,
      },
    });

    expect(logs.map((entry) => entry.message)).toEqual([
      "文档预检已完成 · 上下文已应用",
      "文档上下文：A PDF title · 类型：research paper · 领域：linguistics · 术语 1 · 缩写 1 · 标题层级 2",
      "摘要：A short abstract",
      "翻译注意事项：保留公式格式",
      "任务术语：source term → 目标术语",
      "任务缩写：PDF（Portable Document Format）",
      "上下文 v2 · context-hash",
    ]);
  });

  it("merges continuous progress and deduplicates adjacent entries", () => {
    const first: PdfJobEvent = {
      type: "progress",
      taskId: "task-1",
      progress: { stage: "translating", current: 1, total: 10, fraction: 0.1, message: null },
    };
    const second: PdfJobEvent = {
      type: "progress",
      taskId: "task-1",
      progress: { stage: "translating", current: 2, total: 10, fraction: 0.2, message: null },
    };
    const logs = reducePdfJobLog(reducePdfJobLog([], first), second);
    expect(logs).toHaveLength(1);
    expect(logs[0]?.message).toBe("分段翻译 · 2/10");
    expect(appendPdfJobLogMessage(logs, "progress", "info", "分段翻译 · 2/10")).toBe(logs);
  });

  it("merges continuous progress when the worker omits counters", () => {
    const first: PdfJobEvent = {
      type: "progress",
      taskId: "task-1",
      progress: { stage: "assembling", current: null, total: null, fraction: null, message: null },
    };
    const second: PdfJobEvent = {
      type: "progress",
      taskId: "task-1",
      progress: { stage: "assembling", current: null, total: null, fraction: 0.5, message: "处理中" },
    };
    const logs = reducePdfJobLog(reducePdfJobLog([], first), second);
    expect(logs).toHaveLength(1);
    expect(logs[0]?.message).toBe("组装 PDF · 处理中");
  });

  it("caps long-running logs and records cancellation or failure", () => {
    let logs: PdfJobLogEntry[] = [];
    for (let index = 0; index < 340; index += 1) {
      logs = appendPdfJobLogMessage(logs, "system", "info", `event-${index}`);
    }
    expect(logs).toHaveLength(300);
    expect(logs[0]?.message).toBe("event-40");

    const cancelled = reducePdfJobLog(logs, { type: "cancelled", taskId: "task-1", reason: "用户停止" });
    const failed = reducePdfJobLog(cancelled, { type: "failed", taskId: "task-1", code: "engine_error", message: "Engine 失败" });
    expect(failed.at(-2)?.message).toBe("任务已取消 · 用户停止");
    expect(failed.at(-1)?.level).toBe("error");
    expect(failed.at(-1)?.message).toBe("翻译失败 · Engine 失败 · 错误代码：engine_error");
  });
});
