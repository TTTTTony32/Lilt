import type {
  PdfJobEvent,
  PdfJobUiState,
  PdfPreflightEvent,
  PdfPreflightState,
  PdfQualityDiagnostic,
} from "../types/contracts";

export function createEmptyPdfPreflightState(): PdfPreflightState {
  return {
    requestId: null,
    responsePhase: null,
    status: "idle",
    schemaVersion: null,
    context: null,
    contextHash: null,
    warnings: [],
    applied: false,
    message: null,
  };
}

function isTerminalJobStatus(status: PdfJobUiState["status"]): boolean {
  return status === "completed" || status === "cancelled" || status === "failed";
}

function isTerminalPreflightStatus(status: PdfPreflightState["status"]): boolean {
  return status === "completed" || status === "degraded" || status === "failed";
}

function appendUniqueStrings(current: string[], additions: string[]): string[] {
  return additions.reduce((result, item) => result.includes(item) ? result : [...result, item], current);
}

function diagnosticKey(diagnostic: PdfQualityDiagnostic): string {
  return [
    diagnostic.ruleId ?? "",
    diagnostic.message,
    diagnostic.segmentId ?? "",
    diagnostic.pageNumber ?? "",
    diagnostic.translationRequestId ?? "",
  ].join("|");
}

function appendUniqueDiagnostics(current: PdfQualityDiagnostic[], additions: PdfQualityDiagnostic[]): PdfQualityDiagnostic[] {
  const keys = new Set(current.map(diagnosticKey));
  return additions.reduce((result, diagnostic) => {
    const key = diagnosticKey(diagnostic);
    if (keys.has(key)) return result;
    keys.add(key);
    return [...result, diagnostic];
  }, current);
}

function eventRequestId(event: PdfPreflightEvent): string | null {
  return event.preflightRequestId ?? event.preflight.requestId;
}

function canAcceptPreflightState(
  current: PdfJobUiState,
  incoming: PdfPreflightState,
  incomingRequestId: string | null,
): boolean {
  if (isTerminalJobStatus(current.status)) return false;

  const currentPreflight = current.preflight ?? createEmptyPdfPreflightState();
  if (isTerminalPreflightStatus(currentPreflight.status)) return false;

  const currentRequestId = currentPreflight.requestId;
  if (currentRequestId !== null && (incomingRequestId === null || incomingRequestId !== currentRequestId)) {
    return false;
  }
  return true;
}

function applyPreflightState(
  current: PdfJobUiState,
  incoming: PdfPreflightState,
  incomingRequestId: string | null,
  diagnostics: PdfQualityDiagnostic[] = [],
  warnings: string[] = [],
): PdfJobUiState {
  if (!canAcceptPreflightState(current, incoming, incomingRequestId)) return current;

  const nextPreflight = incoming.requestId === incomingRequestId
    ? incoming
    : { ...incoming, requestId: incomingRequestId };
  const nextWarnings = appendUniqueStrings(current.warnings, [...nextPreflight.warnings, ...warnings]);
  const nextDiagnostics = appendUniqueDiagnostics(current.diagnostics ?? [], diagnostics);
  return {
    ...current,
    preflight: nextPreflight,
    documentContext: nextPreflight.context ?? current.documentContext ?? null,
    warnings: nextWarnings,
    diagnostics: nextDiagnostics,
  };
}

export function mergePdfPreflightState(current: PdfJobUiState, incoming: PdfPreflightState): PdfJobUiState {
  return applyPreflightState(current, incoming, incoming.requestId);
}

export function reducePdfPreflightEvent(current: PdfJobUiState, event: PdfPreflightEvent): PdfJobUiState {
  const requestId = eventRequestId(event);
  const message = event.preflight.message && event.preflight.status !== "running"
    ? [event.preflight.message]
    : [];
  const next = applyPreflightState(current, event.preflight, requestId, event.diagnostics ?? [], message);
  return next === current ? current : { ...next, taskId: event.taskId };
}

export function reducePdfPreflightWarning(current: PdfJobUiState, event: Extract<PdfJobEvent, { type: "warning" }>): PdfJobUiState {
  if (!event.code.toLowerCase().includes("preflight")) return current;

  const currentPreflight = current.preflight ?? createEmptyPdfPreflightState();
  const requestId = event.preflightRequestId ?? event.preflight?.requestId ?? null;
  const failed = event.code.toLowerCase().includes("failed") || event.code.toLowerCase().includes("error");
  const nextPreflight: PdfPreflightState = {
    ...currentPreflight,
    status: failed ? "failed" : "degraded",
    responsePhase: null,
    requestId: requestId ?? currentPreflight.requestId,
    message: event.message,
    applied: currentPreflight.applied && !failed,
    warnings: appendUniqueStrings(currentPreflight.warnings, [event.message]),
  };
  const next = event.preflight
    ? mergePdfPreflightState(current, event.preflight)
    : current;
  return applyPreflightState(
    next,
    nextPreflight,
    requestId,
    event.diagnostic ? [event.diagnostic] : [],
    [event.message],
  );
}

export function markPdfPreflightRunning(current: PdfJobUiState): PdfJobUiState {
  if (isTerminalJobStatus(current.status)) return current;
  const preflight = current.preflight ?? createEmptyPdfPreflightState();
  if (isTerminalPreflightStatus(preflight.status)) return current;
  if (preflight.status === "running" && preflight.responsePhase !== null) return current;
  return {
    ...current,
    preflight: {
      ...preflight,
      status: "running",
      responsePhase: preflight.responsePhase ?? "waiting",
    },
  };
}
