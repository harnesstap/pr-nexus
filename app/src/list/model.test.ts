import { expect, test } from "vitest";
import type { ListRow, ProjectDto } from "../api";
import {
  bulkIds,
  checkState,
  defaultView,
  filtersActive,
  groupRows,
  hiddenSelectedCount,
  matchesSearch,
  parseViewState,
  renameError,
  resetFilters,
  rowsForNav,
  toggleParent,
  visibleIds,
  visibleRows,
} from "./model";

const now = new Date("2026-10-26T12:00:00Z");

function row(partial: Partial<ListRow> & Pick<ListRow, "id" | "title">): ListRow {
  return {
    number: partial.id ?? 1,
    url: "https://github.com/acme/widgets/pull/1",
    body: "",
    owner: "acme",
    repo: "acme/widgets",
    author_login: "ada",
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
    slack: null,
    ...partial,
  };
}

const auth: ProjectDto = { id: 1, name: "Auth", color: "#7c5cbf", archived: false, description: "", sort_order: 0 };
const billing: ProjectDto = {
  id: 2,
  name: "Billing",
  color: "#3f6f8f",
  archived: false,
  description: "",
  sort_order: 1,
};

test("section and global checkboxes derive none, partial, and all", () => {
  const ids = [1, 2, 3];
  expect(checkState(ids, new Set())).toBe("none");
  expect(checkState(ids, new Set([2]))).toBe("partial");
  expect(checkState(ids, new Set([1, 2, 3]))).toBe("all");
  expect(checkState([], new Set([1]))).toBe("none");
});

test("a parent toggle selects or clears only the visible children", () => {
  const visible = [1, 2];
  const selected = toggleParent(visible, new Set([9]));
  expect([...selected].sort()).toEqual([1, 2, 9]);
  const cleared = toggleParent(visible, selected);
  expect([...cleared]).toEqual([9]);
  const fromPartial = toggleParent(visible, new Set([1]));
  expect([...fromPartial].sort()).toEqual([1, 2]);
});

test("search and filters keep hidden rows out of a parent toggle", () => {
  const projects = [auth];
  const rows = [
    row({ id: 1, title: "Latch", project_ids: [1] }),
    row({ id: 2, title: "Secret drawer", project_ids: [1], body: "hidden token path" }),
  ];
  const view = defaultView();
  const shown = visibleRows(rows, projects, "mine", view, "latch", now);
  expect(shown.map((item) => item.id)).toEqual([1]);
  const selected = toggleParent(visibleIds(groupRows(shown, projects, view, now)), new Set());
  expect([...selected]).toEqual([1]);
});

test("checking a pull request updates every section that contains it", () => {
  const shared = row({ id: 7, title: "Shared", project_ids: [1, 2] });
  const sections = groupRows([shared], [auth, billing], defaultView(), now);
  expect(sections.map((section) => section.title)).toEqual(["Auth", "Billing"]);
  const selected = toggleParent(sections[0]!.rows.map((item) => item.id), new Set());
  expect(checkState(sections[0]!.rows.map((item) => item.id), selected)).toBe("all");
  expect(checkState(sections[1]!.rows.map((item) => item.id), selected)).toBe("all");
});

test("bulk ids dedupe and the hidden-selected count ignores visible rows", () => {
  const selected = new Set([4, 4, 9, 2]);
  expect(bulkIds(selected)).toEqual([2, 4, 9]);
  expect(hiddenSelectedCount(selected, [2, 4])).toBe(1);
});

test("rename rejects an empty or duplicate name and Unassigned is not a project", () => {
  expect(renameError("  ", [auth], auth.id)).toMatch(/empty/);
  expect(renameError("Billing", [auth, billing], auth.id)).toMatch(/already exists/);
  expect(renameError("  Auth v2  ", [auth, billing], auth.id)).toBeNull();
  const sections = groupRows([row({ id: 1, title: "Loose" })], [auth], defaultView(), now);
  expect(sections[0]?.title).toBe("Unassigned");
  expect(sections[0]?.projectId).toBeNull();
});

test("search is accent-insensitive and ANDs terms across title, body, repo, and number", () => {
  const item = row({ id: 12, number: 142, title: "Naïve latch", body: "drawer notes", repo: "acme/widgets" });
  expect(matchesSearch(item, ["Auth"], "naive latch")).toBe(true);
  expect(matchesSearch(item, ["Auth"], "auth 142")).toBe(true);
  expect(matchesSearch(item, ["Auth"], "#142")).toBe(true);
  expect(matchesSearch(item, ["Auth"], "naive missing")).toBe(false);
  expect(matchesSearch(item, ["Auth"], "widgets drawer")).toBe(true);
});

test("menu filters combine and reset clears only filters", () => {
  const projects = [auth, { ...billing, archived: true }];
  const rows = [
    row({ id: 1, title: "Open", owner: "acme", readiness: "ready", checks: "success" }),
    row({ id: 2, title: "Draft", owner: "other", is_draft: true, readiness: "blocked", project_ids: [2] }),
    row({ id: 3, title: "Old", hidden: true, owner: "acme" }),
  ];
  const filtered = {
    ...defaultView(),
    grouping: "org" as const,
    filters: { ...defaultView().filters, orgs: ["acme"], statuses: ["ready" as const] },
  };
  expect(visibleRows(rows, projects, "mine", filtered, "", now).map((item) => item.id)).toEqual([1]);
  expect(filtersActive(filtered.filters)).toBe(true);
  const reset = resetFilters(filtered);
  expect(reset.grouping).toBe("org");
  expect(filtersActive(reset.filters)).toBe(false);
  expect(visibleRows(rows, projects, "mine", reset, "", now).map((item) => item.id)).toEqual([1]);
  const withArchived = { ...reset, filters: { ...reset.filters, archived: true } };
  expect(visibleRows(rows, projects, "mine", withArchived, "", now).map((item) => item.id).sort()).toEqual([1, 2, 3]);
});

test("nav views and saved view state round-trip", () => {
  const authored = row({ id: 1, title: "Mine", involvement: 1 });
  const review = row({ id: 2, title: "Review", involvement: 2, project_ids: [1] });
  expect(rowsForNav([authored, review], "mine").map((item) => item.id)).toEqual([1]);
  expect(rowsForNav([authored, review], "to_review").map((item) => item.id)).toEqual([2]);
  expect(rowsForNav([authored, review], "project:1").map((item) => item.id)).toEqual([2]);
  const saved = parseViewState(
    JSON.stringify({ grouping: "status", ordering: "number", direction: "asc", filters: { slack: ["bumped"] } }),
  );
  expect(saved.grouping).toBe("status");
  expect(saved.ordering).toBe("number");
  expect(saved.filters.slack).toEqual(["bumped"]);
  expect(saved.show.threads).toBe(true);
  expect(parseViewState("not json").grouping).toBe("project");
});
