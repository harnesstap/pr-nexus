/**
 * Spread on every text field (§8.2 "Text fields"): names, branch patterns, channel ids and
 * search terms are not prose, so the WebView must not underline, autocorrect, capitalize
 * or autofill them. Free-form Slack message text, if a field for it is added, may keep
 * spellcheck on.
 */
export const plainText = {
  spellCheck: false,
  autoCorrect: "off",
  autoCapitalize: "off",
  autoComplete: "off",
} as const;
