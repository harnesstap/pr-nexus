import { expect, test, vi } from "vitest";
import type { ProjectDto } from "../api";
import { defaultView } from "./model";
import { pageFromRows, type PageRequest, type SearchRow } from "./paging";
import { countRequest } from "./viewCount";
import { loadProjectCounts } from "./projectCount";

const now = new Date("2026-10-26T12:00:00Z");

const projects: ProjectDto[] = [
  { id: 1, name: "Auth", color: "#7c5cbf", archived: false, description: "", sort_order: 2, jira_links: [] },
  { id: 2, name: "Billing", color: "#3f6f8f", archived: false, description: "", sort_order: 0, jira_links: [] },
  { id: 3, name: "Old", color: "#94a3b8", archived: true, description: "", sort_order: 1, jira_links: [] },
  { id: 4, name: "Empty", color: "#2f855a", archived: false, description: "", sort_order: 3, jira_links: [] },
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
    involvement: 0,
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

const rows: SearchRow[] = [
  row({ id: 1, title: "Passkeys", project_ids: [1], involvement: 1 }),
  row({ id: 2, title: "Shared", project_ids: [1, 2] }),
  row({ id: 3, title: "Hidden billing", project_ids: [2], hidden: true }),
  row({ id: 4, title: "Merged billing", project_ids: [2], state: "merged" }),
  row({ id: 5, title: "Closed auth", project_ids: [1], state: "closed" }),
  row({ id: 6, title: "Snoozed auth", project_ids: [1], snoozed: true }),
  row({ id: 7, title: "Draft billing", project_ids: [2], is_draft: true }),
  row({ id: 8, title: "Archived only", project_ids: [3] }),
  row({ id: 9, title: "Live and archived", project_ids: [1, 3] }),
  row({ id: 10, title: "Unassigned" }),
];

function listed(request: PageRequest) {
  return pageFromRows(rows, projects, request);
}

test("project counts match the open list a project view shows", async () => {
  const requests: PageRequest[] = [];
  const counts = await loadProjectCounts(
    [1, 2, 4],
    (request) => {
      requests.push(request);
      return Promise.resolve(listed(request));
    },
    now,
  );
  expect(requests).toHaveLength(1);
  expect(requests[0]).toMatchObject({
    view_key: "sidebar-projects",
    grouping: "project",
    query: "",
    view_rule: "",
    filters: defaultView().filters,
    limit: 1,
  });
  expect(requests[0]?.github_ids).toBeUndefined();

  for (const id of [1, 2, 4]) {
    const direct = listed(countRequest(`project:${id}`, "", [], null, true, now));
    expect(counts[id]).toBe(direct.summary.open + direct.summary.finished);
  }
  // Auth: passkeys, the shared PR, and the one also in the archived project.
  // Billing: the shared PR and the draft. Hidden, merged, closed, and snoozed stay out.
  expect(counts).toEqual({ 1: 3, 2: 2, 4: 0 });
});

test("an empty project list does not read the cache", async () => {
  const listPage = vi.fn();
  expect(await loadProjectCounts([], listPage, now)).toEqual({});
  expect(listPage).not.toHaveBeenCalled();
});

test("a failed read is unknown for every project, not zero", async () => {
  const counts = await loadProjectCounts([1, 4], () => Promise.reject(new Error("database is locked")), now);
  expect(counts).toEqual({ 1: null, 4: null });
});
