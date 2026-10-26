/**
 * Slack mrkdwn → plain text for the Slack panel: `<url|label>` → label,
 * `<url>` → url, `<#C…|name>` → #name, `<@U…>` → @U…, entities decoded.
 * Rendered as text, never as HTML.
 */
export function slackText(text: string): string {
  return text
    .replace(/<#[A-Z0-9]+\|([^>]+)>/g, "#$1")
    .replace(/<@([A-Z0-9]+)(\|[^>]*)?>/g, (_, id: string, label?: string) => `@${label ? label.slice(1) : id}`)
    .replace(/<!(here|channel|everyone)>/g, "@$1")
    .replace(/<((?:https?|mailto):[^|>]+)\|([^>]+)>/g, "$2")
    .replace(/<((?:https?|mailto):[^>]+)>/g, "$1")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&");
}

/** `5 min ago`, `3 h ago`, else a short local date. */
export function shortTime(at: string | null, now: number = Date.now()): string {
  if (!at) {
    return "";
  }
  const then = Date.parse(at);
  if (Number.isNaN(then)) {
    return "";
  }
  const minutes = Math.round((now - then) / 60000);
  if (minutes < 1) {
    return "just now";
  }
  if (minutes < 60) {
    return `${minutes} min ago`;
  }
  if (minutes < 24 * 60) {
    return `${Math.round(minutes / 60)} h ago`;
  }
  return new Date(then).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}
