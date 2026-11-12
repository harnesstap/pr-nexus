/**
 * Split the list search field into a GitHub query and local free text.
 *
 * Token rules match `partition` in `pr-nexus-core`'s `github_query`: words
 * separated by whitespace, quotes kept together, a leading `-` negates.
 * A token is a qualifier when it is `key:value` and `key` is the same shape
 * `is_key` accepts. `prepare` still resolves dates, `@me`, `sort:`, and
 * `is:pr`. This module only decides which tokens take that path.
 */

/** Cap for a stored search, same as the old GitHub query field. */
export const SEARCH_FIELD_MAX = 1024;

export interface SplitSearch {
  /** Qualifier tokens, in order, ready for the GitHub query pipeline. */
  github: string;
  /** Remaining words, for the local substring search. */
  text: string;
}

interface Token {
  start: number;
  end: number;
  text: string;
}

/** Qualifiers offered while typing. Recognition is wider than this list: any `key:value` goes to GitHub. */
const QUALIFIERS: readonly { key: string; hint: string; values?: readonly string[] }[] = [
  { key: "is", hint: "pr, open, closed, merged, draft", values: ["pr", "open", "closed", "merged", "draft"] },
  { key: "state", hint: "open or closed", values: ["open", "closed"] },
  { key: "archived", hint: "true or false", values: ["true", "false"] },
  { key: "author", hint: "login or @me", values: ["@me"] },
  { key: "review-requested", hint: "login or @me", values: ["@me"] },
  { key: "assignee", hint: "login or @me", values: ["@me"] },
  { key: "involves", hint: "login or @me", values: ["@me"] },
  { key: "reviewed-by", hint: "login or @me", values: ["@me"] },
  { key: "user", hint: "login or @me", values: ["@me"] },
  { key: "repo", hint: "owner/name" },
  { key: "org", hint: "organization" },
  { key: "label", hint: "label name" },
  { key: "draft", hint: "true or false", values: ["true", "false"] },
  { key: "created", hint: "for example >@today-1w", values: [">@today-1w", ">=@today-1d", "@today"] },
  { key: "updated", hint: "for example >=@today-1w", values: [">=@today-1w", ">@today-1d", "@today"] },
  { key: "closed", hint: "a date, or >@today-1d" },
  { key: "merged", hint: "a date, or >@today-1d" },
  { key: "head", hint: "branch" },
  { key: "base", hint: "branch" },
  { key: "status", hint: "success, failure, or pending", values: ["success", "failure", "pending"] },
  { key: "sort", hint: "updated-desc, created-asc, …", values: ["updated-desc", "updated-asc", "created-desc", "created-asc", "comments-desc"] },
  { key: "comments", hint: "a number" },
  { key: "milestone", hint: "milestone" },
  { key: "no", hint: "label, assignee, or milestone", values: ["label", "assignee", "milestone"] },
  { key: "bump", hint: "due — kept on this machine, not sent to GitHub", values: ["due"] },
];

export interface Suggestion {
  /** Text that replaces the token being typed, including a trailing space when the value is complete. */
  insert: string;
  label: string;
  hint: string;
}

/** The search box contents: the combined field, or the saved GitHub query from an older view. */
export function searchFieldValue(filters: { search: string; github: string }): string {
  return filters.search !== "" ? filters.search : filters.github;
}

/** Qualifier tokens from the search field. Empty means the usual list, with no GitHub search. */
export function githubQueryOf(filters: { search: string; github: string }): string {
  return splitSearch(searchFieldValue(filters)).github.trim();
}

/** A non-negated `sort:` qualifier. Other qualifiers leave the menu's Sort in charge. */
export function sortLocksOrder(raw: string): boolean {
  for (const token of scanTokens(raw)) {
    if (!isQualifier(token.text) || token.text.startsWith("-")) {
      continue;
    }
    const colon = colonOutsideQuotes(token.text);
    if (colon !== null && token.text.slice(0, colon) === "sort") {
      return true;
    }
  }
  return false;
}

export function splitSearch(raw: string): SplitSearch {
  const tokens = scanTokens(raw);
  const github: string[] = [];
  const text: string[] = [];
  for (const token of tokens) {
    if (isQualifier(token.text)) {
      github.push(token.text);
    } else {
      text.push(token.text);
    }
  }
  return { github: github.join(" "), text: text.join(" ") };
}

/** Completions for the token under the caret. Empty when the token is not the start of a qualifier. */
export function suggestionsFor(token: string): Suggestion[] {
  const negated = token.startsWith("-") && token.length > 1 && !/\s/u.test(token.slice(1, 2));
  const body = negated ? token.slice(1) : token;
  const sign = negated ? "-" : "";
  const colon = colonOutsideQuotes(body);
  if (colon === null) {
    const needle = body.toLowerCase();
    if (needle.length === 0) {
      return [];
    }
    return QUALIFIERS.filter((item) => item.key.startsWith(needle))
      .slice(0, 6)
      .map((item) => ({ insert: `${sign}${item.key}:`, label: `${sign}${item.key}:`, hint: item.hint }));
  }
  const key = body.slice(0, colon).toLowerCase();
  const typed = body.slice(colon + 1);
  const item = QUALIFIERS.find((entry) => entry.key === key);
  if (!item?.values) {
    return [];
  }
  return item.values
    .filter((value) => value.toLowerCase().startsWith(typed.toLowerCase()) && value.toLowerCase() !== typed.toLowerCase())
    .slice(0, 6)
    .map((value) => ({
      insert: `${sign}${item.key}:${value} `,
      label: `${sign}${item.key}:${value}`,
      hint: "",
    }));
}

/** The token containing `caret`, or null when the caret sits on whitespace. */
export function tokenAt(value: string, caret: number): Token | null {
  const at = Math.max(0, Math.min(caret, value.length));
  return scanTokens(value).find((token) => at > token.start && at <= token.end) ?? null;
}

export function replaceToken(value: string, token: Token, insertion: string): { value: string; caret: number } {
  return {
    value: value.slice(0, token.start) + insertion + value.slice(token.end),
    caret: token.start + insertion.length,
  };
}

function scanTokens(input: string): Token[] {
  const out: Token[] = [];
  let index = 0;
  while (index < input.length) {
    while (index < input.length && isSpace(input[index] ?? "")) {
      index += 1;
    }
    if (index >= input.length) {
      break;
    }
    const start = index;
    let inQuote = false;
    let escape = false;
    while (index < input.length) {
      const ch = input[index] ?? "";
      if (escape) {
        escape = false;
        index += 1;
        continue;
      }
      if (ch === "\\" && inQuote) {
        escape = true;
        index += 1;
        continue;
      }
      if (ch === '"') {
        inQuote = !inQuote;
        index += 1;
        continue;
      }
      if (!inQuote && isSpace(ch)) {
        break;
      }
      index += 1;
    }
    // An unfinished quote still counts as one token, so a half-typed `label:"bug`
    // stays a qualifier and GitHub can report it.
    const text = input.slice(start, index);
    if (text !== "") {
      out.push({ start, end: index, text });
    }
  }
  return out;
}

function isQualifier(token: string): boolean {
  const body = qualifierBody(token);
  const colon = colonOutsideQuotes(body);
  if (colon === null || colon === 0) {
    return false;
  }
  if (!isKey(body.slice(0, colon))) {
    return false;
  }
  // `https://…` is a pasted link, not a qualifier.
  return !body.slice(colon + 1).startsWith("//");
}

function qualifierBody(token: string): string {
  if (token.startsWith("-") && token.length > 1 && !isSpace(token[1] ?? "")) {
    return token.slice(1);
  }
  return token;
}

function isKey(key: string): boolean {
  if (!/^[A-Za-z]/u.test(key)) {
    return false;
  }
  for (const ch of key.slice(1)) {
    if (!/[A-Za-z0-9_-]/u.test(ch)) {
      return false;
    }
  }
  return true;
}

/** Index of the first colon that is not inside quotes, or null. */
function colonOutsideQuotes(raw: string): number | null {
  let inQuote = false;
  let escape = false;
  for (let index = 0; index < raw.length; index += 1) {
    const ch = raw[index];
    if (escape) {
      escape = false;
      continue;
    }
    if (ch === "\\" && inQuote) {
      escape = true;
      continue;
    }
    if (ch === '"') {
      inQuote = !inQuote;
      continue;
    }
    if (ch === ":" && !inQuote) {
      return index;
    }
  }
  return null;
}

function isSpace(ch: string): boolean {
  return ch === " " || ch === "\t" || ch === "\n" || ch === "\r" || ch === "\f";
}
