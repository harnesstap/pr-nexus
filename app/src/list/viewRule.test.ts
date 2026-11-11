import { expect, test } from "vitest";
import type { ProjectDto } from "../api";
import { AUTHOR, REVIEW_REQUESTED, defaultView } from "./model";
import { dayBounds, pageFromRows, pageIds, type PageRequest, type SearchRow } from "./paging";
import { BUMP_QUERY, MINE_QUERY, REVIEW_QUERY, analyzeViewRule, isBumpQueueQuery, matchesLocalRule } from "./viewRule";

const now = new Date("2026-10-26T12:00:00Z");

function row(partial: Partial<SearchRow> & Pick<SearchRow, "id" | "title">): SearchRow {
  return {
    number: partial.id,
    url: "https://github.com/acme/widgets/pull/1",
    body: "",
    owner: "acme",
    repo: "acme/widgets",
    author_login: "ada",
    head_ref: "main",
    is_draft: false,
    state: "open",
    checks: "success",
    review_decision: null,
    unresolved_threads: 0,
    total_threads: 0,
    involvement: AUTHOR,
    updated_at: "2026-10-26T11:00:00Z",
    created_at: "2026-10-20T11:00:00Z",
    hidden: false,
    project_ids: [],
    readiness: "ready",
    readiness_notes: [],
    conflicts: false,
    reviewers_requested: false,
    pending_reviewers: [],
    slack: null,
    starred: false,
    unread: false,
    muted: false,
    snoozed: false,
    ...partial,
  };
}

function request(viewKey: string, extra: Partial<PageRequest> = {}): PageRequest {
  const view = defaultView();
  return {
    view_key: viewKey,
    grouping: view.grouping,
    ordering: view.ordering,
    direction: view.direction,
    filters: view.filters,
    query: "",
    collapsed: [],
    cursor: null,
    limit: 50,
    now: now.toISOString(),
    ...dayBounds(now),
    ...extra,
  };
}

function ids(rows: SearchRow[], viewKey: string, extra: Partial<PageRequest> = {}): number[] {
  return pageIds(pageFromRows(rows, [] as ProjectDto[], request(viewKey, extra)).sections);
}

test("default rules match the old lists", () => {
  const rows = [
    row({ id: 1, title: "Mine" }),
    row({ id: 2, title: "Review", involvement: REVIEW_REQUESTED, state: "closed" }),
    row({ id: 3, title: "Both", involvement: AUTHOR | REVIEW_REQUESTED }),
    row({ id: 4, title: "Merged", state: "merged" }),
  ];
  expect(ids(rows, "mine", { view_rule: MINE_QUERY })).toEqual(ids(rows, "mine"));
  expect(ids(rows, "to_review", { view_rule: REVIEW_QUERY })).toEqual(ids(rows, "to_review"));
  const archived = { ...defaultView().filters, archived: true };
  expect(ids(rows, "mine", { view_rule: MINE_QUERY, filters: archived })).toEqual(ids(rows, "mine", { filters: archived }));
  expect(ids(rows, "bumps_due", { view_rule: BUMP_QUERY, due_ids: [1, 2] })).toEqual([1]);
});

test("bump:due stays local and other qualifiers go to GitHub", () => {
  const mine = analyzeViewRule(MINE_QUERY);
  expect(mine.fullyLocal).toBe(true);
  expect(mine.github).toBe("");
  const labeled = analyzeViewRule('is:open author:@me label:"needs review" bump:due');
  expect(labeled.fullyLocal).toBe(false);
  expect(labeled.github).toBe('is:open author:@me label:"needs review"');
  expect(analyzeViewRule("is:open author:@me bump:due passkeys").text).toBe("passkeys");
  expect(isBumpQueueQuery(BUMP_QUERY)).toBe(true);
  expect(isBumpQueueQuery(`${BUMP_QUERY} label:bug`)).toBe(false);
  expect(matchesLocalRule({ involvement: AUTHOR, state: "open", is_draft: false }, true, BUMP_QUERY)).toBe(true);
  expect(matchesLocalRule({ involvement: REVIEW_REQUESTED, state: "open", is_draft: false }, true, BUMP_QUERY)).toBe(false);
});

test("a view rule's free text narrows the list", () => {
  const rows = [row({ id: 1, title: "Passkeys" }), row({ id: 2, title: "Billing" })];
  expect(ids(rows, "custom:1", { view_rule: "is:open author:@me passkeys" })).toEqual([1]);
});
