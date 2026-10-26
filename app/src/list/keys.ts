/** Single-key list shortcuts (§8.2) ignore Cmd, Ctrl and Alt so system shortcuts such as Cmd+R keep working. Shift is allowed (Shift+x). */
export function isPlainShortcut(event: Pick<KeyboardEvent, "metaKey" | "ctrlKey" | "altKey">): boolean {
  return !event.metaKey && !event.ctrlKey && !event.altKey;
}

/** Enter opens the focused row unless it lands on a control that handles Enter itself (a button, link, or menu item). */
export function wantsRowEnter(target: Element | null): boolean {
  return !target?.closest("button, a, summary, [role='menuitem'], [role='tab']");
}

const NON_TEXT_INPUTS = new Set(["checkbox", "radio", "button", "submit", "reset"]);

/**
 * True while focus is in a field that takes typed text, so list shortcuts and
 * Esc stay out of its way. A focused row checkbox is not typing: Esc there
 * still clears the selection.
 */
export function isTypingTarget(target: HTMLElement | null): boolean {
  if (!target) {
    return false;
  }
  if (target.isContentEditable || target.tagName === "TEXTAREA" || target.tagName === "SELECT") {
    return true;
  }
  return target instanceof HTMLInputElement && !NON_TEXT_INPUTS.has(target.type);
}
