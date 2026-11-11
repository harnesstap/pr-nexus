/**
 * Sidebar view queries. The split matches `view_rule` in pr-nexus-core:
 * `bump:` never goes to GitHub, and the qualifiers named in `localPair`
 * are decided from synced rows so My PRs, To review, and Bumps due stay
 * the lists they already were.
 */
import { AUTHOR, REVIEW_REQUESTED } from "./model";

export const MINE_QUERY = "is:open author:@me";
export const REVIEW_QUERY = "is:open review-requested:@me";
export const BUMP_QUERY = "is:open author:@me bump:due";

export const VIEW_NAME_MAX = 80;
export const VIEW_QUERY_MAX = 1024;

export interface ViewQualifier {
  negated: boolean;
  key: string;
  value: string;
}

export interface AnalyzedRule {
  empty: boolean;
  fullyLocal: boolean;
  /** Qualifiers to send when the rule is not fully local. `bump:` is omitted. */
  github: string;
  text: string;
  showsFinished: boolean;
  invalid: boolean;
  qualifiers: ViewQualifier[];
}

interface Token {
  negated: boolean;
  key: string | null;
  value: string;
}

export function analyzeViewRule(raw: string): AnalyzedRule {
  const trimmed = raw.trim();
  if (trimmed === "") {
    return { empty: true, fullyLocal: true, github: "", text: "", showsFinished: false, invalid: false, qualifiers: [] };
  }
  const tokens = scan(trimmed);
  if (tokens === null) {
    return { empty: false, fullyLocal: false, github: "", text: "", showsFinished: false, invalid: true, qualifiers: [] };
  }
  const github: string[] = [];
  const text: string[] = [];
  const qualifiers: ViewQualifier[] = [];
  let fullyLocal = true;
  let showsFinished = false;
  for (const token of tokens) {
    if (token.key === null) {
      text.push(token.value);
      continue;
    }
    qualifiers.push({ negated: token.negated, key: token.key, value: token.value });
    if (token.key === "bump") {
      showsFinished = showsFinished || finished(token);
      continue;
    }
    showsFinished = showsFinished || finished(token);
    if (!localPair(token.key, token.value)) {
      fullyLocal = false;
    }
    github.push(renderQualifier(token));
  }
  return {
    empty: false,
    fullyLocal,
    github: fullyLocal ? "" : github.join(" "),
    text: text.join(" "),
    showsFinished,
    invalid: false,
    qualifiers,
  };
}

/** The bump queue screen: the default rule, nothing else. */
export function isBumpQueueQuery(raw: string): boolean {
  const rule = analyzeViewRule(raw);
  if (!rule.fullyLocal || rule.invalid || rule.text !== "") {
    return false;
  }
  const seen = rule.qualifiers.map((token) => `${token.key}:${token.value.toLowerCase()}`).sort();
  return rule.qualifiers.every((token) => !token.negated) && seen.join(" ") === "author:@me bump:due is:open";
}

export function matchesLocalRule(
  row: { involvement: number; state: string; is_draft: boolean },
  due: boolean,
  raw: string,
): boolean {
  const tokens = scan(raw);
  if (tokens === null) {
    return false;
  }
  return tokens.every((token) => matchToken(token, row.involvement, row.state, row.is_draft, due, true));
}

export function matchesBumpOnly(due: boolean, raw: string): boolean {
  const tokens = scan(raw);
  if (tokens === null) {
    return false;
  }
  return tokens.every((token) => token.key !== "bump" || matchToken(token, 0, "open", false, due, false));
}

export function unfinishedQuote(raw: string): boolean {
  return scan(raw) === null;
}

function localPair(key: string, value: string): boolean {
  const folded = value.toLowerCase();
  return (
    (key === "author" && folded === "@me") ||
    (key === "review-requested" && folded === "@me") ||
    (key === "is" && (folded === "open" || folded === "closed" || folded === "merged" || folded === "draft" || folded === "pr")) ||
    (key === "state" && (folded === "open" || folded === "closed")) ||
    (key === "draft" && (folded === "true" || folded === "false")) ||
    (key === "bump" && folded === "due")
  );
}

function finished(token: Token): boolean {
  if (token.key === null) {
    return false;
  }
  const value = token.value.toLowerCase();
  if (token.negated) {
    return (token.key === "is" || token.key === "state") && value === "open";
  }
  return (token.key === "is" && (value === "closed" || value === "merged")) || (token.key === "state" && value === "closed");
}

function matchToken(token: Token, involvement: number, state: string, isDraft: boolean, due: boolean, strict: boolean): boolean {
  if (token.key === null) {
    return true;
  }
  const value = token.value.toLowerCase();
  const open = state.toLowerCase() === "open";
  let positive: boolean;
  switch (token.key) {
    case "author":
      positive = value === "@me" ? (involvement & AUTHOR) !== 0 : !strict;
      break;
    case "review-requested":
      positive = value === "@me" ? (involvement & REVIEW_REQUESTED) !== 0 : !strict;
      break;
    case "is":
      if (value === "open" || value === "pr") {
        positive = true;
      } else if (value === "closed") {
        positive = state.toLowerCase() === "closed";
      } else if (value === "merged") {
        positive = state.toLowerCase() === "merged";
      } else if (value === "draft") {
        positive = isDraft;
      } else {
        positive = !strict;
      }
      break;
    case "state":
      if (value === "open") {
        positive = true;
      } else if (value === "closed") {
        positive = !open;
      } else {
        positive = !strict;
      }
      break;
    case "draft":
      if (value === "true") {
        positive = isDraft;
      } else if (value === "false") {
        positive = !isDraft;
      } else {
        positive = !strict;
      }
      break;
    case "bump":
      positive = value === "due" ? due : false;
      break;
    default:
      positive = !strict;
      break;
  }
  if (token.negated && (token.key === "is" || token.key === "state") && value === "open") {
    return !open;
  }
  return token.negated ? !positive : positive;
}

function renderQualifier(token: Token): string {
  const value = /\s/u.test(token.value) ? `"${token.value.replaceAll('"', '\\"')}"` : token.value;
  const body = `${token.key ?? ""}:${value}`;
  return token.negated ? `-${body}` : body;
}

/** Tokens, or null when a quote is unfinished. Keys are lowercased and quotes removed. */
function scan(input: string): Token[] | null {
  const out: Token[] = [];
  let rest = input;
  while (rest.trimStart() !== "") {
    rest = rest.trimStart();
    let negated = false;
    let body = rest;
    if (body.startsWith("-") && body.length > 1 && !isSpace(body[1] ?? "")) {
      negated = true;
      body = body.slice(1);
    }
    const taken = take(body);
    if (taken === null) {
      return null;
    }
    rest = taken.rest;
    if (taken.token !== "") {
      out.push(parseToken(negated, taken.token));
    }
  }
  return out;
}

function take(input: string): { token: string; rest: string } | null {
  let inQuote = false;
  let escape = false;
  for (let index = 0; index < input.length; index += 1) {
    const ch = input[index] ?? "";
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
    if (!inQuote && isSpace(ch)) {
      return { token: input.slice(0, index), rest: input.slice(index) };
    }
  }
  if (inQuote || escape) {
    return null;
  }
  return { token: input, rest: "" };
}

function parseToken(negated: boolean, raw: string): Token {
  const colon = colonOutsideQuotes(raw);
  if (colon === null || !isKey(raw.slice(0, colon))) {
    return { negated, key: null, value: raw };
  }
  return { negated, key: raw.slice(0, colon).toLowerCase(), value: unwrap(raw.slice(colon + 1)) };
}

function unwrap(raw: string): string {
  if (raw.length >= 2 && raw.startsWith('"') && raw.endsWith('"')) {
    return raw.slice(1, -1).replaceAll('\\"', '"').replaceAll("\\\\", "\\");
  }
  return raw;
}

function isKey(key: string): boolean {
  return /^[A-Za-z][A-Za-z0-9_-]*$/u.test(key);
}

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
