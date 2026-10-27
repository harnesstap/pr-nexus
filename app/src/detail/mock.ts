import type { PrDetail, PrOverview } from "../api";

/** A made-up open PR for tests, the preview and the screenshot. Override what a case needs. */
export function mockOverview(overrides: Partial<PrOverview> = {}): PrOverview {
  return {
    state: "open",
    is_draft: false,
    base_ref: "main",
    head_ref: "feature/checkout-steps",
    mergeable: "mergeable",
    merge_state: "blocked",
    review_decision: "changes_requested",
    issue_comments: 3,
    review_comments: 5,
    checks: [
      { name: "build", state: "passed", url: null, required: true },
      { name: "lint", state: "passed", url: null, required: true },
      { name: "unit tests", state: "failed", url: "https://example.test/runs/1", required: true },
      { name: "preview deploy", state: "pending", url: null, required: false },
    ],
    checks_total: 4,
    approvals: 1,
    required_approvals: 2,
    reviewers: [
      { kind: "user", login: "kim", name: "Kim Lee", avatar_url: null, state: "approved" },
      { kind: "user", login: "sam", name: "Sam Park", avatar_url: null, state: "changes_requested" },
      { kind: "team", login: "acme/web", name: "Web", avatar_url: null, state: "pending" },
    ],
    participants: [
      { kind: "user", login: "ada", name: "Ada Acme", avatar_url: null },
      { kind: "user", login: "kim", name: "Kim Lee", avatar_url: null },
      { kind: "user", login: "sam", name: "Sam Park", avatar_url: null },
    ],
    participants_total: 3,
    viewer: { login: "ada", permission: "write", can_merge_as_admin: false, can_enable_auto_merge: true, can_disable_auto_merge: false },
    merge_settings: { auto_merge_allowed: true, squash: true, merge: true, rebase: false },
    auto_merge: null,
    fetched_at: new Date().toISOString(),
    ...overrides,
  };
}

export function mockDetail(id: number, overrides: Partial<PrDetail> = {}): PrDetail {
  return {
    id,
    readiness: "blocked",
    readiness_notes: ["check unit tests is fail"],
    head_sha: "0123456789abcdef",
    base_ref: "main",
    codeowners_path: ".github/CODEOWNERS",
    codeowners_error: null,
    codeowners_required: "true",
    code_owner_review_pending: true,
    mandatory_teams: ["@acme/web"],
    optional_teams: [],
    approved_teams: [],
    candidate_teams: ["@acme/web"],
    attribution_unknown: [],
    requested_teams: ["@acme/web"],
    users_ignored: [],
    explanations: [
      { tone: "bad", text: "Check “unit tests” failed." },
      { tone: "warn", text: "@acme/web still has to approve: it owns changed files." },
    ],
    files: [
      { path: "src/checkout/Steps.tsx", teams: ["@acme/web"] },
      { path: "src/checkout/steps.test.tsx", teams: ["@acme/web"] },
      { path: "README.md", teams: [] },
    ],
    files_truncated: false,
    rules_error: null,
    protection: "Branch protection requires code-owner reviews",
    from_cache: false,
    ...overrides,
  };
}
