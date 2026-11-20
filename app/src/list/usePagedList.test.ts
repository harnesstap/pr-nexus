import { act, renderHook } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { pageFromRows, type PageDto, type PageRequest, type SearchRow } from "./paging";
import { defaultView } from "./model";
import { usePagedList, type PageParams } from "./usePagedList";

vi.mock("./paging", async (importOriginal) => ({ ...(await importOriginal<typeof import("./paging")>()), PAGE_SIZE: 3 }));

function row(id: number): SearchRow {
  return {
    id, number: id, title: `T${id}`, url: `u${id}`, body: "", owner: "acme", repo: "acme/w", author_login: "ada", head_ref: `b${id}`,
    is_draft: false, state: "open", checks: "success", review_decision: null, unresolved_threads: 0, total_threads: 0, involvement: 1,
    updated_at: "2026-10-26T11:00:00Z", created_at: `2026-10-${10 + id}T11:00:00Z`, hidden: false, project_ids: [], readiness: "ready",
    readiness_notes: [], conflicts: false, reviewers_requested: false, pending_reviewers: [], slack: null, starred: false, unread: false,
    muted: false, snoozed: false,
  };
}

const flush = () =>
  act(async () => {
    for (let i = 0; i < 5; i += 1) await Promise.resolve();
  });

test("a next page read from a cursor that an in-place refresh replaced is dropped, so no row is listed twice", async () => {
  let data = [1, 2, 3, 4, 5, 6].map(row);
  const held: { request: PageRequest; resolve: (page: PageDto) => void }[] = [];
  let hold = false;
  const loadPage = (request: PageRequest) =>
    hold ? new Promise<PageDto>((resolve) => held.push({ request, resolve })) : Promise.resolve(pageFromRows(data, [], request));
  const view = defaultView();
  const params: PageParams = { view_key: "sidebar-x", grouping: "updated", ordering: view.ordering, direction: view.direction, filters: view.filters, query: "", collapsed: [] };
  const now = new Date("2026-10-26T12:00:00Z");
  const hook = renderHook(({ version }) => usePagedList(loadPage, params, version, now), { initialProps: { version: 0 } });
  await flush();
  const first = hook.result.current.entries.filter((e) => e.row).map((e) => e.row!.id);
  expect(first.length).toBeGreaterThan(0);
  hold = true;
  // A sync drops the first row, then the list re-reads in place...
  data = data.filter((item) => item.id !== first[0]);
  hook.rerender({ version: 1 });
  // ...and the user scrolls to the end while that refresh is in flight.
  act(() => hook.result.current.loadMore());
  expect(held.length).toBe(2);
  const [refresh, more] = held;
  await act(async () => refresh.resolve(pageFromRows(data, [], refresh.request)));
  await act(async () => more.resolve(pageFromRows(data, [], more.request)));
  await flush();
  const ids = hook.result.current.entries.filter((e) => e.row).map((e) => e.row!.id);
  expect(new Set(ids).size).toBe(ids.length);
});
