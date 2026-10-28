import type { Bootstrap, ContextThread, PendingReviewer, PrOverview, ProjectDto, SlackIndicator } from "../api";
import { mockDetail, mockOverview } from "../detail/mock";
import type { SearchRow } from "../list/paging";

/** Made-up data for the README screenshot. Every name is generic. */

const HOUR = 60 * 60 * 1000;
const ago = (hours: number) => new Date(Date.now() - hours * HOUR).toISOString();

export const projects: ProjectDto[] = [
  { id: 1, name: "Checkout redesign", color: "#7c5cbf", archived: false, description: "", sort_order: 0 },
  { id: 2, name: "Passkeys", color: "#3f8f7a", archived: false, description: "", sort_order: 1 },
  { id: 3, name: "Mobile release 4.2", color: "#b0743c", archived: false, description: "", sort_order: 2 },
];

function slack(requestedHoursAgo: number, bumpedHoursAgo: number | null, channel = "web-reviews"): SlackIndicator {
  return {
    requested_at: ago(requestedHoursAgo),
    last_bumped_at: bumpedHoursAgo === null ? null : ago(bumpedHoursAgo),
    bump_count: bumpedHoursAgo === null ? 0 : 1,
    workspace: "acme",
    channel_id: "C0ACME0001",
    channel_name: channel,
    permalink: "https://acme.slack.com/archives/C0ACME0001/p1700000000000100",
    root_ts: "1700000000.000100",
    source: "app",
  };
}

/** Generated avatar: a colored disc with initials, so no real photo is needed. */
function avatar(letters: string, hue: number): string {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl(${hue},55%,58%)"/><stop offset="1" stop-color="hsl(${hue + 30},50%,40%)"/></linearGradient></defs><rect width="64" height="64" fill="url(#g)"/><text x="32" y="41" font-family="Helvetica,Arial,sans-serif" font-size="26" font-weight="600" fill="white" text-anchor="middle">${letters}</text></svg>`;
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

const people: Record<string, PendingReviewer> = {
  sam: { kind: "user", login: "sam", name: "Sam Rivera", avatar_url: avatar("SR", 200) },
  kim: { kind: "user", login: "kim", name: "Kim Lee", avatar_url: avatar("KL", 330) },
  noor: { kind: "user", login: "noor", name: "Noor Haddad", avatar_url: avatar("NH", 140) },
  theo: { kind: "user", login: "theo", name: "Theo Martin", avatar_url: avatar("TM", 30) },
  web: { kind: "team", login: "acme/web", name: "Web", avatar_url: null },
  identity: { kind: "team", login: "acme/identity", name: "Identity", avatar_url: null },
  mobile: { kind: "team", login: "acme/mobile", name: "Mobile", avatar_url: null },
};
const asked = (...logins: string[]) => logins.map((login) => people[login]);

function row(partial: Partial<SearchRow> & Pick<SearchRow, "id" | "number" | "title" | "repo">): SearchRow {
  return {
    url: `https://github.com/${partial.repo}/pull/${partial.number}`,
    body: "",
    owner: "acme",
    author_login: "ada",
    head_ref: `feature/${partial.number}`,
    is_draft: false,
    state: "open",
    checks: "success",
    review_decision: "review_required",
    unresolved_threads: 0,
    total_threads: 0,
    involvement: 1,
    updated_at: ago(partial.id * 3),
    created_at: ago(partial.id * 3 + 48),
    hidden: false,
    project_ids: [],
    readiness: "unknown",
    readiness_notes: [],
    conflicts: false,
    pending_reviewers: [],
    slack: null,
    ...partial,
    reviewers_requested: (partial.pending_reviewers?.length ?? 0) > 0,
  };
}

export const rows: SearchRow[] = [
  row({ id: 1, number: 482, title: "Split the payment step into its own route", repo: "acme/storefront", project_ids: [1], review_decision: "approved", readiness: "ready", total_threads: 4, slack: slack(30, 6) }),
  row({ id: 2, pending_reviewers: asked("kim", "sam", "web"), number: 479, title: "Add address autocomplete to checkout", repo: "acme/storefront", project_ids: [1], unresolved_threads: 2, total_threads: 5, slack: slack(20, null) }),
  row({ id: 3, pending_reviewers: asked("noor"), number: 131, title: "Expose order totals in the cart API", repo: "acme/orders-api", project_ids: [1], checks: "failure", readiness: "blocked", slack: slack(52, 26, "orders-team") }),
  row({ id: 4, number: 488, title: "Show saved cards first", repo: "acme/storefront", project_ids: [1], is_draft: true, review_decision: null }),
  row({ id: 5, number: 77, title: "Register passkeys from account settings", repo: "acme/auth-service", project_ids: [2], review_decision: "approved", readiness: "ready", total_threads: 2, slack: slack(70, 22, "identity") }),
  row({ id: 6, pending_reviewers: asked("kim", "theo", "identity"), number: 79, title: "WebAuthn challenge store in Redis", repo: "acme/auth-service", project_ids: [2], review_decision: "changes_requested", readiness: "blocked", unresolved_threads: 3, total_threads: 3, slack: slack(40, null, "identity") }),
  row({ id: 7, number: 502, title: "Passkey sign-in button on the login page", repo: "acme/storefront", project_ids: [2], review_decision: null }),
  row({ id: 8, pending_reviewers: asked("sam", "kim", "noor", "theo", "mobile"), number: 1204, title: "Bump the minimum OS version", repo: "acme/mobile", project_ids: [3], conflicts: true, readiness: "blocked", slack: slack(90, 40, "mobile") }),
  row({ id: 9, number: 1199, title: "Offline cart sync", repo: "acme/mobile", project_ids: [3], review_decision: "approved", readiness: "ready", total_threads: 6, slack: slack(26, null, "mobile") }),
  row({ id: 10, pending_reviewers: asked("theo"), number: 1210, title: "Crash on empty wishlist", repo: "acme/mobile", project_ids: [3], review_decision: "approved", checks: "pending", slack: slack(8, null, "mobile") }),
  row({ id: 11, pending_reviewers: asked("noor", "sam"), number: 61, title: "Rotate the staging database credentials", repo: "acme/infra", project_ids: [], author_login: "grace", slack: slack(14, null, "platform") }),
  row({ id: 12, number: 58, title: "Pin the base image for CI runners", repo: "acme/infra", project_ids: [], checks: "pending", review_decision: null }),
];

export const boot: Bootstrap = {
  onboarding: { step: "done", completed: [], mapping_skipped: false, slack_skipped: false, finished_at: ago(240) },
  resume_step: "done",
  steps: ["choose_vcs", "choose_chat", "connect_vcs", "choose_vcs_scope", "connect_chat", "chat_mapping", "done"],
  keyring: { state: "ready", message: null, can_retry: false, unsaved: false },
  github_login: "ada",
  github_connected: true,
  tracked_orgs: ["acme"],
  include_personal: false,
  org_choices: ["acme"],
  orgs_needing_sso: [],
  slack_client_id: "",
  slack_redirect_uri: "http://127.0.0.1:47821/mcp/oauth/callback",
  slack_secret_saved: true,
  slack_workspaces: [{ id: 1, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false, team_id: "T0ACME0001", enabled: true, priority: 0 }],
  slack_mentions: "off",
  bump_broadcast: true,
  mapping_count: 12,
  sync: { phase: "ok", message: `Synced · ${rows.length} pull requests`, synced_at: ago(0.05), paused: false, revision: 0, live: { state: "active", checked_at: ago(0.02), detail: null } },
  env_token: false,
  oauth_client_configured: false,
  gh_ready: true,
  gh_detail: "gh is logged in",
  gh_probed: true,
};

/** The PR the detail-panel screenshot opens: #479 above. */
export const detailRowId = 2;

const ada: PendingReviewer = { kind: "user", login: "ada", name: "Ada Park", avatar_url: avatar("AP", 265) };
const grace: PendingReviewer = { kind: "user", login: "grace", name: "Grace Ito", avatar_url: avatar("GI", 95) };

export const overview: PrOverview = mockOverview({
  head_ref: "feature/479",
  issue_comments: 4,
  review_comments: 7,
  checks: [
    { name: "build", state: "passed", url: null, required: true },
    { name: "lint", state: "passed", url: null, required: true },
    { name: "typecheck", state: "passed", url: null, required: true },
    { name: "unit tests", state: "failed", url: "https://ci.example.test/runs/1", required: true },
    { name: "e2e (chrome)", state: "passed", url: null, required: false },
    { name: "preview deploy", state: "pending", url: null, required: false },
  ],
  checks_total: 6,
  reviewers: [
    { ...people.kim, state: "approved" },
    { ...people.sam, state: "changes_requested" },
    { ...people.web, state: "pending" },
  ],
  participants: [ada, people.kim, people.sam, grace, people.noor, people.theo],
  participants_total: 8,
  viewer: { login: "ada", permission: "write", can_merge_as_admin: false, can_enable_auto_merge: true, can_disable_auto_merge: false },
  merge_settings: { auto_merge_allowed: true, squash: true, merge: true, rebase: true },
});

/** #488 above: a draft with no reviews and no Slack request yet. */
export const draftRowId = 4;

export const draftOverview: PrOverview = {
  ...overview,
  is_draft: true,
  merge_state: "draft",
  head_ref: "feature/488",
  review_decision: null,
  issue_comments: 2,
  review_comments: 0,
  approvals: 0,
  required_approvals: 1,
  reviewers: [],
  participants: [ada, grace],
  participants_total: 2,
};

export const detail = mockDetail(detailRowId, {
  files: [
    { path: "src/checkout/AddressField.tsx", teams: ["@acme/web"] },
    { path: "src/checkout/useAutocomplete.ts", teams: ["@acme/web"] },
    { path: "src/checkout/AddressField.test.tsx", teams: ["@acme/web"] },
    { path: "docs/checkout.md", teams: [] },
  ],
});

const slackRoot = "1700000000.000100";
export const slackThreads: ContextThread[] = [
  {
    workspace: "acme",
    channel_id: "C0ACME0001",
    channel_name: "web-reviews",
    root_ts: slackRoot,
    permalink: "https://acme.slack.com/archives/C0ACME0001/p1700000000000100",
    status: "active",
    close_reason: null,
    requested_at: ago(20),
    last_bumped_at: null,
    bump_count: 0,
    messages: [
      { ts: slackRoot, at: ago(20), author: "Ada Park", user_id: null, text: "Could someone from web review #479? Address autocomplete for checkout.", permalink: null, is_bot: false },
      { ts: "1700000100.000200", at: ago(18), author: "Kim Lee", user_id: null, text: "Approved, nice work on the keyboard handling.", permalink: null, is_bot: false },
      { ts: "1700000200.000300", at: ago(6), author: "Sam Rivera", user_id: null, text: "Left a few comments on the debounce, the unit tests need an update.", permalink: null, is_bot: false },
    ],
    has_more: false,
    fetched_at: ago(0.1),
    from_cache: false,
    error: null,
  },
];
