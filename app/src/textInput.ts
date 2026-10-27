/**
 * Spread on every text field (§8.2 "Text fields"): names, branch patterns, channel ids and
 * search terms are not prose, so the WebView must not underline, autocorrect, capitalize
 * or autofill them. A free-form Slack message `<textarea>`, if one is added, may keep
 * spellcheck on (mark it `data-prose`), but never autocorrect.
 */
export const plainText = {
  spellCheck: false,
  autoCorrect: "off",
  autoCapitalize: "off",
  autoComplete: "off",
} as const;

const FIELDS = "input, textarea";
const NON_TEXT = new Set(["checkbox", "radio", "color", "range", "file", "hidden", "button", "submit", "reset", "image"]);

function harden(field: HTMLInputElement | HTMLTextAreaElement): void {
  if (field instanceof HTMLInputElement && NON_TEXT.has(field.type)) {
    return;
  }
  const prose = field instanceof HTMLTextAreaElement && field.dataset.prose !== undefined;
  for (const [name, value] of [
    ["autocorrect", "off"],
    ["autocapitalize", "off"],
    ["autocomplete", "off"],
    ["spellcheck", "false"],
  ] as const) {
    if (prose && name === "spellcheck") {
      continue;
    }
    if (field.getAttribute(name) !== value) {
      field.setAttribute(name, value);
    }
  }
}

function hardenWithin(node: Node): void {
  if (node instanceof HTMLInputElement || node instanceof HTMLTextAreaElement) {
    harden(node);
  }
  if (node instanceof Element) {
    node.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>(FIELDS).forEach(harden);
  }
}

/**
 * Safety net under `plainText`: applies the same attributes to every text field in
 * `doc`, including fields created later (portals, third-party widgets) and fields whose
 * attributes are changed afterwards. Returns a function that stops watching.
 */
export function guardTextFields(doc: Document): () => void {
  hardenWithin(doc.documentElement);
  const observer = new MutationObserver((records) => {
    for (const record of records) {
      if (record.type === "attributes") {
        hardenWithin(record.target);
      } else {
        record.addedNodes.forEach(hardenWithin);
      }
    }
  });
  observer.observe(doc.documentElement, {
    subtree: true,
    childList: true,
    attributes: true,
    attributeFilter: ["autocorrect", "autocapitalize", "autocomplete", "spellcheck", "type"],
  });
  return () => observer.disconnect();
}
