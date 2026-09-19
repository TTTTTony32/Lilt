import { describe, expect, it } from "vitest";
import { formatSelectionShortcut } from "./selection-shortcut";

function keyEvent(overrides: Partial<KeyboardEvent> = {}): Parameters<typeof formatSelectionShortcut>[0] {
  return {
    altKey: false,
    code: "",
    ctrlKey: false,
    key: "",
    metaKey: false,
    shiftKey: false,
    ...overrides,
  };
}

describe("formatSelectionShortcut", () => {
  it("normalizes modifiers and letter codes in a stable order", () => {
    expect(formatSelectionShortcut(keyEvent({ key: "l", code: "KeyL", ctrlKey: true, shiftKey: true }))).toBe("Ctrl+Shift+L");
    expect(formatSelectionShortcut(keyEvent({ key: "k", code: "KeyK", altKey: true, metaKey: true }))).toBe("Alt+Meta+K");
  });

  it("normalizes common key aliases and digits", () => {
    expect(formatSelectionShortcut(keyEvent({ key: " ", code: "Space", ctrlKey: true }))).toBe("Ctrl+Space");
    expect(formatSelectionShortcut(keyEvent({ key: "Escape", code: "Escape" }))).toBe("Escape");
    expect(formatSelectionShortcut(keyEvent({ key: "1", code: "Digit1", shiftKey: true }))).toBe("Shift+1");
  });

  it("waits for a primary key and ignores unidentified keys", () => {
    expect(formatSelectionShortcut(keyEvent({ key: "Control", code: "ControlLeft", ctrlKey: true }))).toBeNull();
    expect(formatSelectionShortcut(keyEvent({ key: "Dead", code: "" }))).toBeNull();
    expect(formatSelectionShortcut(keyEvent({ key: "Unidentified", code: "" }))).toBeNull();
  });
});
