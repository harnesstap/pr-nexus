import { expect, test } from "vitest";
import type { ListRow } from "../api";
import { AUTHOR, REVIEW_REQUESTED } from "./model";
import { attentionRank, NEXT_ACTIONS, nextAction, nudgeDue, perspectiveOf } from "./nextAction";

const now = new Date("2026-10-26T12:00:00Z");
const hoursAgo = (hours: number) => new Date(now.getTime() - hours * 3_600_000).toISOString();

function row(partial: Partial<ListRow> = {}): ListRow {
  return {
    id: 1,
    number: 1,
    title: "Row",
    url: "https://github.com/acme/widgets/pull/1",
    owner: "acme",
    repo: "acme/widgets",
    author_login: "ada",
    head_ref: "topic",
    is_draft: false,
    state: "open",
    checks: "success",
    review_decision: "review_required",
    unresolved_threads: 0,
    total_threads: 0,
    involvement: AUTHOR,
    updated_at: hoursAgo(1),
    created_at: hoursAgo(2),
    hidden: false,
    starred: false,
    unread: false,
    muted: false,
    snoozed: false,
    project_ids: [],
    readiness: "unknown",
    readiness_notes: [],
    conflicts: false,
    reviewers_requested: true,
    pending_reviewers: [],
    slack: null,
    ...partial,
  };
}

function slack(requestedHoursAgo: number, bumpedHoursAgo: number | null = null): ListRow["slack"] {
  return {
    requested_at: hoursAgo(requestedHoursAgo),
    last_bumped_at: bumpedHoursAgo === null ? null : hoursAgo(bumpedHoursAgo),
    bump_count: bumpedHoursAgo === null ? 0 : 1,
    workspace: "acme",
    channel_id: "C0ACME0001",
    channel_name: "team-reviews",
    permalink: null,
    root_ts: "1700000000.000100",
    source: "app",
  };
}

const mine = (partial: Partial<ListRow>) => nextAction(row(partial), "mine", now).action;
const reviewing = (partial: Partial<ListRow>, sla?: { hours: number; workingDays: boolean }) =>
  nextAction(row({ involvement: REVIEW_REQUESTED, ...partial }), "to_review", now, sla).action;

test("author states follow the documented priority", () => {
  expect(mine({ conflicts: true, checks: "failure", review_decision: "changes_requested", is_draft: true })).toBe("conflicts");
  expect(mine({ checks: "failure", review_decision: "changes_requested" })).toBe("ci_failing");
  expect(mine({ checks: "error" })).toBe("ci_failing");
  expect(mine({ review_decision: "changes_requested", is_draft: true })).toBe("changes_requested");
  expect(mine({ is_draft: true, reviewers_requested: false })).toBe("draft");
  expect(mine({ review_decision: "approved" })).toBe("ready_to_merge");
  expect(mine({ review_decision: "approved", checks: "" })).toBe("ready_to_merge");
  expect(mine({ review_decision: "approved", checks: "pending" })).toBe("checks_running");
  expect(mine({ reviewers_requested: false })).toBe("request_review");
  expect(mine({})).toBe("waiting_review");
});

test("review now follows review_sla_hours, and weekdays can skip a weekend", () => {
  const sla = { hours: 12, workingDays: false };
  expect(reviewing({ created_at: hoursAgo(20) }, sla)).toBe("review_overdue");
  expect(reviewing({ created_at: hoursAgo(20) })).toBe("review");
  const monday = new Date("2026-11-09T12:00:00Z");
  const friday = new Date(monday.getTime() - 72 * 3_600_000).toISOString();
  expect(nextAction(row({ involvement: REVIEW_REQUESTED, created_at: friday }), "to_review", monday, { hours: 48, workingDays: true }).action).toBe("review");
  expect(nextAction(row({ involvement: REVIEW_REQUESTED, created_at: friday }), "to_review", monday, { hours: 48, workingDays: false }).action).toBe("review_overdue");
});

test("a reminder is due after 24h since the last Slack send, or the review SLA without one", () => {
  expect(mine({ slack: slack(23) })).toBe("waiting_review");
  expect(mine({ slack: slack(25) })).toBe("nudge");
  expect(mine({ slack: slack(100, 5) })).toBe("waiting_review");
  expect(mine({ slack: slack(100, 30) })).toBe("nudge");
  expect(mine({ reviewers_requested: false, slack: slack(30) })).toBe("nudge");
  expect(mine({ created_at: hoursAgo(47) })).toBe("waiting_review");
  expect(mine({ created_at: hoursAgo(49) })).toBe("nudge");
  expect(nudgeDue(row({ slack: slack(100, 30) }), now)).toBe("Last Slack bump was 30h ago: send a reminder");
  expect(nudgeDue(row({ created_at: hoursAgo(24 * 5) }), now)).toBe("Waiting on review for 5d with no Slack request: send one");
});

test("closed and merged PRs are done, with the state as the reason", () => {
  expect(nextAction(row({ state: "merged", conflicts: true }), "mine", now)).toEqual({ action: "done", reason: "Merged" });
  expect(nextAction(row({ state: "closed" }), "to_review", now)).toEqual({ action: "done", reason: "Closed" });
});

test("reviewer states for To review", () => {
  expect(reviewing({})).toBe("review");
  expect(reviewing({ created_at: hoursAgo(60) })).toBe("review_overdue");
  expect(reviewing({ created_at: hoursAgo(60), slack: slack(10) })).toBe("review");
  expect(reviewing({ review_decision: "changes_requested" })).toBe("waiting_author");
  expect(reviewing({ conflicts: true })).toBe("waiting_author");
  expect(reviewing({ checks: "failure" })).toBe("waiting_author");
  expect(reviewing({ review_decision: "approved" })).toBe("reviewed");
  expect(reviewing({ is_draft: true, review_decision: "changes_requested" })).toBe("author_draft");
  expect(nextAction(row({ involvement: REVIEW_REQUESTED, checks: "failure" }), "to_review", now).reason).toBe("Checks are failing: waiting on the author");
});

test("the perspective is the reviewer's in To review and for others' PRs elsewhere", () => {
  expect(perspectiveOf({ involvement: AUTHOR }, "to_review")).toBe("reviewer");
  expect(perspectiveOf({ involvement: AUTHOR }, "mine")).toBe("author");
  expect(perspectiveOf({ involvement: REVIEW_REQUESTED }, "project:1")).toBe("reviewer");
  expect(perspectiveOf({ involvement: AUTHOR | REVIEW_REQUESTED }, "project:1")).toBe("author");
  expect(perspectiveOf({ involvement: 0 }, "project:1")).toBe("author");
});

test("attention rank orders by action, then the oldest first", () => {
  const rows = [
    row({ id: 1, created_at: hoursAgo(3) }),
    row({ id: 2, checks: "failure", created_at: hoursAgo(5) }),
    row({ id: 3, conflicts: true, created_at: hoursAgo(1) }),
    row({ id: 4, checks: "failure", created_at: hoursAgo(30) }),
    row({ id: 5, state: "merged" }),
  ];
  const order = [...rows].sort((left, right) => attentionRank(right, "mine", now) - attentionRank(left, "mine", now)).map((item) => item.id);
  expect(order).toEqual([3, 4, 2, 1, 5]);
});

test("every action has a distinct priority and the rank stays a safe integer", () => {
  expect(new Set(NEXT_ACTIONS).size).toBe(NEXT_ACTIONS.length);
  expect(Number.isSafeInteger(attentionRank(row({ conflicts: true, created_at: "1970-01-01T00:00:00Z" }), "mine", now))).toBe(true);
});

test("the same input always gives the same action", () => {
  const input = row({ slack: slack(30) });
  expect(nextAction(input, "mine", now)).toEqual(nextAction(input, "mine", now));
});
