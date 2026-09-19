export type ShortcutKeyboardEvent = Pick<KeyboardEvent, "altKey" | "code" | "ctrlKey" | "key" | "metaKey" | "shiftKey">;

const SHORTCUT_MODIFIER_KEYS = new Set(["Alt", "AltGraph", "Control", "Fn", "FnLock", "Hyper", "Meta", "OS", "Shift"]);
const SHORTCUT_KEY_ALIASES: Record<string, string> = {
  " ": "Space",
  Spacebar: "Space",
  Esc: "Escape",
  Return: "Enter",
  Del: "Delete",
  Ins: "Insert",
};

export function formatSelectionShortcut(event: ShortcutKeyboardEvent): string | null {
  if (SHORTCUT_MODIFIER_KEYS.has(event.key)) return null;

  const codeKey = /^Key([A-Z])$/.exec(event.code)?.[1]
    ?? /^Digit([0-9])$/.exec(event.code)?.[1];
  const key = codeKey
    ?? SHORTCUT_KEY_ALIASES[event.key]
    ?? (event.key.length === 1 ? event.key.toUpperCase() : event.key);
  if (!key || key === "Unidentified" || key === "Dead") return null;

  const modifiers: string[] = [];
  if (event.ctrlKey) modifiers.push("Ctrl");
  if (event.altKey) modifiers.push("Alt");
  if (event.shiftKey) modifiers.push("Shift");
  if (event.metaKey) modifiers.push("Meta");
  return [...modifiers, key].join("+");
}
