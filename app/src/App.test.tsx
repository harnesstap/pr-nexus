import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, test } from "vitest";
import App from "./App";
import type { Api, Bootstrap, ReviewTarget } from "./api";
import { memoryProjectApi } from "./memoryProjects";
import { fakeSettingsApi } from "./settings/fakes";

const boot: Bootstrap = {
  onboarding: {
    step: "choose_vcs",
    completed: [],
    mapping_skipped: false,
    slack_skipped: false,
    finished_at: null,
  },
  resume_step: "choose_vcs",
  steps: [
    "choose_vcs",
    "choose_chat",
    "connect_vcs",
    "choose_vcs_scope",
    "connect_chat",
    "chat_mapping",
    "done",
  ],
  keyring_warning: null,
  keyring_available: true,
  github_login: null,
  github_connected: false,
  tracked_orgs: [],
  include_personal: false,
  org_choices: [],
  orgs_needing_sso: [],
  slack_client_id: "",
  slack_redirect_uri: "http://127.0.0.1:47821/mcp/oauth/callback",
  slack_secret_saved: false,
  slack_workspaces: [],
  slack_mentions: "off",
  bump_broadcast: true,
  mapping_count: 0,
  sync: { phase: "idle", message: "Not synced yet", synced_at: null, paused: false },
  env_token: false,
  oauth_client_configured: false,
  gh_ready: false,
  gh_detail: "gh is not available",
};

function api(): Api {
  return {
    bootstrap: () => Promise.resolve(boot),
    saveProviders: () => Promise.resolve(),
    saveOnboarding: () => Promise.resolve(),
    connectGithub: () => Promise.resolve({ login: "ada", organizations: ["acme"], organizations_needing_sso: [] }),
    startDevice: () => Promise.reject(new Error("unused")),
    pollDevice: () => Promise.reject(new Error("unused")),
    saveOrgs: () => Promise.resolve(),
    saveSlack: () => Promise.resolve(),
    slackManifestUrl: () => Promise.resolve("https://api.slack.com/apps?new_app=1"),
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
    listRows: () => Promise.resolve([]),
    listProjects: () => Promise.resolve([]),
    createProject: (name) => Promise.resolve({ id: 99, name, color: "#7c5cbf", archived: false, description: "", sort_order: 99 }),
    renameProject: () => Promise.resolve(),
    archiveProject: () => Promise.resolve(),
    deleteProject: () => Promise.resolve(),
    ...memoryProjectApi([], []),
    loadViewState: () => Promise.resolve(null),
    saveViewState: () => Promise.resolve(),
    openExternal: () => Promise.resolve(),
    prDetail: () => Promise.reject(new Error("unused")),
    prSlackContext: () => Promise.resolve([]),
    prSlackMentions: () => Promise.resolve([]),
    saveSlackMentions: () => Promise.resolve(),
    ...fakeSettingsApi(),
  };
}

test("first launch opens the setup wizard on one step", async () => {
  render(<App api={api()} />);
  expect(await screen.findByRole("heading", { name: /Step 1 of 7 · Git provider/ })).toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: /Communication provider/ })).not.toBeInTheDocument();
});

test("Settings → About → Re-run setup opens the wizard at step 1 and Back returns to the list", async () => {
  const finished = { ...boot, onboarding: { ...boot.onboarding, step: "done" as const, finished_at: "2026-10-26T00:00:00Z" }, resume_step: "done" as const, tracked_orgs: ["acme"] };
  const saved: (string | null)[] = [];
  const fake: Api = {
    ...api(),
    bootstrap: () => Promise.resolve(finished),
    saveOnboarding: (patch) => {
      saved.push(patch.finished_at);
      return Promise.resolve();
    },
  };
  render(<App api={fake} />);
  fireEvent.click(await screen.findByRole("button", { name: /Settings/ }));
  fireEvent.click(await screen.findByRole("tab", { name: "About" }));
  fireEvent.click(await screen.findByRole("button", { name: "Re-run setup" }));
  expect(await screen.findByRole("heading", { name: /Step 1 of 7/ })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
  await waitFor(() => expect(saved.length).toBeGreaterThan(0));
  expect(saved.every((value) => value === "2026-10-26T00:00:00Z")).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "Back to PR-Nexus" }));
  expect(await screen.findByRole("button", { name: "My PRs" })).toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: /Step 1 of 7/ })).not.toBeInTheDocument();
});
