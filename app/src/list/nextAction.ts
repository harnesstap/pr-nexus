import type { ListRow } from "../api";
import { AUTHOR, ciBucket, REVIEW_REQUESTED } from "./model";
import { STALE_REVIEW_HOURS, waitingSince } from "./rowCues";

const HOUR_MS = 60 * 60 * 1000;

/**
 * A Slack request or bump older than this (or, without one, a GitHub review
 * request waiting longer than `STALE_REVIEW_HOURS`) makes a reminder due.
 * Same 24 h as the "Bump due" Slack filter. Mirrors `NUDGE_HOURS` in
 * `app/src-tauri/src/listing.rs`.
 */
export const NUDGE_HOURS = 24;

/**
 * The one thing to do next on a PR (§8.2), from the row data alone. Ordered
 * by how much attention it needs: the "Needs attention" sort follows this list.
 * Mirrors `NextAction` in `app/src-tauri/src/listing.rs`.
 */
export const NEXT_ACTIONS = [
  "conflicts",
  "ci_failing",
  "changes_requested",
  "ready_to_merge",
  "review_overdue",
  "draft",
  "request_review",
  "nudge",
  "review",
  "checks_running",
  "waiting_review",
  "waiting_author",
  "reviewed",
  "author_draft",
  "done",
] as const;
export type NextAction = (typeof NEXT_ACTIONS)[number];

/** Whose next step it is: the author's, or the reviewer's (To review). */
export type Perspective = "author" | "reviewer";

/** To review always reads as a reviewer; elsewhere, PRs you did not author but must review do too. */
export function perspectiveOf(row: Pick<ListRow, "involvement">, viewKey: string): Perspective {
  if (viewKey === "to_review") {
    return "reviewer";
  }
  return (row.involvement & AUTHOR) === 0 && (row.involvement & REVIEW_REQUESTED) !== 0 ? "reviewer" : "author";
}

export interface NextActionResult {
  action: NextAction;
  /** Tooltip: why this is the next step. */
  reason: string;
}

function hours(ms: number): string {
  const total = Math.floor(ms / HOUR_MS);
  return total < 72 ? `${total}h` : `${Math.floor(total / 24)}d`;
}

/** Pure: same row, view and clock always give the same action. */
export function nextAction(row: ListRow, viewKey: string, now: Date): NextActionResult {
  if (row.state !== "open") {
    return { action: "done", reason: row.state === "merged" ? "Merged" : "Closed" };
  }
  return perspectiveOf(row, viewKey) === "reviewer" ? reviewerAction(row, now) : authorAction(row, now);
}

function authorAction(row: ListRow, now: Date): NextActionResult {
  const ci = ciBucket(row.checks);
  const decision = row.review_decision;
  if (row.conflicts) {
    return { action: "conflicts", reason: "GitHub reports merge conflicts: rebase or merge the base branch" };
  }
  if (ci === "failing") {
    return { action: "ci_failing", reason: "Checks are failing: open them to fix CI" };
  }
  if (decision === "changes_requested") {
    return { action: "changes_requested", reason: "A reviewer requested changes: address them, then ask again" };
  }
  if (row.is_draft) {
    return { action: "draft", reason: "Draft PR: mark it ready for review when it is done" };
  }
  if (decision === "approved") {
    return ci === "pending"
      ? { action: "checks_running", reason: "Approved; waiting for checks to finish" }
      : { action: "ready_to_merge", reason: ci === "passing" ? "Approved, checks green and no conflicts" : "Approved and no conflicts (no checks reported)" };
  }
  if (!row.reviewers_requested && !row.slack) {
    return { action: "request_review", reason: "Nobody is asked to review it yet, on GitHub or Slack" };
  }
  const nudge = nudgeDue(row, now);
  if (nudge) {
    return { action: "nudge", reason: nudge };
  }
  return { action: "waiting_review", reason: "Review requested recently: give reviewers time" };
}

/** Why a reminder is due, or null while the last request is recent. */
export function nudgeDue(row: ListRow, now: Date): string | null {
  if (row.slack) {
    const stamp = row.slack.last_bumped_at ?? row.slack.requested_at;
    const age = now.getTime() - new Date(stamp).getTime();
    const what = row.slack.last_bumped_at ? "Last Slack bump" : "Slack request";
    return age >= NUDGE_HOURS * HOUR_MS ? `${what} was ${hours(age)} ago: send a reminder` : null;
  }
  const since = waitingSince(row);
  if (since === null) {
    return null;
  }
  const age = now.getTime() - since;
  return age > STALE_REVIEW_HOURS * HOUR_MS ? `Waiting on review for ${hours(age)} with no Slack request: send one` : null;
}

function reviewerAction(row: ListRow, now: Date): NextActionResult {
  const decision = row.review_decision;
  if (row.is_draft) {
    return { action: "author_draft", reason: "Still a draft: the author has not asked for review yet" };
  }
  if (decision === "changes_requested") {
    return { action: "waiting_author", reason: "Changes were requested: waiting on the author" };
  }
  if (row.conflicts || ciBucket(row.checks) === "failing") {
    return { action: "waiting_author", reason: row.conflicts ? "Merge conflicts: waiting on the author" : "Checks are failing: waiting on the author" };
  }
  if (decision === "approved") {
    return { action: "reviewed", reason: "Already approved: nothing needed from you" };
  }
  const since = waitingSince(row) ?? new Date(row.created_at).getTime();
  const age = now.getTime() - since;
  if (age > STALE_REVIEW_HOURS * HOUR_MS) {
    return { action: "review_overdue", reason: `Waiting on your review for ${hours(age)}` };
  }
  return { action: "review", reason: `Review requested ${hours(age)} ago` };
}

/**
 * Sort key for "Needs attention", read descending: the most urgent action
 * first, then the oldest PR within an action. Mirrors `attention_rank` in
 * `listing.rs`.
 */
export function attentionRank(row: ListRow, viewKey: string, now: Date): number {
  const priority = NEXT_ACTIONS.length - NEXT_ACTIONS.indexOf(nextAction(row, viewKey, now).action);
  const since = waitingSince(row) ?? new Date(row.created_at).getTime();
  const age = Math.max(0, now.getTime() - since);
  return priority * ATTENTION_STEP + Math.min(age, ATTENTION_STEP - 1);
}

/** Wider than any PR age in ms (about 317 years), so priority always wins. */
export const ATTENTION_STEP = 1e13;
