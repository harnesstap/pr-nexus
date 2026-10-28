import { expect, test } from "vitest";
import type { ProjectDto } from "../api";
import { checkState, defaultView, filtersActive, resetFilters, toggleParent, type SavedView } from "./model";
import { PAGE_SIZE, dayBounds, pageFromRows, pageIds, summaryText, candidatesFromRows, type PageRequest, type SearchRow } from "./paging";

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
    involvement: 1,
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
    ...partial,
  };
}

const auth: ProjectDto = { id: 1, name: "Auth", color: "#7c5cbf", archived: false, description: "", sort_order: 0 , jira_links: [] };
const billing: ProjectDto = { id: 2, name: "Billing", color: "#3f6f8f", archived: false, description: "", sort_order: 1 , jira_links: [] };

function request(viewKey: string, view: SavedView, query = "", extra: Partial<PageRequest> = {}): PageRequest {
  return {
    view_key: viewKey,
    grouping: view.grouping,
    ordering: view.ordering,
    direction: view.direction,
    filters: view.filters,
    query,
    collapsed: [],
    cursor: null,
    limit: 5000,
    now: now.toISOString(),
    ...dayBounds(now),
    ...extra,
  };
}

function shownIds(rows: SearchRow[], projects: ProjectDto[], viewKey: string, view: SavedView, query = ""): number[] {
  return pageIds(pageFromRows(rows, projects, request(viewKey, view, query)).sections);
}

test("search and filters keep non-matching rows out of a parent toggle", () => {
  const rows = [row({ id: 1, title: "Latch", project_ids: [1] }), row({ id: 2, title: "Secret drawer", project_ids: [1], body: "hidden token path" })];
  const ids = shownIds(rows, [auth], "mine", defaultView(), "latch");
  expect(ids).toEqual([1]);
  expect([...toggleParent(ids, new Set())]).toEqual([1]);
});

test("checking a pull request updates every section that contains it", () => {
  const page = pageFromRows([row({ id: 7, title: "Shared", project_ids: [1, 2] })], [auth, billing], request("mine", defaultView()));
  expect(page.sections.map((section) => section.title)).toEqual(["Auth", "Billing"]);
  const selected = toggleParent(page.sections[0]!.ids, new Set());
  expect(checkState(page.sections[1]!.ids, selected)).toBe("all");
});

test("rows outside live projects land in Unassigned", () => {
  const page = pageFromRows([row({ id: 1, title: "Loose" })], [auth], request("mine", defaultView()));
  expect(page.sections.at(-1)).toMatchObject({ key: "unassigned", title: "Unassigned", project_id: null });
});

test("search is accent-insensitive and ANDs terms across title, body, repo, number, author, branch and project", () => {
  const rows = [
    row({ id: 12, number: 142, title: "Naïve latch", body: "drawer notes", author_login: "grace", head_ref: "feature/door-hinge", project_ids: [1] }),
  ];
  const ids = (query: string) => shownIds(rows, [auth], "mine", defaultView(), query);
  expect(ids("naive latch")).toEqual([12]);
  expect(ids("auth 142")).toEqual([12]);
  expect(ids("#142")).toEqual([12]);
  expect(ids("widgets drawer")).toEqual([12]);
  expect(ids("grace hinge")).toEqual([12]);
  expect(ids("naive missing")).toEqual([]);
});

test("search matches hyphenated repos, owner/name, partial input, punctuation and numbers", () => {
  const ssp = row({
    id: 7,
    number: 123,
    title: "fix(bids): v2.1 time-outs on /auction #9",
    owner: "acme",
    repo: "acme/service-ssp",
    author_login: "Grace-H",
    head_ref: "feat/ssp-timeouts",
  });
  const rows = [ssp, row({ id: 8, number: 1234, title: "Tune widgets" })];
  const ids = (query: string) => shownIds(rows, [], "mine", defaultView(), query);
  for (const query of [
    "service-ssp",
    "SERVICE-SSP",
    "acme/service-ssp",
    "ssp",
    "service-s",
    "service\u2011ssp",
    "fix(bids):",
    "v2.1",
    "time-outs",
    "/auction",
    "#9",
    "feat/ssp-timeouts",
    "grace-h",
    "ssp 123",
  ]) {
    expect(ids(query), query).toEqual([7]);
  }
  expect(ids("123")).toEqual([7, 8]);
  expect(ids("#123")).toEqual([7, 8]);
  expect(ids("#1234")).toEqual([8]);
  expect(ids("other/service-ssp")).toEqual([]);
});

test("a search reports matches in other views, behind filters, and among closed rows", () => {
  const rows = [
    row({ id: 1, title: "Tune the floor", repo: "acme/service-ssp", involvement: 2 }),
    row({ id: 2, title: "Drop the old floor", repo: "acme/service-ssp", state: "merged" }),
    row({ id: 3, title: "Floor in a draft", repo: "acme/service-ssp", is_draft: true }),
    row({ id: 4, title: "Hidden floor", repo: "acme/service-ssp", hidden: true, involvement: 3 }),
  ];
  const view = defaultView();
  view.filters.statuses = ["ready"];
  const page = pageFromRows(rows, [], request("mine", view, "service-ssp"));
  expect(pageIds(page.sections)).toEqual([]);
  expect(page.elsewhere).toEqual({ views: [{ view_key: "to_review", count: 1 }], filtered: 1, finished: 1 });
  expect(pageFromRows(rows, [], request("mine", view)).elsewhere).toBeNull();
  const withClosed = { ...view, filters: { ...view.filters, statuses: [], archived: true } };
  const all = pageFromRows(rows, [], request("mine", withClosed, "service-ssp"));
  expect(pageIds(all.sections)).toEqual([2, 3]);
  expect(all.quick.draft).toBe(1);
  expect(all.elsewhere).toEqual({ views: [{ view_key: "to_review", count: 1 }], filtered: 0, finished: 0 });
});

test("menu filters combine and reset clears only filters", () => {
  const projects = [auth, { ...billing, archived: true }];
  const rows = [
    row({ id: 1, title: "Open", owner: "acme", readiness: "ready", checks: "success" }),
    row({ id: 2, title: "Draft", owner: "other", is_draft: true, readiness: "blocked", project_ids: [2] }),
    row({ id: 3, title: "Old", hidden: true, owner: "acme" }),
    row({ id: 4, title: "Shipped", state: "merged", owner: "acme" }),
  ];
  const filtered = { ...defaultView(), grouping: "org" as const, filters: { ...defaultView().filters, orgs: ["acme"], statuses: ["ready" as const] } };
  expect(shownIds(rows, projects, "mine", filtered)).toEqual([1]);
  expect(filtersActive(filtered.filters)).toBe(true);
  const reset = resetFilters(filtered);
  expect(reset.grouping).toBe("org");
  expect(shownIds(rows, projects, "mine", reset)).toEqual([1]);
  const withArchived = { ...reset, filters: { ...reset.filters, archived: true } };
  expect(shownIds(rows, projects, "mine", withArchived).sort()).toEqual([1, 2, 4]);
});

test("merged and closed PRs leave the open list and the header count", () => {
  const rows = [
    row({ id: 1, title: "Live", project_ids: [1] }),
    row({ id: 2, title: "Shipped", state: "merged", project_ids: [1] }),
    row({ id: 3, title: "Dropped", state: "closed" }),
  ];
  const view = defaultView();
  const open = pageFromRows(rows, [auth], request("mine", view));
  expect(pageIds(open.sections)).toEqual([1]);
  expect(summaryText(open.summary, "mine")).toBe("1 open PR across 1 project");
  expect(summaryText(open.summary, "mine", "Syncing… 340 so far")).toBe("1 open PR across 1 project · Syncing… 340 so far");
  expect(shownIds(rows, [auth], "to_review", view, "shipped")).toEqual([]);
  const archived = { ...view, filters: { ...view.filters, archived: true } };
  const all = pageFromRows(rows, [auth], request("mine", archived));
  expect(pageIds(all.sections).sort()).toEqual([1, 2, 3]);
  expect(summaryText(all.summary, "mine")).toBe("1 open PR across 1 project · 2 merged or closed · 1 unassigned");
});

test("nav views pick authored, review-requested, or project rows", () => {
  const rows = [row({ id: 1, title: "Mine", involvement: 1 }), row({ id: 2, title: "Review", involvement: 2, project_ids: [1] })];
  expect(shownIds(rows, [auth], "mine", defaultView())).toEqual([1]);
  expect(shownIds(rows, [auth], "to_review", defaultView())).toEqual([2]);
  expect(shownIds(rows, [auth], "project:1", defaultView())).toEqual([2]);
});

test("empty project sections stay only without search or filters", () => {
  const projects: ProjectDto[] = [auth, { ...billing, name: "Empty" }, { id: 3, name: "Old", color: "#3f6f8f", archived: true, description: "", sort_order: 2 , jira_links: [] }];
  const rows = [row({ id: 1, title: "A", project_ids: [1] })];
  const page = pageFromRows(rows, projects, request("mine", defaultView()));
  expect(page.sections.map((section) => [section.title, section.count])).toEqual([
    ["Auth", 1],
    ["Empty", 0],
  ]);
  expect(summaryText(page.summary, "mine")).toBe("1 open PR across 2 projects");
  expect(pageFromRows(rows, projects, request("mine", defaultView(), "a")).sections.map((section) => section.title)).toEqual(["Auth"]);
  expect(summaryText(pageFromRows([], projects, request("project:2", defaultView())).summary, "project:2")).toBe("0 open PRs in this project");
});

test("pages walk the whole stream with counts over every match", () => {
  const rows = Array.from({ length: 130 }, (_, index) => row({ id: index + 1, title: `PR ${index + 1}`, updated_at: new Date(now.getTime() - index * 60_000).toISOString() }));
  const view = { ...defaultView(), grouping: "repository" as const };
  const first = pageFromRows(rows, [], request("mine", view, "", { limit: PAGE_SIZE }));
  expect(first.entries).toHaveLength(PAGE_SIZE);
  expect(first.sections[0]!.count).toBe(130);
  expect(first.summary.open).toBe(130);
  const seen: number[] = [];
  let cursor: string | null = null;
  do {
    const next = pageFromRows(rows, [], request("mine", view, "", { limit: PAGE_SIZE, cursor }));
    seen.push(...next.entries.flatMap((entry) => (entry.row ? [entry.row.id] : [])));
    cursor = next.next_cursor;
  } while (cursor);
  expect(seen).toEqual(rows.map((item) => item.id));
  expect(first.entries[1]!.row).not.toHaveProperty("body");
});

test("collapsed sections keep their header and counts but skip rows", () => {
  const rows = [row({ id: 1, title: "A", project_ids: [1] }), row({ id: 2, title: "B" })];
  const page = pageFromRows(rows, [auth], request("mine", defaultView(), "", { collapsed: ["project:1"] }));
  expect(page.entries.map((entry) => [entry.section, entry.row?.id ?? null])).toEqual([
    ["project:1", null],
    ["unassigned", null],
    ["unassigned", 2],
  ]);
  expect(page.sections[0]).toMatchObject({ count: 1, ids: [1] });
});

test("link candidates put an exact number first and skip linked rows", () => {
  const rows = [row({ id: 1, number: 120, title: "Newer", project_ids: [1] }), row({ id: 2, number: 12, title: "Older", updated_at: "2026-10-01T00:00:00Z" }), row({ id: 3, number: 312, title: "Newest" })];
  expect(candidatesFromRows(rows, [auth], new Set([1]), "12", 8).map((item) => item.id)).toEqual([2, 3]);
});

function quickRows(): SearchRow[] {
  // Same fixture as the Rust `quick_fixture`: 1 draft, 2 nobody asked, 3 failing CI,
  // 4 conflicts, 5 declined, 6 ready, 7 asked on Slack, 8 reviewer requested on GitHub.
  return [
    row({ id: 1, title: "One", is_draft: true }),
    row({ id: 2, title: "Two", owner: "acme-labs" }),
    row({ id: 3, title: "Three", checks: "failure", reviewers_requested: true }),
    row({ id: 4, title: "Four", conflicts: true, review_decision: "approved", owner: "acme-labs" }),
    row({ id: 5, title: "Five", review_decision: "changes_requested" }),
    row({ id: 6, title: "Six", review_decision: "approved" }),
    row({
      id: 7,
      title: "Seven",
      slack: { requested_at: "2026-10-26T09:00:00Z", last_bumped_at: null, bump_count: 0, workspace: "acme", channel_id: "C0ACME0001", channel_name: "team-reviews", permalink: null, root_ts: "1.0", source: "app" },
    }),
    row({ id: 8, title: "Eight", reviewers_requested: true }),
  ];
}

test("quick filters match the backend definitions", () => {
  const base = { ...defaultView(), grouping: "repository" as const };
  const expected = { draft: [1], no_review: [2], ci_trouble: [3, 4], declined: [5], ready: [6] } as const;
  for (const [quick, ids] of Object.entries(expected)) {
    const view = { ...base, filters: { ...base.filters, quick: quick as keyof typeof expected } };
    expect(shownIds(quickRows(), [], "all", view).sort((a, b) => a - b)).toEqual(ids);
  }
});

test("quick counts follow search and menu filters but ignore the quick filter", () => {
  const base = { ...defaultView(), grouping: "repository" as const };
  const view = { ...base, filters: { ...base.filters, quick: "draft" as const } };
  const page = pageFromRows(quickRows(), [], request("all", view));
  expect(page.quick).toEqual({ draft: 1, no_review: 1, ci_trouble: 2, declined: 1, ready: 1 });
  expect(page.summary.open).toBe(1);
  const labs = pageFromRows(quickRows(), [], request("all", { ...view, filters: { ...view.filters, orgs: ["acme-labs"] } }));
  expect(labs.quick).toEqual({ draft: 0, no_review: 1, ci_trouble: 1, declined: 0, ready: 0 });
  const searched = pageFromRows(quickRows(), [], request("all", { ...view, filters: { ...view.filters, quick: "ci_trouble" } }, "four"));
  expect(pageIds(searched.sections)).toEqual([4]);
  expect(searched.quick.ci_trouble).toBe(1);
  expect(filtersActive(view.filters)).toBe(true);
  expect(resetFilters(view).filters.quick).toBeNull();
});

test("To review drops empty project groups and says nothing is requested", () => {
  const rows = [row({ id: 1, title: "Review", involvement: 2, project_ids: [1] })];
  const page = pageFromRows(rows, [auth, billing], request("to_review", defaultView()));
  expect(page.sections.map((section) => section.title)).toEqual(["Auth"]);
  const empty = pageFromRows([], [auth, billing], request("to_review", defaultView()));
  expect(empty.sections).toEqual([]);
  expect(summaryText(empty.summary, "to_review")).toBe("No reviews requested from you");
});

test("waiting longest puts the oldest review wait first and unrequested rows last", () => {
  const rows = [
    row({ id: 1, title: "Fresh", reviewers_requested: true, created_at: "2026-10-26T00:00:00Z" }),
    row({ id: 2, title: "Old", reviewers_requested: true, created_at: "2026-10-20T00:00:00Z" }),
    row({ id: 3, title: "Unasked", created_at: "2026-10-01T00:00:00Z" }),
  ];
  const view = { ...defaultView(), grouping: "org" as const, ordering: "waiting" as const, direction: "desc" as const };
  expect(shownIds(rows, [], "mine", view)).toEqual([2, 1, 3]);
});

test("starred rows pin to the top within the current sort, and the Starred filter keeps only them", () => {
  const rows = [row({ id: 1, title: "One" }), row({ id: 2, title: "Two", starred: true }), row({ id: 3, title: "Three" }), row({ id: 4, title: "Four", starred: true })];
  const view = defaultView();
  const all = shownIds(rows, [], "mine", view);
  expect(all.slice(0, 2).sort()).toEqual([2, 4]);
  const unstarredOrder = shownIds(rows.map((item) => ({ ...item, starred: false })), [], "mine", view).filter((id) => id !== 2 && id !== 4);
  expect(all.slice(2)).toEqual(unstarredOrder);
  const starredOnly = { ...view, filters: { ...view.filters, starred: true } };
  expect(shownIds(rows, [], "mine", starredOnly).sort()).toEqual([2, 4]);
});
