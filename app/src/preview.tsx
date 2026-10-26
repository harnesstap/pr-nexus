import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import type { Api, Bootstrap, ListRow, ProjectDto } from "./api";
import "./index.css";
import { ListScreen } from "./list/ListScreen";
import { Wizard } from "./wizard/Wizard";

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
  importMapping: () => Promise.resolve({ rows: 0 }),
  finish: () => Promise.resolve(boot.sync),
  syncNow: () => Promise.resolve(boot.sync),
  syncStatus: () => Promise.resolve(boot.sync),
  listRows: () => Promise.resolve(rows),
  listProjects: () => Promise.resolve(projects),
  createProject: (name) => Promise.resolve({ id: 99, name, color: "#7c5cbf", archived: false, description: "", sort_order: 99 }),
  renameProject: () => Promise.resolve(),
  archiveProject: () => Promise.resolve(),
  deleteProject: () => Promise.resolve(),
  loadViewState: () => Promise.resolve(JSON.stringify({ show: { notRequested: true } })),
  saveViewState: () => Promise.resolve(),
  openExternal: () => Promise.resolve(),
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
