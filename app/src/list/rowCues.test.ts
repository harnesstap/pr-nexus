import { expect, test } from "vitest";
import type { ListRow } from "../api";
import { commentsText, rowPills, staleCue, viewEmptyState } from "./rowCues";

const now = new Date("2026-10-26T12:00:00Z");

function row(partial: Partial<ListRow>): ListRow {
  return {
    id: 1,
    number: 1,
    title: "Row",
    url: "https://github.com/acme/widgets/pull/1",
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
    created_at: "2026-10-26T10:00:00Z",
    hidden: false,
    project_ids: [],
    readiness: "ready",
    readiness_notes: [],
    conflicts: false,
    reviewers_requested: false,
    pending_reviewers: [],
    slack: null,
    ...partial,
  } as ListRow;
}

function slack(requested_at: string, last_bumped_at: string | null = null): ListRow["slack"] {
  return { requested_at, last_bumped_at, permalink: "https://acme.slack.example/x" } as unknown as ListRow["slack"];
}

test("pills show draft then conflicts, only on open PRs", () => {
  expect(rowPills(row({ is_draft: true, conflicts: true }))).toEqual(["draft", "conflicts"]);
  expect(rowPills(row({ conflicts: true }))).toEqual(["conflicts"]);
  expect(rowPills(row({}))).toEqual([]);
  expect(rowPills(row({ is_draft: true, conflicts: true, state: "merged" }))).toEqual([]);
});

test("a review wait over 48h gets a waiting chip, in hours then days", () => {
  expect(staleCue(row({ reviewers_requested: true, created_at: "2026-10-25T00:00:00Z" }), now)).toBeNull();
  expect(staleCue(row({ reviewers_requested: true, created_at: "2026-10-24T08:00:00Z" }), now)?.label).toBe("Waiting 52h");
  expect(staleCue(row({ slack: slack("2026-10-21T12:00:00Z"), created_at: "2026-10-01T00:00:00Z" }), now)?.label).toBe("Waiting 5d");
});

test("drafts, approved and unrequested PRs never wait", () => {
  const old = { created_at: "2026-10-20T00:00:00Z" };
  expect(staleCue(row({ ...old, reviewers_requested: true, is_draft: true }), now)).toBeNull();
  expect(staleCue(row({ ...old, reviewers_requested: true, review_decision: "approved" }), now)).toBeNull();
  expect(staleCue(row(old), now)).toBeNull();
});

test("an open PR with no update or bump for 7 days is idle; a bump resets it", () => {
  const idle = row({ updated_at: "2026-10-17T12:00:00Z", created_at: "2026-10-01T00:00:00Z" });
  expect(staleCue(idle, now)).toMatchObject({ kind: "idle", label: "Idle 9d" });
  const bumped = row({ ...idle, slack: slack("2026-10-10T00:00:00Z", "2026-10-25T00:00:00Z"), review_decision: "approved" });
  expect(staleCue(bumped, now)).toBeNull();
  expect(staleCue(row({ ...idle, state: "closed" }), now)).toBeNull();
});

test("comment counter hides at zero and shows unresolved over total", () => {
  expect(commentsText(0, 0)).toBeNull();
  expect(commentsText(0, 3)).toBe("0/3");
  expect(commentsText(2, 5)).toBe("2/5");
  expect(commentsText(120, 150)).toBe("100+/100+");
});

test("empty state depends on the view, and never covers a search or filter miss", () => {
  expect(viewEmptyState("to_review", 0, false, false)).toEqual({ kind: "no_reviews" });
  expect(viewEmptyState("mine", 0, false, false)).toEqual({ kind: "no_prs", settings: true });
  expect(viewEmptyState("project:3", 0, false, false)).toEqual({ kind: "empty_project" });
  expect(viewEmptyState("to_review", 0, true, false)).toEqual({ kind: "rows" });
  expect(viewEmptyState("to_review", 0, false, true)).toEqual({ kind: "rows" });
  expect(viewEmptyState("to_review", 2, false, false)).toEqual({ kind: "rows" });
});
