import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import type { Api, Bootstrap, BulkPreview, ContextThread, ProfileDto, ProjectDto, ReviewTarget } from "./api";
import { ProfileReuse } from "./wizard/ProfileReuse";
import { mockBranch, mockDetail, mockOverview } from "./detail/mock";
import "./index.css";
import { ListScreen } from "./list/ListScreen";
import { memoryProjectApi } from "./memoryProjects";
import type { SearchRow } from "./list/paging";
import { variedRows } from "./previewRows";
import { Wizard } from "./wizard/Wizard";
import { recordSeenVersion } from "./changelog/changelog";
import { RELEASES } from "./changelog/data";
import { SettingsScreen } from "./settings/SettingsScreen";
import { BulkDialog } from "./slack/BulkDialog";
import { ReviewDialog } from "./slack/ReviewDialog";
import { FAKE_SLACK_WORKSPACES, fakeMcp, fakeSettingsApi, sampleActivity } from "./settings/fakes";
import { AgentsSettings } from "./settings/AgentsSettings";
import { McpActivity } from "./settings/McpActivity";
import { McpClientConfig } from "./settings/McpClientConfig";

const steps = [
  "choose_vcs",
  "choose_chat",
  "connect_vcs",
  "choose_vcs_scope",
  "connect_chat",
  "chat_mapping",
  "done",
] as const;

// `?sync=mine` shows the header mid-sync of My PRs; `?sync=reviews` shows
// the To review tab while review requests load in the background;
// `?sync=filtered` shows To review with a CI filter answered on GitHub.
const scene = new URLSearchParams(window.location.search).get("sync");
// `?screen=wizard&gh=missing` (or `gh=logged-out`) shows the GitHub step
// when gh cannot be used.
const ghScene = new URLSearchParams(window.location.search).get("gh");
const ghStatus =
  ghScene === "missing"
    ? { gh_ready: false, gh_detail: "GitHub CLI (gh) not found. Install it from cli.github.com, or use a personal access token." }
    : ghScene === "logged-out"
      ? { gh_ready: false, gh_detail: "gh found at /opt/homebrew/bin/gh but not logged in. Run `gh auth login`, then try again." }
      : { gh_ready: true, gh_detail: "gh is logged in" };

const boot: Bootstrap = {
  onboarding: {
    step: "choose_vcs",
    completed: [],
    mapping_skipped: false,
    slack_skipped: false,
    finished_at: new Date().toISOString(),
  },
  resume_step: "choose_vcs",
  steps: [...steps],
  keyring: { state: "ready", message: null, can_retry: false, unsaved: false },
  github_login: "ada",
  github_connected: true,
  tracked_orgs: ["acme"],
  include_personal: false,
  org_choices: ["acme"],
  orgs_needing_sso: [],
  slack_client_id: "",
  slack_redirect_uri: "http://127.0.0.1:47821/mcp/oauth/callback",
  slack_secret_saved: false,
  slack_workspaces: [],
  slack_mentions: "off",
  bump_broadcast: true,
  mapping_count: 0,
  sync: {
    phase: "ok",
    message: "Synced · 4 pull requests",
    synced_at: new Date(Date.now() - 2 * 60 * 1000).toISOString(),
    paused: false,
    revision: 0,
    live: { state: "demo", checked_at: null, detail: null },
  },

  env_token: false,
  oauth_client_configured: false,
  gh_ready: true,
  gh_detail: "gh is logged in",
  gh_probed: true,
};

const projects: ProjectDto[] = [
  { id: 1, name: "Auth", color: "#7c5cbf", archived: false, description: "", sort_order: 0 , jira_links: [] },
  { id: 2, name: "Billing", color: "#3f6f8f", archived: false, description: "", sort_order: 1 , jira_links: [] },
];

function row(partial: Partial<SearchRow> & Pick<SearchRow, "id" | "title" | "number">): SearchRow {
  return {
    url: `https://github.com/acme/widgets/pull/${partial.number}`,
    body: "Demo pull request body for search.",
    owner: "acme",
    repo: "acme/widgets",
    author_login: "ada",
    head_ref: `feature/demo-${partial.number}`,
    is_draft: false,
    state: "open",
    checks: "success",
    review_decision: "approved",
    unresolved_threads: 0,
    total_threads: 2,
    involvement: 1,
    updated_at: new Date().toISOString(),
    created_at: new Date(Date.now() - 3 * 24 * 60 * 60 * 1000).toISOString(),
    hidden: false,
    project_ids: [1],
    readiness: "ready",
    readiness_notes: [],
    conflicts: false,
    reviewers_requested: false,
    pending_reviewers: [],
    slack: null,
    starred: false,
    ...partial,
  };
}

const rows: SearchRow[] = [
  row({
    id: 1,
    number: 142,
    title: "Migrate login to new auth",
    project_ids: [1],
    review_decision: "approved",
    slack: {
      requested_at: new Date(Date.now() - 2 * 24 * 60 * 60 * 1000).toISOString(),
      last_bumped_at: new Date(Date.now() - 5 * 60 * 60 * 1000).toISOString(),
      bump_count: 1,
      workspace: "acme",
      channel_id: "C0123456789",
      channel_name: "team-reviews",
      permalink: "https://acme.slack.com/archives/C0123456789/p1700000000000100",
      root_ts: "1700000000.000100",
      source: "app",
    },
  }),
  row({
    id: 2,
    number: 138,
    title: "Add passkeys support",
    project_ids: [1],
    checks: "success",
    review_decision: "review_required",
    unresolved_threads: 1,
    total_threads: 4,
    reviewers_requested: true,
    pending_reviewers: [
      { kind: "user", login: "dana", name: "Dana Example", avatar_url: null },
      { kind: "user", login: "lee", name: null, avatar_url: null },
      { kind: "team", login: "acme/auth", name: "Auth Team", avatar_url: null },
      { kind: "team", login: "acme/security", name: null, avatar_url: null },
    ],
  }),
  row({
    id: 3,
    number: 135,
    title: "Refresh token rotation",
    project_ids: [1],
    checks: "failure",
    review_decision: "changes_requested",
    readiness: "blocked",
    unresolved_threads: 3,
    total_threads: 3,
  }),
  row({
    id: 4,
    number: 23,
    title: "Fix tax calculation",
    repo: "acme/billing",
    project_ids: [],
    checks: "success",
    review_decision: "review_required",
    readiness: "unknown",
    slack: {
      requested_at: new Date(Date.now() - 5 * 24 * 60 * 60 * 1000).toISOString(),
      last_bumped_at: null,
      bump_count: 0,
      workspace: "acme",
      channel_id: "C0123456789",
      channel_name: "team-reviews",
      permalink: "https://acme.slack.com/archives/C0123456789/p1700000000000200",
      root_ts: "1700000000.000200",
      source: "app",
    },
  }),
];

// `?rows=varied` swaps in one PR per next action (§8.2) for the badges and the "Needs attention" sort.
if (new URLSearchParams(window.location.search).get("rows") === "varied") {
  rows.splice(0, rows.length, ...variedRows(row));
}

/** Demo plan for the bulk Slack dialog: one card per acme channel, or one card for "All to one channel". */
function demoBulkPreview(ids: number[], target: ReviewTarget | null): BulkPreview {
  const prs = rows.filter((item) => ids.includes(item.id)).map((item) => ({ pr_id: item.id, number: item.number, title: item.title, repo: item.repo }));
  const text = (list: typeof prs) =>
    `Hello team\nCould someone please review ${list.length === 1 ? "this pull request" : "these pull requests"}?\n${list.map((pr) => `• \`${pr.title}\`\nhttps://github.com/${pr.repo}/pull/${pr.number}`).join("\n")}\nThanks!`;
  const card = (channel: ReviewTarget, list: typeof prs) => ({
    target: channel,
    messages: [{ action: "request" as const, prs: list, text: text(list), permalink: null }],
    warnings: [],
    signed_in: true,
    single_top_level: false,
    can_switch: false,
    fingerprint: `demo-${channel.channel_id}`,
  });
  if (target) {
    return { cards: [card(target, prs)], skipped: [], pr_warnings: [] };
  }
  const half = Math.ceil(prs.length / 2);
  const cards = [
    card({ workspace: "acme", channel_id: "C0123456789", channel_name: "web-reviews" }, prs.slice(0, half)),
    card({ workspace: "acme", channel_id: "C0987654321", channel_name: "platform" }, prs.slice(half)),
  ].filter((item) => item.messages[0]!.prs.length > 0);
  return { cards, skipped: [], pr_warnings: [] };
}

const previewThreads: ContextThread[] = [
  {
    workspace: "acme",
    channel_id: "C0123456789",
    channel_name: "web-reviews",
    root_ts: "1700000000.000100",
    permalink: "https://acme.slack.com/archives/C0123456789/p1700000000000100",
    status: "active",
    close_reason: null,
    requested_at: "2023-11-14T22:13:20Z",
    last_bumped_at: null,
    bump_count: 0,
    messages: [
      { ts: "1700000000.000100", at: "2023-11-14T22:13:20Z", author: "Ada Acme", user_id: "U0ACME0001", text: "Hello team\nCould someone please review this?\n<https://github.com/acme/web/pull/1|github.com/acme/web/pull/1>", permalink: "https://acme.slack.com/archives/C0123456789/p1700000000000100", is_bot: false },
      { ts: "1700000300.000200", at: "2023-11-14T22:18:20Z", author: "Bob Acme", user_id: "U0ACME0002", text: "On it, <@U0ACME0001>", permalink: null, is_bot: false },
    ],
    has_more: false,
    fetched_at: "2023-11-14T22:20:00Z",
    from_cache: false,
    error: null,
  },
];

const api: Api = {
  bootstrap: () => Promise.resolve(boot),
  keyringStatus: () => Promise.resolve({ state: "ready", message: null, can_retry: false, unsaved: false }),
  keyringRetry: () => Promise.resolve({ state: "ready", message: null, can_retry: false, unsaved: false }),
  saveProviders: () => Promise.resolve(),
  saveOnboarding: () => Promise.resolve(),
  connectGithub: () => Promise.resolve({ login: "ada", organizations: ["acme"], organizations_needing_sso: [] }),
  startDevice: () => Promise.reject(new Error("preview")),
  pollDevice: () => Promise.reject(new Error("preview")),
  saveOrgs: () => Promise.resolve(),
  saveSlack: () => Promise.resolve(),
  slackManifestUrl: () => Promise.resolve("https://api.slack.com/apps?new_app=1"),
  slackConnect: () =>
    Promise.resolve({ id: 9, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false, team_id: "T0ACME0001", enabled: true, priority: 0 }),
  slackCancelConnect: () => Promise.resolve(),
  slackRemoveWorkspace: () => Promise.resolve(),
  slackSetWorkspaceEnabled: () => Promise.resolve([]),
  slackSetWorkspaceOrder: () => Promise.resolve([]),
  reviewTargets: (_prId: number, teams: string[]) =>
    Promise.resolve([{ target: { workspace: "acme", channel_id: "C0123456789", channel_name: "web-reviews" }, teams, source: "mapping" as const }]),
  searchSlackTargets: (query: string) => {
    const workspace = "acme";
    const term = query.replace(/^[#@]/, "").trim().toLowerCase();
    const people = query.startsWith("#") ? [] : [{ id: "U0ACME0002", name: "Grace Hopper" }];
    const channels = query.startsWith("@") ? [] : [{ id: "C0ACME0001", name: "team-reviews" }];
    return Promise.resolve([
      ...channels.map((item) => ({ kind: "channel" as const, item })),
      ...people.map((item) => ({ kind: "member" as const, item })),
    ]
      .filter(({ item }) => item.name.toLowerCase().includes(term))
      .map(({ kind, item }) => ({ kind, target: { workspace, channel_id: item.id, channel_name: item.name } })));
  },
  reviewPreview: (prId: number, target: ReviewTarget) =>
    Promise.resolve({
      target,
      action: "request" as const,
      text: demoBulkPreview([prId], target).cards[0]?.messages[0]?.text ?? "Hi, could you review this?",
      warnings: [],
      pr_warnings: [],
      signed_in: true,
      bump_count: 0,
      permalink: null,
      broadcast: true,
    }),
  bulkPreview: (request) => Promise.resolve(demoBulkPreview(request.pr_ids, request.target)),
  sendBulk: () => Promise.resolve({ batch_id: "bulk-1", results: [], indicators: [] }),
  sendReview: (_prId: number, target: ReviewTarget) =>
    Promise.resolve({
      requested_at: new Date().toISOString(),
      last_bumped_at: null,
      bump_count: 0,
      workspace: target.workspace,
      channel_id: target.channel_id,
      channel_name: target.channel_name,
      permalink: `https://acme.slack.com/archives/${target.channel_id}/p1700000000000100`,
      root_ts: "1700000000.000100",
      source: "app",
    }),
  importMapping: () => Promise.resolve({ rows: 0 }),
  finish: () => Promise.resolve(boot.sync),
  syncNow: () => Promise.resolve(boot.sync),
  syncStatus: () => Promise.resolve(boot.sync),
  fullResync: () => Promise.resolve(boot.sync),
  activateReviews: () => Promise.resolve(boot.sync),
  loadMoreReviews: () => Promise.resolve(boot.sync),
  searchReviews: () => Promise.resolve(boot.sync),
  filterReviews: () => Promise.resolve(boot.sync),
  probeGh: () => Promise.resolve(ghStatus),
  listProjects: () => Promise.resolve(projects.map((item) => ({ ...item }))),
  createProject: (name) => Promise.resolve({ id: 99, name, color: "#7c5cbf", archived: false, description: "", sort_order: 99 , jira_links: [] }),
  renameProject: () => Promise.resolve(),
  archiveProject: () => Promise.resolve(),
  deleteProject: () => Promise.resolve(),
  setProjectDescription: (id: number, description: string) => Promise.resolve({ id, name: "Project", color: "#7c3aed", archived: false, description, sort_order: 0, jira_links: [] }),
  addJiraLink: (_id: number, link: { text?: string }) => Promise.resolve({ id: 1, key: link.text ?? "", url: "", href: null }),
  removeJiraLink: () => Promise.resolve(),
  setJiraLinks: () => Promise.resolve([]),
  prNote: (prId: number) => Promise.resolve({ pr_id: prId, note: "", updated_at: null }),
  setPrNote: (prId: number, note: string) => Promise.resolve({ pr_id: prId, note, updated_at: null }),
  setPrStarred: (_id: number, starred: boolean) => Promise.resolve(starred),
  ...memoryProjectApi(rows, projects, [{ id: 1, project_id: 1, kind: "ticket_key_in_branch", pattern: "AUTH-*", priority: 10 }]),
  loadViewState: () => Promise.resolve(JSON.stringify({ show: { notRequested: true } })),
  saveViewState: () => Promise.resolve(),
  openExternal: () => Promise.resolve(),
  prDetail: (id) => Promise.resolve(mockDetail(id)),
  prOverview: () => Promise.resolve(mockOverview()),
  setAutoMerge: (_id, method) =>
    Promise.resolve(mockOverview(method ? { auto_merge: { method, enabled_by: "ada", enabled_at: null } } : {})),
  mergePr: () => Promise.resolve(mockOverview({ state: "merged", merge_state: "unknown" })),
  closePullRequests: () => Promise.resolve([]),
  setPrDraft: (_id, draft) => Promise.resolve(mockOverview({ is_draft: draft, merge_state: draft ? "draft" : "blocked" })),
  prBranchStatus: () => Promise.resolve(mockBranch({ state: "behind", behind_by: 3 })),
  updatePrBranch: () => Promise.resolve({ overview: mockOverview(), branch: mockBranch() }),
  prSlackContext: () => Promise.resolve(previewThreads),
  prSlackMentions: () => Promise.resolve([]),
  saveSlackMentions: () => Promise.resolve(),
  ...fakeSettingsApi(),
};

if (scene === "mine") {
  boot.sync = { ...boot.sync, phase: "syncing", message: "Checking my PRs… 40/132" };
} else if (scene === "reviews") {
  const loading = { ...boot.sync, reviews: { state: "loading" as const, message: "Checking reviews… 30/100", total: 412, loaded: 2, window: 100 } };
  api.syncStatus = () => Promise.resolve(loading);
  api.activateReviews = () => Promise.resolve(loading);
} else if (scene === "filtered") {
  const reviews = { state: "ready" as const, message: "", total: 412, loaded: 100, window: 100 };
  const filtered = { key: "||status:failure", state: "ready" as const, message: "", total: 57, loaded: 40, window: 100, retry_at: null };
  const answered = { ...boot.sync, revision: boot.sync.revision + 1, reviews: { ...reviews, filtered } };
  api.activateReviews = () => Promise.resolve({ ...boot.sync, reviews });
  api.filterReviews = () => Promise.resolve(answered);
  api.syncStatus = () => Promise.resolve(answered);
  api.loadViewState = (key) =>
    Promise.resolve(JSON.stringify(key === "to_review" ? { show: { notRequested: true }, filters: { ci: ["failing"] } } : { show: { notRequested: true } }));
}

// Two profiles so the account menu, the reuse step and Settings → Profiles have something to show.
// ?profiles=menu opens the account menu; ?screen=profile-reuse shows the first step of "Add a profile";
// ?settings=profiles shows Settings → Profiles.
let previewProfiles: ProfileDto[] = [
  { id: "default", name: "Acme", active: true, githubLogin: "ada", isDefault: true },
  { id: "acme-labs", name: "Acme Labs", active: false, githubLogin: null, isDefault: false },
];
boot.profiles = previewProfiles;
api.listProfiles = () => Promise.resolve(previewProfiles);
api.switchProfile = (id) => {
  previewProfiles = previewProfiles.map((profile) => ({ ...profile, active: profile.id === id }));
  return Promise.resolve(previewProfiles);
};
api.profileReuseOptions = (id) =>
  Promise.resolve(
    id === "default"
      ? { profile: id, githubLogin: "ada", slackTeams: [{ team: "T0ACME0001", name: "Acme" }, { team: "T0ACME0002", name: "Acme Labs" }], templates: 3, projects: 4, sendPolicy: true }
      : { profile: id, githubLogin: null, slackTeams: [], templates: 0, projects: 1, sendPolicy: false },
  );
api.createProfile = (name) => Promise.resolve({ id: "new", name, active: true, githubLogin: null, isDefault: false });
api.renameProfile = (id, name) => {
  previewProfiles = previewProfiles.map((profile) => (profile.id === id ? { ...profile, name } : profile));
  return Promise.resolve(previewProfiles);
};
api.deleteProfile = (id) => {
  previewProfiles = previewProfiles.filter((profile) => profile.id !== id);
  return Promise.resolve(previewProfiles);
};
api.ghAccounts = () => Promise.resolve(["ada", "ada-labs"]);
const profilesScene = new URLSearchParams(window.location.search).get("profiles");
const settingsScene = new URLSearchParams(window.location.search).get("settings");
if (profilesScene === "menu") {
  window.setTimeout(() => document.querySelector<HTMLButtonElement>('button[aria-haspopup="menu"]')?.click(), 600);
}

document.body.style.margin = "0";
document.body.style.background = "#020617";

const screenName = new URLSearchParams(window.location.search).get("screen");
// ?changelog=modal opens "What's new" as after an update from the oldest embedded release;
// ?changelog=tab shows Settings → Changelog.
const changelogPreview = new URLSearchParams(window.location.search).get("changelog");
// ?activity=stats (or log) shows Settings → Agents → Activity on that sub-tab.
const activityPreview = new URLSearchParams(window.location.search).get("activity");
// `?mcp-clients=found` shows the OpenCode tab with an existing opencode.jsonc;
// `?mcp-clients=missing` shows it before the file exists.
const mcpClientsScene = new URLSearchParams(window.location.search).get("mcp-clients");
if (mcpClientsScene) {
  window.localStorage.setItem("pr-nexus.mcp-client-tab", "opencode");
}
const latest = RELEASES[0]?.version ?? "0.1.0";
if (changelogPreview === "modal") {
  recordSeenVersion(RELEASES.length > 1 ? RELEASES[RELEASES.length - 1].version! : "0.0.0");
}
const previewApi: Api = changelogPreview
  ? { ...api, appInfo: () => api.appInfo().then((info) => ({ ...info, version: latest })) }
  : api;
// ?screen=remapped shows the bulk preview after a team moved to another channel:
// a new request in the mapped channel, with a note that the old thread is not bumped.
if (screenName === "remapped") {
  const prs = [1, 2, 3, 4, 5].map((n) => ({ pr_id: n, number: 100 + n, title: `Change ${n}`, repo: "acme/web" }));
  previewApi.slackWorkspaces = () => Promise.resolve([{ id: 1, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false, team_id: "T0ACME0001", enabled: true, priority: 0 }]);
  previewApi.bulkPreview = () =>
    Promise.resolve({
      cards: [
        {
          target: { workspace: "acme", team_id: "T0ACME0001", workspace_name: "Acme", channel_id: "C0ACME0002", channel_name: "new-channel" },
          messages: [{ action: "request", prs, text: "Hi team, could you review these PRs?\n" + prs.map((pr) => `• acme/web#${pr.number} ${pr.title}`).join("\n"), permalink: null }],
          warnings: [],
          signed_in: true,
          single_top_level: false,
          can_switch: false,
          fingerprint: "preview",
          notes: [`Channel changed since the last request: new request in #new-channel for ${prs.map((pr) => `#${pr.number}`).join(", ")}. The old thread in #old-channel is not bumped.`],
        },
      ],
      skipped: [],
      pr_warnings: [],
    });
}
// ?screen=mapping-mismatch shows Settings → Mappings with a row whose saved
// channel name is not the name Slack gives for its channel id.
if (screenName === "mapping-mismatch") {
  const row = (slug: string, name: string, id: string) => ({
    row: { github_org: "acme", team_slug: slug, team_name: "", slack_workspace: "acme", slack_team_id: "T0ACME0001", slack_channel_name: name, slack_channel_id: id },
    source: "user" as const,
    disabled: false,
    has_imported: false,
  });
  const real: Record<string, string> = { C0ACME0001: "web-reviews", C0ACME0002: "new-channel" };
  previewApi.listMappings = () => Promise.resolve([row("api", "old-channel", "C0ACME0002"), row("web", "web-reviews", "C0ACME0001")]);
  previewApi.slackChannelNames = (channels) => Promise.resolve(channels.map((channel) => real[channel.channel_id] ?? null));
  previewApi.slackWorkspaces = () => Promise.resolve([{ id: 1, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false, team_id: "T0ACME0001", enabled: true, priority: 0 }]);
}
// ?screen=slack-scopes shows Settings → Agents after Sign in again, when the
// Slack app still does not list the new scopes (the button is clicked for you).
function slackScopesApi(): Parameters<typeof AgentsSettings>[0]["api"] {
  const stale = [FAKE_SLACK_WORKSPACES[0]!, { ...FAKE_SLACK_WORKSPACES[1]!, scopes_missing: ["reactions:read", "files:read", "canvases:read", "lists:read"] }];
  const fake = fakeMcp({ enabled: true, slack_reads: true }, [], stale);
  fake.slackConnect = () => Promise.resolve(stale[1]);
  return fake;
}
if (screenName === "slack-scopes") {
  window.setTimeout(() => document.querySelector<HTMLButtonElement>('button[aria-label^="Sign in again"]')?.click(), 800);
}
// ?screen=request-review-suggest&case=mapped|search|name-only|empty|manual shows the
// single-PR review dialog for a PR owned by @acme/data (mapped to #acme-data):
// preselected, "#" typed, a mapping without an ID, a search with no match, and a bad ID.
const suggestCase = new URLSearchParams(window.location.search).get("case") ?? "mapped";
const suggestRow = { id: 1, number: 549, title: "Reject budgets below one sampling unit", url: "https://github.com/acme/data-service/pull/549", repo: "acme/data-service", state: "open", is_draft: false, slack: null } as unknown as Parameters<typeof ReviewDialog>[0]["row"];
if (screenName === "request-review-suggest") {
  const acmeData = { workspace: "acme", team_id: "T0ACME0001", workspace_name: "Acme", channel_id: suggestCase === "name-only" ? "" : "C0ACME0001", channel_name: "acme-data" };
  previewApi.prDetail = () => Promise.resolve({ ...mockDetail(1), mandatory_teams: ["@acme/data"], candidate_teams: [] });
  previewApi.reviewTargets = (_id: number, teams: string[]) =>
    Promise.resolve(suggestCase === "empty" || suggestCase === "manual" ? [] : [{ target: acmeData, teams, source: suggestCase === "name-only" ? ("unresolved" as const) : ("mapping" as const) }]);
  previewApi.slackWorkspaces = () => Promise.resolve([{ id: 1, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false, team_id: "T0ACME0001", enabled: true, priority: 0 }]);
  const type = (selector: string, value: string) => {
    const input = document.querySelector<HTMLInputElement>(selector);
    if (!input) return;
    input.focus();
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  };
  window.setTimeout(() => {
    if (suggestCase === "search" || suggestCase === "name-only") type('input[role="combobox"]', "#");
    if (suggestCase === "empty") type('input[role="combobox"]', "#nope");
    if (suggestCase === "manual") {
      [...document.querySelectorAll("button")].find((b) => b.textContent === "Enter an ID")?.click();
      window.setTimeout(() => {
        type('input[placeholder="C0123456789"]', "C12");
        [...document.querySelectorAll("button")].find((b) => b.textContent === "Preview")?.click();
      }, 200);
    }
  }, 600);
}
// ?screen=codeowners shows one PR that already has a request in #team-a-pr-reviews
// and a second CODEOWNERS line with no saved channel. ?case=open focuses that
// line's channel picker. The picker searches the fictional channels below.
const codeownersRow = {
  id: 1,
  number: 549,
  title: "Reject budgets below one sampling unit",
  url: "https://github.com/acme/data-service/pull/549",
  repo: "acme/data-service",
  state: "open",
  is_draft: false,
  slack: null,
} as unknown as Parameters<typeof ReviewDialog>[0]["row"];
if (screenName === "codeowners") {
  const workspace = { workspace: "acme", team_id: "T0ACME0001", workspace_name: "Acme" };
  const known = [
    { ...workspace, channel_id: "C0ACME0001", channel_name: "team-a-pr-reviews" },
    { ...workspace, channel_id: "C0ACME0004", channel_name: "team-b-pr-reviews" },
    { ...workspace, channel_id: "C0ACME0009", channel_name: "ops-reviews" },
  ];
  previewApi.slackWorkspaces = () =>
    Promise.resolve([{ id: 1, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false, team_id: "T0ACME0001", enabled: true, priority: 0 }]);
  previewApi.reviewTargets = () =>
    Promise.resolve([
      { target: known[0]!, teams: ["acme/team-a"], source: "request" as const },
    ]);
  previewApi.searchSlackTargets = (query: string) => {
    const term = query.replace(/^[#@]/, "").trim().toLowerCase();
    return Promise.resolve(
      known
        .filter((target) => target.channel_name.includes(term))
        .map((target) => ({ kind: "channel" as const, target })),
    );
  };
  previewApi.prDetail = () =>
    Promise.resolve({
      ...mockDetail(1),
      owner_groups: [
        { id: "line-a", pattern: "/src/", teams: ["acme/team-a"], users: [], satisfied: false },
        { id: "line-b", pattern: "/billing/", teams: ["acme/team-b"], users: [], satisfied: false },
      ],
    });
  previewApi.bulkPreview = (request) => {
    const skipped = new Set((request.skipped_groups ?? []).map((item) => item.group_id));
    const chosen = new Map((request.group_channels ?? []).map((item) => [item.group_id, item.target]));
    const text = `Hello team\nCould someone please review this pull request?\n• \`${codeownersRow.title}\`\nhttps://github.com/acme/data-service/pull/549\nThanks!`;
    const groups: NonNullable<BulkPreview["groups"]> = [];
    const byChannel = new Map<string, ReviewTarget>();
    const add = (id: string, pattern: string, team: string, fallback: ReviewTarget | null) => {
      if (skipped.has(id)) {
        return;
      }
      const picked = chosen.get(id);
      const target = picked
        ? { ...workspace, channel_id: picked.channel_id, channel_name: picked.channel_name || picked.channel_id }
        : fallback;
      if (!target) {
        groups.push({
          pr_id: 1,
          number: 549,
          group_id: id,
          pattern,
          teams: [team],
          selected: team,
          needs_channel: true,
          hint: `No Slack channel is saved for @${team}.`,
          channel_id: null,
          workspace: null,
        });
        return;
      }
      groups.push({
        pr_id: 1,
        number: 549,
        group_id: id,
        pattern,
        teams: [team],
        selected: team,
        needs_channel: false,
        hint: null,
        channel_id: target.channel_id,
        workspace: "acme",
      });
      byChannel.set(target.channel_id, { ...target, channel_name: known.find((item) => item.channel_id === target.channel_id)?.channel_name || target.channel_name });
    };
    add("line-a", "/src/", "acme/team-a", known[0]!);
    add("line-b", "/billing/", "acme/team-b", null);
    const preview: BulkPreview = {
      cards: [...byChannel.entries()].map(([channelId, target]) => ({
        target,
        messages: [
          {
            action: channelId === "C0ACME0001" ? "bump" : "request",
            prs: [{ pr_id: 1, number: 549, title: codeownersRow.title, repo: codeownersRow.repo }],
            text,
            permalink: channelId === "C0ACME0001" ? "https://acme.slack.com/archives/C0ACME0001/p1700000000000100" : null,
          },
        ],
        warnings: [],
        signed_in: true,
        single_top_level: false,
        can_switch: false,
        fingerprint: channelId,
      })),
      skipped: [],
      pr_warnings: [],
      groups,
    };
    return Promise.resolve(preview);
  };
  window.setTimeout(() => {
    document.querySelector<HTMLInputElement>('section[aria-label^="Choose a channel"] input[role="combobox"]')?.focus();
  }, 400);
}
if (screenName === "notifications") {
  const permission = new URLSearchParams(window.location.search).get("permission") === "denied" ? "denied" : "granted";
  previewApi.notificationPermission = () => Promise.resolve(permission);
  previewApi.requestNotificationPermission = () => Promise.resolve(permission);
}
const root = document.getElementById("root");
if (!root) {
  throw new Error("root element missing");
}

createRoot(root).render(
  <StrictMode>
    {screenName === "wizard" || screenName === "slack" ? (
      <Wizard
        api={api}
        boot={{
          ...boot,
          onboarding: { ...boot.onboarding, finished_at: null },
          resume_step: screenName === "slack" ? "connect_chat" : ghScene ? "connect_vcs" : "choose_vcs",
          ...(ghScene ? { ...ghStatus, gh_probed: true } : {}),
        }}
        onFinished={() => undefined}
      />
    ) : screenName === "codeowners" ? (
      <div className="min-h-screen bg-slate-950 text-slate-200">
        <ReviewDialog api={previewApi} row={codeownersRow} onClose={() => undefined} onSent={() => undefined} onOpen={() => undefined} />
      </div>
    ) : screenName === "request-review-suggest" ? (
      <div className="min-h-screen bg-slate-950 text-slate-200">
        <ReviewDialog api={previewApi} row={suggestRow} onClose={() => undefined} onSent={() => undefined} onOpen={() => undefined} />
      </div>
    ) : screenName === "remapped" ? (
      <div className="min-h-screen bg-slate-950 text-slate-200">
        <BulkDialog api={previewApi} prIds={[1, 2, 3, 4, 5]} onClose={() => undefined} onSent={() => undefined} onOpen={() => undefined} />
      </div>
    ) : screenName === "mapping-mismatch" ? (
      <div className="min-h-screen bg-slate-950 px-6 py-8 text-slate-200">
        <SettingsScreen api={previewApi} boot={boot} onBoot={() => undefined} accounts={null} tab="mappings" onTabChange={() => undefined} onRerunSetup={() => undefined} />
      </div>
    ) : screenName === "profile-reuse" ? (
      <div className="min-h-screen bg-slate-950 px-6 py-8 text-slate-200">
        <ProfileReuse api={api} profiles={previewProfiles} onCreated={() => undefined} onCancel={() => undefined} />
      </div>
    ) : screenName === "notifications" ? (
      <div className="min-h-screen bg-slate-950 px-6 py-8 text-slate-200">
        <SettingsScreen api={previewApi} boot={boot} onBoot={() => undefined} accounts={null} tab="sync" onTabChange={() => undefined} onRerunSetup={() => undefined} />
      </div>
    ) : settingsScene === "profiles" ? (
      <div className="min-h-screen bg-slate-950 px-6 py-8 text-slate-200">
        <SettingsScreen api={previewApi} boot={boot} onBoot={() => undefined} accounts={null} tab="profiles" onTabChange={() => undefined} onRerunSetup={() => undefined} />
      </div>
    ) : screenName === "slack-scopes" ? (
      <div className="min-h-screen bg-slate-950 px-6 py-8 text-slate-200">
        <AgentsSettings api={slackScopesApi()} />
      </div>
    ) : activityPreview === "stats" || activityPreview === "log" ? (
      <div className="min-h-screen max-w-3xl bg-slate-950 px-6 py-8 text-sm text-slate-200">
        <McpActivity api={fakeMcp({}, sampleActivity())} initialView={activityPreview} />
      </div>
    ) : mcpClientsScene ? (
      <div className="min-h-screen max-w-3xl bg-slate-950 px-6 py-8 text-sm text-slate-200">
        <McpClientConfig
          api={fakeMcp({ enabled: true }, [], [], [], mcpClientsScene === "found" ? { opencode: "added" } : {})}
          url="http://127.0.0.1:47800/mcp"
          tokenMasked="pnx_••••••••••••beef"
          tokenVersion="1"
          fixedPort
          onCopy={() => undefined}
        />
      </div>
    ) : changelogPreview === "tab" ? (
      <div className="min-h-screen bg-slate-950 px-6 py-8 text-slate-200">
        <SettingsScreen api={previewApi} boot={boot} onBoot={() => undefined} accounts={null} tab="changelog" onTabChange={() => undefined} onRerunSetup={() => undefined} />
      </div>
    ) : (
      <ListScreen api={previewApi} boot={boot} onBoot={() => undefined} />
    )}
  </StrictMode>,
);
