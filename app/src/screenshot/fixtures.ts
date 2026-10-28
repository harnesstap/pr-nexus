import type {
  BranchStatus,
  Bootstrap,
  ContextThread,
  PendingReviewer,
  PrOverview,
  ProjectDto,
  ReviewTarget,
  SlackIndicator,
  TargetChoice,
  TargetMatch,
} from "../api";
import { mockBranch, mockDetail, mockOverview } from "../detail/mock";
import type { SearchRow } from "../list/paging";

/** Made-up data for the README screenshot. Every name is generic. */

const HOUR = 60 * 60 * 1000;
const ago = (hours: number) => new Date(Date.now() - hours * HOUR).toISOString();

export const projects: ProjectDto[] = [
  {
    id: 1,
    name: "Checkout redesign",
    color: "#7c5cbf",
    archived: false,
    description: "One-page checkout with saved cards and address autocomplete.\nTarget: end of quarter.",
    sort_order: 0,
    jira_links: [
      { id: 1, key: "ACME-123", url: "https://acme-labs.atlassian.net/browse/ACME-123", href: "https://acme-labs.atlassian.net/browse/ACME-123" },
      { id: 2, key: "ACME-140", url: "", href: "https://acme-labs.atlassian.net/browse/ACME-140" },
    ],
  },
  {
    id: 2,
    name: "Passkeys",
    color: "#3f8f7a",
    archived: false,
    description: "Passwordless sign-in for web and mobile.",
    sort_order: 1,
    jira_links: [{ id: 3, key: "AUTH-42", url: "", href: "https://acme-labs.atlassian.net/browse/AUTH-42" }],
  },
  { id: 3, name: "Mobile release 4.2", color: "#b0743c", archived: false, description: "", sort_order: 2, jira_links: [] },
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
    starred: false,
    ...partial,
    reviewers_requested: (partial.pending_reviewers?.length ?? 0) > 0,
  };
}

export const rows: SearchRow[] = [
  row({
    id: 1,
    number: 482,
    title: "feat(checkout): split the payment step into its own route and lazy-load the card form",
    repo: "acme/storefront",
    project_ids: [1],
    review_decision: "approved",
    readiness: "ready",
    total_threads: 4,
    slack: slack(30, 6),
  }),
  row({
    id: 2,
    pending_reviewers: asked("kim", "sam", "web"),
    number: 479,
    title: "Add address autocomplete to checkout",
    repo: "acme/storefront",
    project_ids: [1],
    unresolved_threads: 2,
    total_threads: 5,
    slack: slack(20, null),
    starred: true,
  }),
  row({
    id: 3,
    pending_reviewers: asked("noor"),
    number: 131,
    title: "fix: expose order totals in the cart API so the mobile client stops recomputing tax",
    repo: "acme/orders-api",
    project_ids: [1],
    checks: "failure",
    readiness: "blocked",
    slack: slack(52, 26, "orders-team"),
  }),
  row({ id: 4, number: 488, title: "Show saved cards first", repo: "acme/storefront", project_ids: [1], is_draft: true, review_decision: null }),
  row({
    id: 5,
    number: 77,
    title: "Register passkeys from account settings",
    repo: "acme/auth-service",
    project_ids: [2],
    review_decision: "approved",
    readiness: "ready",
    total_threads: 2,
    slack: slack(70, 22, "identity"),
  }),
  row({
    id: 6,
    pending_reviewers: asked("kim", "theo", "identity"),
    number: 79,
    title: "WebAuthn challenge store in Redis",
    repo: "acme/auth-service",
    project_ids: [2],
    review_decision: "changes_requested",
    readiness: "blocked",
    unresolved_threads: 3,
    total_threads: 3,
    slack: slack(40, null, "identity"),
  }),
  row({
    id: 7,
    number: 502,
    title: "feat: stop bypassing the auth gate from the admin proxy",
    repo: "acme-labs/identity-gateway-service",
    project_ids: [2],
    review_decision: null,
  }),
  row({
    id: 8,
    pending_reviewers: asked("sam", "kim", "noor", "theo", "mobile"),
    number: 1204,
    title: "Bump the minimum OS version",
    repo: "acme/mobile",
    project_ids: [3],
    conflicts: true,
    readiness: "blocked",
    slack: slack(90, 40, "mobile"),
  }),
  row({
    id: 9,
    number: 1199,
    title: "Offline cart sync",
    repo: "acme/mobile",
    project_ids: [3],
    review_decision: "approved",
    readiness: "ready",
    total_threads: 6,
    slack: slack(26, null, "mobile"),
    starred: true,
  }),
  row({
    id: 10,
    pending_reviewers: asked("theo"),
    number: 1210,
    title: "Crash on empty wishlist",
    repo: "acme/mobile",
    project_ids: [3],
    review_decision: "approved",
    checks: "pending",
    slack: slack(8, null, "mobile"),
  }),
  row({
    id: 11,
    pending_reviewers: asked("noor", "sam"),
    number: 61,
    title: "chore(infra): rotate the staging database credentials and move them to the secrets manager",
    repo: "acme/infra",
    project_ids: [],
    author_login: "grace",
    slack: slack(14, null, "platform"),
  }),
  row({
    id: 12,
    number: 58,
    title: "Pin the base image for CI runners",
    repo: "acme/infra",
    project_ids: [],
    checks: "pending",
    review_decision: null,
    starred: true,
  }),
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
  slack_workspaces: [
    { id: 1, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false, team_id: "T0ACME0001", enabled: true, priority: 0 },
    {
      id: 2,
      team_name: "Acme Labs",
      domain: "acme-labs",
      user_name: "ada",
      signed_in: true,
      needs_sign_in: false,
      team_id: "T0ACME0002",
      enabled: true,
      priority: 1,
    },
  ],
  slack_mentions: "off",
  bump_broadcast: true,
  mapping_count: 12,
  sync: {
    phase: "ok",
    message: `Synced · ${rows.length} pull requests`,
    synced_at: ago(0.05),
    paused: false,
    revision: 0,
    live: { state: "active", checked_at: ago(0.02), detail: null },
  },
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
  author: ada,
  assignees: [ada],
  commenters: [people.kim, grace, people.noor, people.theo],
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
  issue_comments: 3,
  review_comments: 1,
  approvals: 1,
  required_approvals: 2,
  author: grace,
  reviewers: [
    { ...people.kim, state: "approved" },
    { ...people.web, state: "pending" },
  ],
  assignees: [ada],
  commenters: [people.noor, { kind: "bot", login: "ci-bot", name: null, avatar_url: avatar("CI", 180) }, people.kim, ada],
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
      {
        ts: slackRoot,
        at: ago(20),
        author: "Ada Park",
        user_id: null,
        text: "Could someone from web review #479? Address autocomplete for checkout.",
        permalink: null,
        is_bot: false,
      },
      {
        ts: "1700000100.000200",
        at: ago(18),
        author: "Kim Lee",
        user_id: null,
        text: "Approved, nice work on the keyboard handling.",
        permalink: null,
        is_bot: false,
      },
      {
        ts: "1700000200.000300",
        at: ago(6),
        author: "Sam Rivera",
        user_id: null,
        text: "Left a few comments on the debounce, the unit tests need an update.",
        permalink: null,
        is_bot: false,
      },
    ],
    has_more: false,
    fetched_at: ago(0.1),
    from_cache: false,
    error: null,
  },
];

/** #482 above: approved, green and clean, so it can merge now. */
export const mergeableRowId = 1;

export const mergeableOverview: PrOverview = {
  ...overview,
  head_ref: "feature/482",
  merge_state: "clean",
  review_decision: "approved",
  approvals: 2,
  required_approvals: 2,
  checks: overview.checks.map((check) => ({ ...check, state: "passed" })),
  reviewers: [
    { ...people.kim, state: "approved" },
    { ...people.sam, state: "approved" },
  ],
};

const webReviews: ReviewTarget = { workspace: "acme", team_id: "T0ACME0001", workspace_name: "Acme", channel_id: "C0ACME0001", channel_name: "web-reviews" };

/** The code-owner channel the review dialog suggests. */
export const reviewChoices: TargetChoice[] = [{ target: webReviews, teams: ["@acme/web"], source: "mapping" }];

/** The request text as the backend renders it: the title as inline code, the bare URL on its own line. */
export function reviewText(title: string, url: string): string {
  return `Hello team\nCould someone please review \`${title.replaceAll("`", "'")}\`? (Ada Park)\n${url}\nThanks!\n`;
}

const labs = (target: ReviewTarget): ReviewTarget => ({
  ...target,
  workspace: "acme-labs",
  team_id: "T0ACME0002",
  workspace_name: "Acme Labs",
  channel_id: `${target.channel_id}L`,
});
const directory: TargetMatch[] = [
  { kind: "channel", target: webReviews },
  { kind: "channel", target: { ...webReviews, channel_id: "C0ACME0005", channel_name: "web" } },
  { kind: "channel", target: { ...webReviews, channel_id: "C0ACME0006", channel_name: "frontend-web" } },
  { kind: "member", target: { ...webReviews, channel_id: "U0ACME0007", channel_name: "Webster Lane" } },
];

/** Search across both workspaces, as the backend merges them. */
export function searchTargets(query: string): TargetMatch[] {
  const trimmed = query.trim();
  const term = trimmed.replace(/^[#@]/, "").toLowerCase();
  const kinds = trimmed.startsWith("#") ? ["channel"] : trimmed.startsWith("@") ? ["member"] : ["channel", "member"];
  const hits = directory.filter((item) => kinds.includes(item.kind) && item.target.channel_name.toLowerCase().includes(term));
  return [...hits, ...hits.map((item) => ({ ...item, target: labs(item.target) }))];
}

/** The private notes the detail panel shows, by PR id. */
export const notes: Record<number, string> = {
  [detailRowId]: "Waiting on Sam's debounce comments. Ship behind the checkout flag; QA sign-off in ACME-140.",
};

export const jiraBaseUrl = "https://acme-labs.atlassian.net";

const rootOnly = (text: string, hoursAgo: number): ContextThread["messages"] => [{ ...slackThreads[0].messages[0], at: ago(hoursAgo), text }];

/** Slack card states the screenshot script picks through `window.__slackVariant`. */
export const slackVariants: Record<string, ContextThread[]> = {
  replies: slackThreads,
  requested: [
    { ...slackThreads[0], requested_at: ago(0.3), messages: rootOnly("Could someone from web review #479 `Add address autocomplete to checkout`?", 0.3) },
  ],
  bumpable: [
    {
      ...slackThreads[0],
      requested_at: ago(26),
      last_bumped_at: ago(20),
      bump_count: 1,
      messages: rootOnly(
        "Could someone from web review #479 `Add address autocomplete to checkout`? It adds the autocomplete hook, keyboard handling for the suggestion list, and tests for the debounce. The address field also gains a loading state and an error message when the lookup fails.",
        26,
      ),
    },
  ],
  "long-channel": [{ ...slackThreads[0], channel_name: "acme-labs-web-platform-checkout-and-payments-reviews", requested_at: ago(1) }],
};

/** One open PR per Branch card state, by row id; `overview` patches that row's overview. */
export const branchFixtures: Record<number, { overview: Partial<PrOverview>; branch: BranchStatus }> = {
  1: { overview: {}, branch: mockBranch({ head_ref: "feature/482", conflicts_url: "https://github.com/acme/storefront/pull/482/conflicts" }) },
  2: {
    overview: {},
    branch: mockBranch({
      state: "behind",
      behind_by: 3,
      other_authors: ["grace"],
      head_ref: "feature/479",
      conflicts_url: "https://github.com/acme/storefront/pull/479/conflicts",
    }),
  },
  5: {
    overview: { merge_state: "behind" },
    branch: mockBranch({
      state: "update_required",
      behind_by: 5,
      head_ref: "feature/77",
      conflicts_url: "https://github.com/acme/auth-service/pull/77/conflicts",
    }),
  },
  8: {
    overview: { mergeable: "conflicting", merge_state: "dirty" },
    branch: mockBranch({ state: "conflicts", behind_by: null, head_ref: "feature/1204", conflicts_url: "https://github.com/acme/mobile/pull/1204/conflicts" }),
  },
  10: {
    overview: { mergeable: "unknown", merge_state: "unknown" },
    branch: mockBranch({ state: "checking", behind_by: null, head_ref: "feature/1210", conflicts_url: "https://github.com/acme/mobile/pull/1210/conflicts" }),
  },
};
