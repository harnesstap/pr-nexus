/** Single-key list shortcuts (§8.2) ignore Cmd, Ctrl and Alt so system shortcuts such as Cmd+R keep working. Shift is allowed (Shift+x). */
export function isPlainShortcut(event: Pick<KeyboardEvent, "metaKey" | "ctrlKey" | "altKey">): boolean {
  return !event.metaKey && !event.ctrlKey && !event.altKey;
}

/** Enter opens the focused row unless it lands on a control that handles Enter itself (a button, link, or menu item). */
export function wantsRowEnter(target: Element | null): boolean {
  return !target?.closest("button, a, summary, [role='menuitem'], [role='tab']");
}
