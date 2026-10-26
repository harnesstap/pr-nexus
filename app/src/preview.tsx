import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import type { Api, Bootstrap, ContextThread, ListRow, PrDetail, ProjectDto, ReviewTarget } from "./api";
import "./index.css";
import { ListScreen } from "./list/ListScreen";
import { memoryProjectApi } from "./memoryProjects";
import { Wizard } from "./wizard/Wizard";
import { fakeSettingsApi } from "./settings/fakes";

const steps = [
  "choose_vcs",
  "choose_chat",
  "connect_vcs",
  "choose_vcs_scope",
  "connect_chat",
  "chat_mapping",
  "done",
] as const;

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
  keyring_warning: null,
  keyring_available: true,
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
  },
  env_token: false,
  oauth_client_configured: false,
  gh_ready: true,
  gh_detail: "gh is logged in",
};

const projects: ProjectDto[] = [
  { id: 1, name: "Auth", color: "#7c5cbf", archived: false, description: "", sort_order: 0 },
  { id: 2, name: "Billing", color: "#3f6f8f", archived: false, description: "", sort_order: 1 },
];

function row(partial: Partial<ListRow> & Pick<ListRow, "id" | "title" | "number">): ListRow {
  return {
    url: `https://github.com/acme/widgets/pull/${partial.number}`,
    body: "Demo pull request body for search.",
    owner: "acme",
    repo: "acme/widgets",
    author_login: "ada",
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
    slack: null,
    ...partial,
  };
}

const rows: ListRow[] = [
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

function previewDetail(id: number): PrDetail {
  return {
    id,
    readiness: "ready",
    readiness_notes: [],
    head_sha: "0123456789abcdef",
    base_ref: "main",
    codeowners_path: ".github/CODEOWNERS",
    codeowners_error: null,
    codeowners_required: "true",
    codeowners_required_reason: "A branch rule requires code-owner review.",
    code_owner_review_pending: true,
    mandatory_teams: ["@acme/platform"],
    optional_teams: [],
    approved_teams: [],
    candidate_teams: ["@acme/platform"],
    attribution_unknown: [],
    requested_teams: ["@acme/platform"],
    users_ignored: [],
    rationale: ["src/app.ts is owned by @acme/platform"],
    files: [{ path: "src/app.ts", teams: ["@acme/platform"] }],
    files_truncated: false,
    rules_error: null,
    protection: "Classic branch protection is only visible to repository admins.",
    from_cache: false,
  };
}

const api: Api = {
  bootstrap: () => Promise.resolve(boot),
  saveProviders: () => Promise.resolve(),
  saveOnboarding: () => Promise.resolve(),
  connectGithub: () => Promise.resolve({ login: "ada", organizations: ["acme"], organizations_needing_sso: [] }),
  startDevice: () => Promise.reject(new Error("preview")),
  pollDevice: () => Promise.reject(new Error("preview")),
  saveOrgs: () => Promise.resolve(),
  saveSlack: () => Promise.resolve(),
  slackManifestUrl: () => Promise.resolve("https://api.slack.com/apps?new_app=1"),
  slackWorkspaces: () => Promise.resolve([]),
  slackConnect: () =>
    Promise.resolve({ id: 9, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false }),
  slackCancelConnect: () => Promise.resolve(),
  slackRemoveWorkspace: () => Promise.resolve(),
  reviewTargets: () => Promise.resolve([]),
  reviewPreview: (_prId: number, target: ReviewTarget) =>
    Promise.resolve({
      target,
      action: "request" as const,
      text: "Hi, could you review this?",
      warnings: [],
      signed_in: true,
      bump_count: 0,
      permalink: null,
      broadcast: true,
    }),
  bulkPreview: () => Promise.resolve({ cards: [], skipped: [] }),
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
  listRows: () => Promise.resolve(rows.map((item) => ({ ...item }))),
  listProjects: () => Promise.resolve(projects.map((item) => ({ ...item }))),
  createProject: (name) => Promise.resolve({ id: 99, name, color: "#7c5cbf", archived: false, description: "", sort_order: 99 }),
  renameProject: () => Promise.resolve(),
  archiveProject: () => Promise.resolve(),
  deleteProject: () => Promise.resolve(),
  ...memoryProjectApi(rows, projects, [{ id: 1, project_id: 1, kind: "ticket_key_in_branch", pattern: "AUTH-*", priority: 10 }]),
  loadViewState: () => Promise.resolve(JSON.stringify({ show: { notRequested: true } })),
  saveViewState: () => Promise.resolve(),
  openExternal: () => Promise.resolve(),
  prDetail: (id) => Promise.resolve(previewDetail(id)),
  prSlackContext: () => Promise.resolve(previewThreads),
  prSlackMentions: () => Promise.resolve([]),
  saveSlackMentions: () => Promise.resolve(),
  ...fakeSettingsApi(),
};

document.body.style.margin = "0";
document.body.style.background = "#020617";

const screenName = new URLSearchParams(window.location.search).get("screen");
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
          resume_step: screenName === "slack" ? "connect_chat" : "choose_vcs",
        }}
        onFinished={() => undefined}
      />
    ) : (
      <ListScreen api={api} boot={boot} onBoot={() => undefined} />
    )}
  </StrictMode>,
);
