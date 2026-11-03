/** One PR as the Markdown copy needs it. */
export interface MarkdownPr {
  title: string;
  url: string;
}

/** Escapes the characters that would end a Markdown link label early. */
export function escapeLinkText(text: string): string {
  return text.trim().replace(/([\\[\]])/g, "\\$1");
}

/** `- [title](url)` per PR, in the given order, with a trailing newline; empty for no PRs. */
export function prsToMarkdown(prs: readonly MarkdownPr[]): string {
  if (prs.length === 0) {
    return "";
  }
  return `${prs.map((pr) => `- [${escapeLinkText(pr.title)}](${pr.url.trim()})`).join("\n")}\n`;
}

/** Writes text to the clipboard, falling back to a hidden textarea and `execCommand("copy")`. */
export async function copyText(text: string): Promise<void> {
  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return;
    } catch {
      // Fall through to the legacy path.
    }
  }
  const area = document.createElement("textarea");
  area.value = text;
  area.setAttribute("readonly", "");
  area.style.position = "fixed";
  area.style.opacity = "0";
  document.body.appendChild(area);
  area.select();
  try {
    if (!document.execCommand?.("copy")) {
      throw new Error("Clipboard is not available");
    }
  } finally {
    area.remove();
  }
}
