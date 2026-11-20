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
  /** Remaining words, for the local substring search. Pull request links are left out. */
  text: string;
  /** Pull request URLs and `owner/repo#n` tokens, in order. */
  links: PrLink[];
}

/** A GitHub pull request named by URL or `owner/repo#n`. `searchText.ts` matches `pr_link::parse`. */
export interface PrLink {
  owner: string;
  name: string;
  number: number;
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
  { key: "team-review-requested", hint: "org/team" },
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

/**
 * Qualifiers that replace view membership. Menu-owned tokens (`org:`,
 * `is:draft`, `review:`, `status:`, `team-review-requested:`, `assignee:`,
 * `no:assignee`) are applied as filters instead, so they do not replace
 * Mine or To review.
 */
export function githubQueryOf(filters: { search: string; github: string }): string {
  return membershipQuery(searchFieldValue(filters)).trim();
}

/** Qualifier tokens from the field, minus the ones the filter menu owns. */
export function membershipQuery(raw: string): string {
  return scanTokens(raw)
    .filter((token) => isQualifier(token.text) && !isPopoverQualifier(token.text))
    .map((token) => token.text)
    .join(" ");
}

export interface ScannedQualifier {
  start: number;
  end: number;
  text: string;
  negated: boolean;
  key: string;
  value: string;
}

/** Qualifier tokens with their place in the field. Free text is left out. */
export function scannedQualifiers(raw: string): ScannedQualifier[] {
  const out: ScannedQualifier[] = [];
  for (const token of scanTokens(raw)) {
    if (!isQualifier(token.text)) {
      continue;
    }
    const negated = token.text.startsWith("-") && token.text.length > 1;
    const body = negated ? token.text.slice(1) : token.text;
    const colon = colonOutsideQuotes(body);
    if (colon === null) {
      continue;
    }
    out.push({
      start: token.start,
      end: token.end,
      text: token.text,
      negated,
      key: body.slice(0, colon).toLowerCase(),
      value: body.slice(colon + 1),
    });
  }
  return out;
}

/** A positive menu qualifier. Negated tokens stay in the membership query. */
export function isPopoverQualifier(text: string): boolean {
  const parsed = scannedQualifiers(text)[0];
  if (!parsed || parsed.negated || parsed.text !== text || parsed.value === "") {
    return false;
  }
  const value = parsed.value.toLowerCase();
  switch (parsed.key) {
    case "org":
    case "team-review-requested":
    case "assignee":
      return true;
    case "is":
      return value === "draft";
    case "review":
      return value === "approved" || value === "changes_requested" || value === "required" || value === "none";
    case "status":
      return value === "success" || value === "failure" || value === "pending";
    case "no":
      return value === "assignee";
    default:
      return false;
  }
}

/** A non-negated `sort:` qualifier. Other qualifiers leave the menu's Sort in charge. */
export function sortLocksOrder(raw: string): boolean {
  for (const token of scanTokens(raw)) {
    if (!isQualifier(token.text) || token.text.startsWith("-")) {
      continue;
    }
    const colon = colonOutsideQuotes(token.text);
    if (colon !== null && token.text.slice(0, colon).toLowerCase() === "sort") {
      return true;
    }
  }
  return false;
}

export function splitSearch(raw: string): SplitSearch {
  const tokens = scanTokens(raw);
  const github: string[] = [];
  const text: string[] = [];
  const links: PrLink[] = [];
  for (const token of tokens) {
    if (isQualifier(token.text)) {
      github.push(token.text);
    } else {
      const link = parsePrLink(token.text);
      if (link) {
        links.push(link);
      } else {
        text.push(token.text);
      }
    }
  }
  return { github: github.join(" "), text: text.join(" "), links };
}

/**
 * The field is only a pull request link: no other words and no qualifiers.
 * That match ignores the view's base rule.
 */
export function exactPrLink(raw: string): PrLink | null {
  const parts = splitSearch(raw);
  if (parts.links.length === 1 && parts.text.trim() === "" && parts.github.trim() === "") {
    return parts.links[0] ?? null;
  }
  return null;
}

/**
 * `https://github.com/<owner>/<repo>/pull/<n>` (scheme and `www.` optional,
 * trailing path, query, or fragment allowed) or `owner/repo#n`.
 */
export function parsePrLink(raw: string): PrLink | null {
  const text = raw.trim();
  if (text === "" || /\s/u.test(text)) {
    return null;
  }
  if (looksLikeUrl(text)) {
    return parseUrl(text);
  }
  return parseShorthand(text);
}

/** Case-insensitive owner and repository, exact number. `repo` is `owner/name` or a bare name. */
export function linkMatches(row: { owner: string; repo: string; number: number }, link: PrLink): boolean {
  if (row.number !== link.number) {
    return false;
  }
  const slash = row.repo.lastIndexOf("/");
  const repoOwner = slash >= 0 ? row.repo.slice(0, slash) : "";
  const repoName = slash >= 0 ? row.repo.slice(slash + 1) : row.repo;
  const ownerOk = sameLogin(row.owner, link.owner) || sameLogin(repoOwner, link.owner);
  return ownerOk && sameLogin(repoName, link.name);
}

function looksLikeUrl(text: string): boolean {
  const host = stripScheme(text).split(/[/?#]/u)[0] ?? "";
  const bare = stripPrefixCi(host, "www.") ?? host;
  return bare.toLowerCase() === "github.com";
}

function parseShorthand(text: string): PrLink | null {
  const hash = text.indexOf("#");
  if (hash <= 0 || text.indexOf("#", hash + 1) !== -1) {
    return null;
  }
  const repo = text.slice(0, hash);
  const slash = repo.indexOf("/");
  if (slash <= 0 || repo.indexOf("/", slash + 1) !== -1) {
    return null;
  }
  const owner = repo.slice(0, slash);
  const name = repo.slice(slash + 1);
  const number = parseNumber(text.slice(hash + 1));
  if (number === null || !isLogin(owner) || !isLogin(name)) {
    return null;
  }
  return { owner, name, number };
}

function parseUrl(text: string): PrLink | null {
  let rest = stripScheme(text);
  rest = stripPrefixCi(rest, "www.") ?? rest;
  const path = stripPrefixCi(rest, "github.com");
  if (path === null || !path.startsWith("/")) {
    return null;
  }
  const parts = path.split(/[/?#]/u).filter((part) => part !== "");
  const owner = parts[0];
  const name = parts[1];
  if (!owner || !name || parts[2]?.toLowerCase() !== "pull") {
    return null;
  }
  const number = parseNumber(parts[3] ?? "");
  if (number === null || !isLogin(owner) || !isLogin(name)) {
    return null;
  }
  return { owner, name, number };
}

function stripScheme(text: string): string {
  return stripPrefixCi(text, "https://") ?? stripPrefixCi(text, "http://") ?? text;
}

function stripPrefixCi(text: string, prefix: string): string | null {
  return text.slice(0, prefix.length).toLowerCase() === prefix.toLowerCase() ? text.slice(prefix.length) : null;
}

function parseNumber(value: string): number | null {
  if (!/^[0-9]+$/u.test(value)) {
    return null;
  }
  const number = Number(value);
  return Number.isSafeInteger(number) && number > 0 ? number : null;
}

function isLogin(value: string): boolean {
  return /^[A-Za-z0-9][A-Za-z0-9._-]{0,99}$/u.test(value);
}

function sameLogin(left: string, right: string): boolean {
  return left.toLowerCase() === right.toLowerCase();
}

/** Completions for the token under the caret. Empty when the token is not the start of a qualifier. */
export function suggestionsFor(token: string, extra?: { teams?: readonly string[]; assignees?: readonly string[] }): Suggestion[] {
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
  const values = valueChoices(key, item?.values ?? [], extra);
  if (values.length === 0) {
    return [];
  }
  return values
    .filter((value) => value.toLowerCase().startsWith(typed.toLowerCase()) && value.toLowerCase() !== typed.toLowerCase())
    .slice(0, 6)
    .map((value) => ({
      insert: `${sign}${key}:${value} `,
      label: `${sign}${key}:${value}`,
      hint: "",
    }));
}

function valueChoices(key: string, values: readonly string[], extra: { teams?: readonly string[]; assignees?: readonly string[] } | undefined): string[] {
  const more = key === "team-review-requested" ? (extra?.teams ?? []) : key === "assignee" ? (extra?.assignees ?? []) : [];
  const out = [...values];
  for (const value of more) {
    const trimmed = value.trim();
    if (trimmed !== "" && !out.some((item) => item.toLowerCase() === trimmed.toLowerCase())) {
      out.push(trimmed);
    }
  }
  return out;
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
