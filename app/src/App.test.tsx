import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import App from "./App";
import type { Api, Bootstrap } from "./api";

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
    importMapping: () => Promise.resolve({ rows: 0 }),
    finish: () => Promise.resolve(boot.sync),
    syncNow: () => Promise.resolve(boot.sync),
    syncStatus: () => Promise.resolve(boot.sync),
    listRows: () => Promise.resolve([]),
    listProjects: () => Promise.resolve([]),
    renameProject: () => Promise.resolve(),
    archiveProject: () => Promise.resolve(),
    deleteProject: () => Promise.resolve(),
    loadViewState: () => Promise.resolve(null),
    saveViewState: () => Promise.resolve(),
    openExternal: () => Promise.resolve(),
  };
}

test("first launch opens the setup wizard on one step", async () => {
  render(<App api={api()} />);
  expect(await screen.findByRole("heading", { name: /Step 1 of 7 · Git provider/ })).toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: /Communication provider/ })).not.toBeInTheDocument();
});
