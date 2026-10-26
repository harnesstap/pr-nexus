/**
 * Release builds have no native context menu (§8.2 Desktop shell): the WebView's default menu
 * (Reload, Inspect Element) is meaningless in a desktop app. Dev builds keep it.
 */
export function disableNativeContextMenu(target: Pick<Document, "addEventListener" | "removeEventListener">, production: boolean): () => void {
  if (!production) {
    return () => undefined;
  }
  const block = (event: Event) => event.preventDefault();
  target.addEventListener("contextmenu", block);
  return () => target.removeEventListener("contextmenu", block);
}
