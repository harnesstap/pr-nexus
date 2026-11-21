/**
 * Two-way sync between the filter menu and the search field.
 *
 * A menu choice that GitHub can spell is written into the field
 * (`is:draft`, `review:approved`, `org:acme`, `team-review-requested:org/team`,
 * `assignee:@me`, `no:assignee`). Editing that text updates the menu.
 * Qualifiers with no menu equivalent stay as typed. Menu-only choices
 * (Slack, starred, archived, readiness, CI None, No team) are left alone.
 *
 * Tokens already present in the view's own rule are not written again.
 * New tokens are appended. Removed tokens drop in place.
 */
import { scannedQualifiers, type ScannedQualifier } from "./searchText";

export interface ExpressibleFilters {
  orgs: string[];
  statuses: readonly string[];
  reviews: readonly string[];
  ci: readonly string[];
  teams: string[];
  assignees: string[];
  unassigned: boolean;
  search: string;
}

const REVIEW_TOKENS: Readonly<Record<string, string>> = {
  approved: "approved",
  changes_requested: "changes_requested",
  required: "review_required",
  none: "none",
};

const CI_TOKENS: Readonly<Record<string, string>> = {
  success: "passing",
  failure: "failing",
  pending: "pending",
};

function norm(token: string): string {
  return token.toLowerCase();
}

function has(tokens: readonly string[], token: string): boolean {
  const key = norm(token);
  return tokens.some((item) => norm(item) === key);
}

function setMember<T extends string>(values: readonly T[], value: T, on: boolean): T[] {
  const key = value.toLowerCase();
  const without = values.filter((item) => item.toLowerCase() !== key);
  return on ? [...without, value] : without;
}

/** Canonical menu tokens, in a stable order. */
export function tokensFor(filters: ExpressibleFilters): string[] {
  const out: string[] = [];
  const push = (token: string) => {
    if (!has(out, token)) {
      out.push(token);
    }
  };
  for (const org of filters.orgs) {
    if (org.trim() !== "") {
      push(`org:${org.trim()}`);
    }
  }
  if (filters.statuses.some((status) => status === "draft")) {
    push("is:draft");
  }
  for (const review of filters.reviews) {
    const token = reviewToken(review);
    if (token) {
      push(token);
    }
  }
  for (const ci of filters.ci) {
    const token = ciToken(ci);
    if (token) {
      push(token);
    }
  }
  for (const team of filters.teams) {
    if (team.trim() !== "") {
      push(`team-review-requested:${team.trim()}`);
    }
  }
  for (const login of filters.assignees) {
    if (login.trim() !== "") {
      push(`assignee:${login.trim()}`);
    }
  }
  if (filters.unassigned) {
    push("no:assignee");
  }
  return out;
}

function reviewToken(review: string): string | null {
  switch (review) {
    case "approved":
      return "review:approved";
    case "changes_requested":
      return "review:changes_requested";
    case "review_required":
      return "review:required";
    case "none":
      return "review:none";
    default:
      return null;
  }
}

function ciToken(ci: string): string | null {
  switch (ci) {
    case "passing":
      return "status:success";
    case "failing":
      return "status:failure";
    case "pending":
      return "status:pending";
    default:
      return null;
  }
}

function owned(raw: string): ScannedQualifier[] {
  return scannedQualifiers(raw).filter((token) => !token.negated && token.value !== "" && isOwned(token));
}

function isOwned(token: ScannedQualifier): boolean {
  const value = token.value.toLowerCase();
  switch (token.key) {
    case "org":
    case "team-review-requested":
    case "assignee":
      return true;
    case "is":
      return value === "draft";
    case "review":
      return value in REVIEW_TOKENS;
    case "status":
      return value in CI_TOKENS;
    case "no":
      return value === "assignee";
    default:
      return false;
  }
}

function ruleTokens(viewRule: string): string[] {
  return owned(viewRule).map((token) => token.text);
}

function exceptRule(tokens: readonly string[], viewRule: string): string[] {
  const skip = ruleTokens(viewRule);
  return tokens.filter((token) => !has(skip, token));
}

function removeTokens(search: string, remove: ReadonlySet<string>): string {
  const hits = scannedQualifiers(search).filter((token) => remove.has(norm(token.text)) && isOwned(token));
  let out = search;
  for (const token of hits.reverse()) {
    let start = token.start;
    let end = token.end;
    if (end < out.length && out[end] === " ") {
      end += 1;
    } else if (start > 0 && out[start - 1] === " ") {
      start -= 1;
    }
    out = out.slice(0, start) + out.slice(end);
  }
  return out;
}

function appendToken(search: string, token: string): string {
  const trimmed = search.trimEnd();
  return trimmed === "" ? token : `${trimmed} ${token}`;
}

/**
 * Writes the menu change into the field. `before` is the menu before the
 * click, `after` is the menu after it.
 */
export function applyFilterChange(search: string, before: ExpressibleFilters, after: ExpressibleFilters, viewRule: string): string {
  const previous = exceptRule(tokensFor(before), viewRule);
  const next = exceptRule(tokensFor(after), viewRule);
  const drop = new Set(previous.filter((token) => !has(next, token)).map(norm));
  let out = drop.size === 0 ? search : removeTokens(search, drop);
  const present = new Set(scannedQualifiers(out).map((token) => norm(token.text)));
  for (const token of next) {
    if (!present.has(norm(token))) {
      out = appendToken(out, token);
      present.add(norm(token));
    }
  }
  return out;
}

function applyToken<T extends ExpressibleFilters>(filters: T, token: ScannedQualifier, on: boolean): void {
  const value = token.value;
  switch (token.key) {
    case "org":
      filters.orgs = setMember(filters.orgs, value, on);
      break;
    case "is":
      if (value.toLowerCase() === "draft") {
        filters.statuses = setMember(filters.statuses, "draft", on);
      }
      break;
    case "review": {
      const mapped = REVIEW_TOKENS[value.toLowerCase()];
      if (mapped) {
        filters.reviews = setMember(filters.reviews, mapped, on);
      }
      break;
    }
    case "status": {
      const mapped = CI_TOKENS[value.toLowerCase()];
      if (mapped) {
        filters.ci = setMember(filters.ci, mapped, on);
      }
      break;
    }
    case "team-review-requested":
      filters.teams = setMember(filters.teams, value, on);
      break;
    case "assignee":
      filters.assignees = setMember(filters.assignees, value, on);
      break;
    case "no":
      if (value.toLowerCase() === "assignee") {
        filters.unassigned = on;
      }
      break;
    default:
      break;
  }
}

/** Turns menu filters on or off to match tokens added or deleted in the field. */
export function applySearchChange<T extends ExpressibleFilters>(filters: T, previousSearch: string, nextSearch: string): T {
  const copy: T = {
    ...filters,
    orgs: [...filters.orgs],
    statuses: [...filters.statuses],
    reviews: [...filters.reviews],
    ci: [...filters.ci],
    teams: [...filters.teams],
    assignees: [...filters.assignees],
  };
  const before = owned(previousSearch);
  const after = owned(nextSearch);
  for (const token of before) {
    if (!after.some((item) => norm(item.text) === norm(token.text))) {
      applyToken(copy, token, false);
    }
  }
  for (const token of after) {
    if (!before.some((item) => norm(item.text) === norm(token.text))) {
      applyToken(copy, token, true);
    }
  }
  return copy;
}

/** Drops menu tokens the view rule already applies, without changing the menu. */
export function omitRuleQualifiers(search: string, viewRule: string): string {
  const skip = new Set(ruleTokens(viewRule).map(norm));
  return skip.size === 0 ? search : removeTokens(search, skip);
}
