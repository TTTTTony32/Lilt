export function isDictionarySelection(text: string): boolean {
  return /^[A-Za-z]+(?:['-][A-Za-z]+)*$/.test(text.trim());
}

export type SelectionRoute = "dictionary" | "paragraph";

export function routeSelection(text: string): SelectionRoute {
  return isDictionarySelection(text) ? "dictionary" : "paragraph";
}
