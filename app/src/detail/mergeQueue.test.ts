import { expect, test } from "vitest";
import type { ListRow, PrOverview } from "../api";
import { mockOverview } from "./mock";
import { mergeVerdict } from "./status";
import { applyQueueVerdict, enqueueBlock, enqueuePlan, etaLabel, queueChip, rowEnqueueSkip } from "./mergeQueue";

function queueOverview(partial: Partial<NonNullable<PrOverview["merge_queue"]>> = {}): PrOverview {
  return mockOverview({
    merge_state: "clean",
    mergeable: "mergeable",
    review_decision: "approved",
    head_sha: "abc123",
    checks: [],
    merge_queue: {
      required: true,
      phase: null,
      position: null,
      eta_seconds: null,
      reason: null,
      ...partial,
    },
  });
}

function listRow(partial: Partial<ListRow>): ListRow {
  return {
    id: 7,
    number: 7,
    title: "Tighten the latch",
    url: "https://github.com/acme/widgets/pull/7",
    owner: "acme",
    repo: "acme/widgets",
    author_login: "octocat",
    head_ref: "latch",
    is_draft: false,
    state: "open",
    checks: "success",
    review_decision: "approved",
    unresolved_threads: 0,
    total_threads: 0,
    involvement: 1,
    updated_at: "2026-11-10T12:00:00Z",
    created_at: "2026-11-10T11:00:00Z",
    hidden: false,
    starred: false,
    unread: false,
    muted: false,
    snoozed: false,
    project_ids: [],
    readiness: "ready",
    readiness_notes: [],
    conflicts: false,
    reviewers_requested: false,
    pending_reviewers: [],
    slack: null,
    merge_state: "clean",
    merge_queue: { required: true, phase: null, position: null, eta_seconds: null, reason: null },
    ...partial,
  };
}

test("eta rounds to a minute or an hour", () => {
  expect(etaLabel(null)).toBeNull();
  expect(etaLabel(45)).toBe("~1 min");
  expect(etaLabel(300)).toBe("~5 min");
  expect(etaLabel(5400)).toBe("~2 hours");
});

test("row chips name the queue state", () => {
  expect(queueChip({ required: true, phase: "queued", position: 2, eta_seconds: 300, reason: null })).toEqual({
    label: "In queue · #2 · ~5 min",
    detail: null,
    tone: "queue",
  });
  expect(queueChip({ required: true, phase: "merging", position: 1, eta_seconds: 20, reason: null })?.label).toBe("Merging");
  expect(queueChip({ required: true, phase: "failed", position: 4, eta_seconds: null, reason: "Checks failed" })?.label).toBe(
    "Queue failed",
  );
  expect(queueChip({ required: true, phase: "removed", position: null, eta_seconds: null, reason: "Left the queue" })?.detail).toBe(
    "Left the queue",
  );
  expect(queueChip({ required: true, phase: null, position: null, eta_seconds: null, reason: null })).toBeNull();
});

test("a clean acme pull request can join and a draft cannot", () => {
  expect(enqueueBlock(queueOverview())).toBeNull();
  expect(enqueueBlock(queueOverview({ phase: "queued" }))).toBe("Already in the merge queue");
  expect(enqueueBlock(mockOverview({ merge_queue: { required: false, phase: null, position: null, eta_seconds: null, reason: null } }))).toBe(
    "No merge queue on the base branch",
  );
  const draft = queueOverview();
  draft.is_draft = true;
  draft.merge_state = "draft";
  expect(enqueueBlock(draft)).toBe("Still a draft");
});

test("the verdict points at the queue instead of a direct merge", () => {
  const ready = applyQueueVerdict(queueOverview(), mergeVerdict(queueOverview()));
  expect(ready.title).toBe("Ready for the merge queue");
  expect(ready.can).toBe(false);
  const queued = queueOverview({ phase: "queued", position: 2, eta_seconds: 300 });
  expect(applyQueueVerdict(queued, mergeVerdict(queued)).title).toBe("In the merge queue");
});

test("bulk plan keeps the ready pull request and names the skip", () => {
  const ready = listRow({ id: 7, number: 7 });
  const draft = listRow({ id: 8, number: 8, title: "Draft latch", is_draft: true, author_login: "octocat" });
  const plan = enqueuePlan([ready, draft]);
  expect(plan.add.map((row) => row.number)).toEqual([7]);
  expect(plan.skip).toEqual([{ row: draft, reason: "Still a draft" }]);
  expect(rowEnqueueSkip(listRow({ merge_state: "behind" }))).toBe("Branch is behind");
  expect(rowEnqueueSkip(listRow({ merge_queue: { required: true, phase: "failed", position: null, eta_seconds: null, reason: null } }))).toBeNull();
});
