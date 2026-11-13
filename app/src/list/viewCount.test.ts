import { expect, test, vi } from "vitest";
import type { ProjectDto } from "../api";
import { AUTHOR, REVIEW_REQUESTED, defaultView } from "./model";
import { pageFromRows, type PageRequest, type SearchRow } from "./paging";
import { BUMP_QUERY, MINE_QUERY, REVIEW_QUERY } from "./viewRule";
import { countPlan, formatViewCount, loadViewCounts, type RemoteFetch } from "./viewCount";

const now = new Date("2026-10-26T12:00:00Z");

const projects: ProjectDto[] = [
  { id: 1, name: "Billing", color: "#7c5cbf", archived: false, description: "", sort_order: 0, jira_links: [] },
  { id: 2, name: "Old", color: "#94a3b8", archived: true, description: "", sort_order: 1, jira_links: [] },
];

function row(partial: Partial<SearchRow> & Pick<SearchRow, "id" | "title">): SearchRow {
  return {
    number: partial.id,
    url: "https://github.com/acme/widgets/pull/1",
    body: "",
    owner: "acme",
    repo: "acme/widgets",
    author_login: "ada",
    head_ref: "feature",
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
    project_ids: [1],
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

const rows: SearchRow[] = [
  row({ id: 1, title: "Passkeys" }),
  row({ id: 2, title: "Billing export", involvement: REVIEW_REQUESTED, author_login: "lee" }),
  row({ id: 3, title: "Both", involvement: AUTHOR | REVIEW_REQUESTED }),
  row({ id: 4, title: "Merged passkeys", state: "merged" }),
  row({ id: 5, title: "Hidden draft", hidden: true, is_draft: true }),
  row({ id: 6, title: "Snoozed passkeys", snoozed: true }),
  row({ id: 7, title: "Archived only", project_ids: [2] }),
  row({ id: 8, title: "Closed passkeys", state: "closed" }),
];

function listed(request: PageRequest): number {
  const page = pageFromRows(rows, projects, request);
  return page.summary.open + page.summary.finished;
}

function listing(requests: PageRequest[]) {
  return (request: PageRequest) => {
    requests.push(request);
    return Promise.resolve({ summary: { open: listed(request), finished: 0 } });
  };
}

const views = [
  { key: "mine", query: MINE_QUERY },
  { key: "to_review", query: REVIEW_QUERY },
  { key: "bumps_due", query: BUMP_QUERY },
  { key: "custom:1", query: "is:open author:@me passkeys" },
  { key: "custom:2", query: "label:bug" },
];

test("local queries count the open list and skip hidden, snoozed, and archived-only rows", async () => {
  const requests: PageRequest[] = [];
  const counts = await loadViewCounts(views.slice(0, 4), 2, [1], {}, listing(requests), now);
  expect(counts.mine).toBe(2);
  expect(counts.to_review).toBe(2);
  expect(counts.bumps_due).toBe(2);
  expect(counts["custom:1"]).toBe(1);
  expect(requests.map((request) => request.view_key)).toEqual(["mine", "to_review", "custom:1"]);
  expect(requests.every((request) => request.github_ids === undefined)).toBe(true);
  expect(requests[0]?.filters).toEqual(defaultView().filters);
});

test("a merged query counts finished rows and an invalid query counts nothing", async () => {
  const requests: PageRequest[] = [];
  const counts = await loadViewCounts(
    [
      { key: "custom:3", query: "is:merged author:@me" },
      { key: "custom:4", query: 'label:"unfinished' },
    ],
    0,
    [],
    {},
    listing(requests),
    now,
  );
  expect(counts["custom:3"]).toBe(1);
  expect(counts["custom:4"]).toBe(0);
});

test("the bump queue uses the due count and does not list", async () => {
  const listPage = vi.fn();
  const counts = await loadViewCounts([{ key: "bumps_due", query: BUMP_QUERY }], 4, [1], {}, listPage, now);
  expect(counts.bumps_due).toBe(4);
  expect(listPage).not.toHaveBeenCalled();
});

test("a remote query counts its complete result and ignores rows outside it", async () => {
  const remote: RemoteFetch = { query: "label:bug bump:due passkeys", ids: [1, 2, 3, 8], total: 4, ok: true, openOnly: true };
  const requests: PageRequest[] = [];
  const counts = await loadViewCounts(
    [{ key: "custom:2", query: remote.query }],
    0,
    [1, 2],
    { "custom:2": remote },
    listing(requests),
    now,
  );
  expect(counts["custom:2"]).toBe(1);
  expect(requests[0]?.github_ids).toEqual([1, 2, 3, 8]);
  expect(requests[0]?.github_any_state).toBe(false);
});

test("a search that returned closed rows keeps them, and an open-only search does not", async () => {
  const closed: RemoteFetch = { query: "label:bug", ids: [8, 1], total: 2, ok: true, openOnly: false };
  const open: RemoteFetch = { ...closed, openOnly: true };
  const requests: PageRequest[] = [];
  const counts = await loadViewCounts(
    [
      { key: "custom:2", query: "label:bug" },
      { key: "custom:3", query: "label:bug" },
    ],
    0,
    [],
    { "custom:2": closed, "custom:3": open },
    listing(requests),
    now,
  );
  expect(counts["custom:2"]).toBe(2);
  expect(counts["custom:3"]).toBe(1);
  expect(requests.map((request) => request.github_any_state)).toEqual([true, false]);
});

test("a missing, failed, mismatched, or partial search is unknown, not zero", async () => {
  const partial: RemoteFetch = { query: "label:bug", ids: [1], total: 5, ok: true, openOnly: true };
  const failed: RemoteFetch = { query: "label:bug", ids: [], total: 0, ok: false, openOnly: true };
  const other: RemoteFetch = { query: "label:old", ids: [1, 2], total: 2, ok: true, openOnly: true };
  const listPage = vi.fn();
  const counts = await loadViewCounts(
    [
      { key: "custom:2", query: "label:bug" },
      { key: "custom:3", query: "label:bug" },
      { key: "custom:4", query: "label:bug" },
      { key: "custom:5", query: "label:bug" },
    ],
    0,
    [],
    { "custom:3": partial, "custom:4": failed, "custom:5": other },
    listPage,
    now,
  );
  expect(counts).toEqual({ "custom:2": null, "custom:3": null, "custom:4": null, "custom:5": null });
  expect(listPage).not.toHaveBeenCalled();
  expect(countPlan("custom:2", "label:bug", null).kind).toBe("unknown");
});

test("a failed read is unknown and the other views still count", async () => {
  const counts = await loadViewCounts(
    [
      { key: "mine", query: MINE_QUERY },
      { key: "custom:1", query: "is:open is:draft" },
    ],
    0,
    [],
    {},
    (request) => (request.view_key === "mine" ? Promise.reject(new Error("database is locked")) : Promise.resolve({ summary: { open: 3, finished: 1 } })),
    now,
  );
  expect(counts.mine).toBeNull();
  expect(counts["custom:1"]).toBe(4);
});

test("an edited bumps view counts the query instead of the queue", () => {
  expect(countPlan("bumps_due", "is:open author:@me is:draft", null).kind).toBe("local");
  expect(countPlan("bumps_due", `${BUMP_QUERY} label:bug`, null).kind).toBe("unknown");
});

test("the badge is blank at zero and caps above 99", () => {
  expect(formatViewCount(0)).toBeNull();
  expect(formatViewCount(1)).toBe("1");
  expect(formatViewCount(99)).toBe("99");
  expect(formatViewCount(100)).toBe("99+");
});
