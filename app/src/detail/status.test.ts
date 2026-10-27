import { expect, test } from "vitest";
import { mockOverview } from "./mock";
import { approvalsSummary, autoMergeView, checksSummary, commentsSummary, mergeabilitySummary, mergeVerdict } from "./status";

test("checks count skipped as passing and list failures before running ones", () => {
  const summary = checksSummary(
    mockOverview({
      checks: [
        { name: "slow", state: "pending", url: null, required: false },
        { name: "build", state: "passed", url: null, required: true },
        { name: "docs", state: "skipped", url: null, required: false },
        { name: "test", state: "failed", url: null, required: true },
      ],
      checks_total: 4,
    }),
  );
  expect(summary).toMatchObject({ value: "2/4", tone: "bad", caption: "1 failing", ok: 2, total: 4 });
  expect(summary.attention.map((check) => check.name)).toEqual(["test", "slow"]);
  expect(checksSummary(mockOverview({ checks: [{ name: "a", state: "pending", url: null, required: true }], checks_total: 1 }))).toMatchObject({ tone: "warn", caption: "1 running" });
  expect(checksSummary(mockOverview({ checks: [], checks_total: 0 }))).toMatchObject({ tone: "muted", caption: "No checks" });
  // GitHub lists only the first checks; the total still counts them all.
  expect(checksSummary(mockOverview({ checks: [{ name: "a", state: "passed", url: null, required: true }], checks_total: 120 })).value).toBe("1/120");
});

test("comments add the conversation and the code review", () => {
  expect(commentsSummary({ issue_comments: 2, review_comments: 3 })).toMatchObject({ value: "5", caption: "2 on the PR · 3 in code" });
  expect(commentsSummary({ issue_comments: 0, review_comments: 0 }).caption).toBe("No comments");
});

test("approvals show the missing count, changes requested, and an unknown requirement", () => {
  expect(approvalsSummary({ approvals: 1, required_approvals: 2, review_decision: "review_required" })).toMatchObject({ value: "1/2", tone: "warn", caption: "1 more needed" });
  expect(approvalsSummary({ approvals: 2, required_approvals: 2, review_decision: "approved" })).toMatchObject({ tone: "good", caption: "Approved" });
  expect(approvalsSummary({ approvals: 1, required_approvals: 1, review_decision: "changes_requested" })).toMatchObject({ tone: "bad" });
  expect(approvalsSummary({ approvals: 1, required_approvals: null, review_decision: null })).toMatchObject({ value: "1/?", requiredKnown: false });
  expect(approvalsSummary({ approvals: 0, required_approvals: 0, review_decision: null })).toMatchObject({ tone: "muted", caption: "No approvals" });
});

test("mergeability reads conflicts, behind and the unknown state", () => {
  expect(mergeabilitySummary({ state: "open", mergeable: "conflicting", merge_state: "dirty" }).value).toBe("Conflicts");
  expect(mergeabilitySummary({ state: "open", mergeable: "unknown", merge_state: "unknown" }).value).toBe("Checking");
  expect(mergeabilitySummary({ state: "open", mergeable: "mergeable", merge_state: "behind" }).tone).toBe("warn");
  expect(mergeabilitySummary({ state: "open", mergeable: "mergeable", merge_state: "blocked" }).value).toBe("Clean");
  expect(mergeabilitySummary({ state: "merged", mergeable: "unknown", merge_state: "unknown" }).value).toBe("Merged");
});

test("the merge verdict names the first thing in the way", () => {
  const verdict = (overrides: Parameters<typeof mockOverview>[0]) => mergeVerdict(mockOverview(overrides));
  expect(verdict({ merge_state: "clean" })).toMatchObject({ can: true, title: "You can merge" });
  expect(verdict({ merge_state: "unstable" })).toMatchObject({ can: true, tone: "warn" });
  expect(verdict({ is_draft: true, merge_state: "draft" }).reason).toMatch(/draft/);
  expect(verdict({ merge_state: "dirty", mergeable: "conflicting" }).reason).toMatch(/conflicts/);
  expect(verdict({ merge_state: "behind" }).reason).toMatch(/behind/);
  expect(verdict({ merge_state: "clean", viewer: { ...mockOverview().viewer, permission: "read" } })).toMatchObject({ can: false, reason: "You don't have write access to this repository." });
  // Blocked: the most specific reason the overview supports.
  expect(verdict({ review_decision: "changes_requested" }).reason).toBe("A reviewer requested changes.");
  expect(verdict({ review_decision: "review_required", approvals: 0, required_approvals: 2 }).reason).toBe("It needs 2 more approvals.");
  expect(verdict({ review_decision: "approved", approvals: 2 }).reason).toBe("A required check is failing.");
  expect(verdict({ review_decision: "approved", approvals: 2, checks: [] }).reason).toBe("A branch rule isn't met yet.");
  expect(verdict({ viewer: { ...mockOverview().viewer, can_merge_as_admin: true } })).toMatchObject({ can: true, tone: "warn" });
  expect(verdict({ merge_state: "unknown", mergeable: "unknown" })).toMatchObject({ can: false, tone: "muted" });
  expect(verdict({ state: "merged" }).reason).toBe("It is already merged.");
});

test("auto-merge offers the allowed methods, squash first, and says why it can't", () => {
  expect(autoMergeView(mockOverview())).toMatchObject({ available: true, enabled: false, methods: ["squash", "merge"], defaultMethod: "squash" });
  expect(autoMergeView(mockOverview({ merge_settings: { auto_merge_allowed: true, squash: false, merge: false, rebase: true } })).defaultMethod).toBe("rebase");
  const off = (overrides: Parameters<typeof mockOverview>[0]) => autoMergeView(mockOverview(overrides)).reason;
  expect(off({ merge_settings: { auto_merge_allowed: false, squash: true, merge: true, rebase: true } })).toMatch(/turned off in this repository/);
  expect(off({ is_draft: true })).toMatch(/Drafts/);
  expect(off({ viewer: { ...mockOverview().viewer, permission: "read" } })).toMatch(/write access/);
  expect(off({ merge_state: "clean", viewer: { ...mockOverview().viewer, can_enable_auto_merge: false } })).toMatch(/direct merge/);
  expect(off({ state: "closed" })).toMatch(/not open/);
  const on = mockOverview({ auto_merge: { method: "merge", enabled_by: "kim", enabled_at: null } });
  expect(autoMergeView(on)).toMatchObject({ enabled: true, available: false });
  expect(autoMergeView({ ...on, viewer: { ...on.viewer, can_disable_auto_merge: true } })).toMatchObject({ enabled: true, available: true, reason: null });
});
