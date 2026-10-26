import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import type { Api, Bootstrap } from "../api";
import { Wizard } from "./Wizard";

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
  keyring_available: false,
  github_login: null,
  github_connected: false,
  tracked_orgs: [],
  include_personal: false,
  org_choices: ["acme"],
  slack_client_id: "",
  slack_redirect_uri: "http://127.0.0.1:47821/mcp/oauth/callback",
  mapping_count: 0,
  sync: { phase: "idle", message: "Not synced yet", synced_at: null, paused: false },
  env_token: false,
  oauth_client_configured: false,
  gh_ready: true,
  gh_detail: "gh is logged in",
};

function api(overrides: Partial<Api> = {}): Api {
  return {
    bootstrap: () => Promise.resolve(boot),
    saveProviders: () => Promise.resolve(),
    saveOnboarding: () => Promise.resolve(),
    connectGithub: () => Promise.resolve({ login: "ada", organizations: ["acme"] }),
    startDevice: () => Promise.reject(new Error("unused")),
    pollDevice: () => Promise.reject(new Error("unused")),
    saveOrgs: () => Promise.resolve(),
    saveSlack: () => Promise.resolve(),
    slackManifestUrl: () => Promise.resolve("https://api.slack.com/apps?new_app=1&manifest_json=%7B%7D"),
    importMapping: () => Promise.resolve({ rows: 1 }),
    finish: () => Promise.resolve(boot.sync),
    syncNow: () => Promise.resolve(boot.sync),
    syncStatus: () => Promise.resolve(boot.sync),
    listRows: () => Promise.resolve([]),
    listProjects: () => Promise.resolve([]),
    openExternal: () => Promise.resolve(),
    ...overrides,
  };
}

test("coming soon providers cannot be selected and back is disabled on step 1", () => {
  render(<Wizard api={api()} boot={boot} onFinished={() => undefined} />);
  expect(screen.getByRole("button", { name: "Back" })).toBeDisabled();
  expect(screen.queryByRole("button", { name: "Skip" })).not.toBeInTheDocument();
  const gitlab = screen.getByRole("radio", { name: /GitLab/ });
  expect(gitlab).toBeDisabled();
  expect(gitlab).toHaveAttribute("aria-checked", "false");
  expect(screen.getByRole("radio", { name: /GitHub/ })).toHaveAttribute("aria-checked", "true");
});

test("skip is on the Slack step and a second workspace cannot be added yet", async () => {
  render(<Wizard api={api()} boot={{ ...boot, resume_step: "connect_chat" }} onFinished={() => undefined} />);
  expect(await screen.findByRole("heading", { name: /Connect Slack/ })).toBeInTheDocument();
  expect(screen.getByText(/Slack sign-in comes in M10/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Skip" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "Add another workspace" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Create your Slack app" })).toBeEnabled();
});

test("next is disabled on orgs until one org or personal repos is chosen", () => {
  render(
    <Wizard api={api()} boot={{ ...boot, resume_step: "choose_vcs_scope", org_choices: ["acme"] }} onFinished={() => undefined} />,
  );
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
  fireEvent.click(screen.getByRole("checkbox", { name: "acme" }));
  expect(screen.getByRole("button", { name: "Next" })).toBeEnabled();
});
