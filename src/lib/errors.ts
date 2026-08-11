export function describeError(reason: unknown, fallback: string): string {
  if (reason instanceof Error && reason.message.trim()) return reason.message;
  if (typeof reason === "string" && reason.trim()) return reason;
  if (typeof reason === "object" && reason !== null && "message" in reason) {
    const message = reason.message;
    if (typeof message === "string" && message.trim()) return message;
  }
  return fallback;
}
