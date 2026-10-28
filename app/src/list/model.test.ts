import { expect, test } from "vitest";
import type { BulkResult, ProjectDto } from "../api";
import {
  bulkIds,
  bulkToast,
  sentToast,
  checkState,
  slackStatus,
  threadsLabel,
  dragIds,
  parseDragIds,
  hiddenSelectedCount,
  liveLabel,
  parseViewState,
  renameError,
  syncLabel,
  toggleParent,
} from "./model";

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

test("bulk ids dedupe and the hidden-selected count ignores visible rows", () => {
  const selected = new Set([4, 4, 9, 2]);
  expect(bulkIds(selected)).toEqual([2, 4, 9]);
  expect(hiddenSelectedCount(selected, [2, 4])).toBe(1);
});

test("rename rejects an empty or duplicate name and Unassigned is not a project", () => {
  expect(renameError("  ", [auth], auth.id)).toMatch(/empty/);
  expect(renameError("Billing", [auth, billing], auth.id)).toMatch(/already exists/);
  expect(renameError("  Auth v2  ", [auth, billing], auth.id)).toBeNull();
});

test("saved view state round-trips", () => {
  const saved = parseViewState(
    JSON.stringify({ grouping: "status", ordering: "number", direction: "asc", filters: { slack: ["bumped"] } }),
  );
  expect(saved.grouping).toBe("status");
  expect(saved.ordering).toBe("number");
  expect(saved.filters.slack).toEqual(["bumped"]);
  expect(saved.show.threads).toBe(true);
  expect(parseViewState("not json").grouping).toBe("project");
  expect(parseViewState(JSON.stringify({ filters: { quick: "ready" } })).filters.quick).toBe("ready");
  expect(parseViewState(JSON.stringify({ filters: { quick: "bogus" } })).filters.quick).toBeNull();
  expect(parseViewState(null).filters.quick).toBeNull();
});

test("demo mode shows a neutral sync label instead of an error", () => {
  const now = new Date("2026-10-26T00:00:00Z");
  const demo = { phase: "demo", message: "Demo data · sync disabled", synced_at: null, paused: false, revision: 0 };
  expect(syncLabel(demo, now)).toBe("Demo data · sync off");
  expect(syncLabel({ ...demo, phase: "error", message: "Sync failed · boom" }, now)).toBe(
    "Sync failed · boom",
  );
});

test("syncing shows the running count once the first page is stored", () => {
  const now = new Date("2026-10-26T00:00:00Z");
  const base = { phase: "syncing", message: "Syncing…", synced_at: null, paused: false, revision: 0 };
  expect(syncLabel(base, now)).toBe("Syncing…");
  expect(syncLabel({ ...base, message: "Syncing… 340 so far" }, now)).toBe("Syncing… 340 so far");
  expect(syncLabel({ ...base, message: "Synced · 12 pull requests" }, now)).toBe("Syncing…");
});

test("an incremental pass keeps its updated count next to the age", () => {
  const now = new Date("2026-10-26T00:02:00Z");
  const base = { phase: "ok", message: "Synced · 3 updated", synced_at: "2026-10-26T00:00:00Z", paused: false, revision: 4 };
  expect(syncLabel(base, now)).toMatch(/^Synced · 3 updated · /);
  expect(syncLabel({ ...base, message: "Synced · 0 updated" }, now)).not.toMatch(/updated/);
  expect(syncLabel({ ...base, message: "Synced · 120 pull requests" }, now)).not.toMatch(/pull requests/);
});

test("slack status prefers the latest bump and threads label explains the count", () => {
  const now = new Date("2026-10-26T12:00:00Z");
  const slack = {
    workspace: "team",
    channel_id: "C1",
    channel_name: "reviews",
    permalink: null,
    root_ts: "1.0",
    source: "app",
    bump_count: 0,
    last_bumped_at: null,
    requested_at: "2026-10-24T12:00:00Z",
  };
  expect(slackStatus(null, now)).toBeNull();
  expect(slackStatus(slack, now)).toEqual({ label: "Requested · 2d ago", title: "team · #reviews · requested 2d ago" });
  expect(slackStatus({ ...slack, bump_count: 1, last_bumped_at: "2026-10-26T07:00:00Z" }, now)?.label).toBe("Bumped · 5h ago");
  expect(sentToast(slack)).toEqual({ text: "Review requested in team · #reviews", link: null });
  const dm = { ...slack, channel_id: "D0123456789", channel_name: "D0123456789", permalink: "https://acme.slack.com/archives/D0123456789/p1", bump_count: 1 };
  expect(sentToast(dm)).toEqual({ text: "Bumped in team · DM", link: "https://acme.slack.com/archives/D0123456789/p1" });
  expect(threadsLabel(2, 5)).toEqual({ text: "2", title: "2 unresolved of 5 review threads" });
  expect(threadsLabel(120, 130).text).toBe("100+");
});

test("dragging a selected row carries the selection", () => {
  expect(dragIds(3, new Set([2, 3]))).toEqual([2, 3]);
  expect(dragIds(4, new Set([2, 3]))).toEqual([4]);
  expect(parseDragIds("[1,2.5,\"x\",3]")).toEqual([1, 3]);
  expect(parseDragIds("nope")).toEqual([]);
});

test("bulk toast counts outcomes and drops zero counts", () => {
  const outcome = (value: "sent" | "skipped" | "failed") => ({ outcome: value }) as BulkResult["results"][number];
  const done = (...values: ("sent" | "skipped" | "failed")[]) => ({ batch_id: "b", results: values.map(outcome), indicators: [] });
  expect(bulkToast(done("sent", "sent", "failed"))).toBe("Slack: 2 sent · 1 failed");
  expect(bulkToast(done("skipped"))).toBe("Slack: 1 skipped");
  expect(bulkToast(done())).toBe("Slack: nothing sent");
});

test("the live line says active with its last check, off, or why it is unavailable", () => {
  const now = new Date("2026-10-26T00:05:00Z");
  const base = { phase: "ok", message: "Synced", synced_at: null, paused: false, revision: 0 };
  expect(liveLabel(base, now)).toBeNull();
  expect(liveLabel({ ...base, live: { state: "demo", checked_at: null, detail: null } }, now)).toBeNull();
  expect(liveLabel({ ...base, live: { state: "active", checked_at: "2026-10-26T00:02:00Z", detail: null } }, now)).toBe(
    "Live updates on · checked 3m ago",
  );
  expect(liveLabel({ ...base, live: { state: "off", checked_at: null, detail: null } }, now)).toBe("Live updates off");
  const reason = "Live updates unavailable with this token type: falls back to periodic sync";
  expect(liveLabel({ ...base, live: { state: "unavailable", checked_at: null, detail: reason } }, now)).toBe(reason);
});
