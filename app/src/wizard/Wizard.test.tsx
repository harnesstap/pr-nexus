import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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
  orgs_needing_sso: [],
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
    connectGithub: () => Promise.resolve({ login: "ada", organizations: ["acme"], organizations_needing_sso: [] }),
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
    createProject: (name) => Promise.resolve({ id: 99, name, color: "#7c5cbf", archived: false, description: "", sort_order: 99 }),
    renameProject: () => Promise.resolve(),
    archiveProject: () => Promise.resolve(),
    deleteProject: () => Promise.resolve(),
    loadViewState: () => Promise.resolve(null),
    saveViewState: () => Promise.resolve(),
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
  expect(screen.getByText(/Slack sign-in isn't available in this version yet/)).toBeInTheDocument();
  expect(screen.queryByText(/M\d+/)).not.toBeInTheDocument();
  expect(screen.getByText(/unlocks with Slack sign-in/)).toBeInTheDocument();
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

test("skipping the mapping step lands on Done before the list opens", async () => {
  let finished = false;
  render(
    <Wizard
      api={api()}
      boot={{ ...boot, resume_step: "chat_mapping", github_connected: true, github_login: "ada" }}
      onFinished={() => {
        finished = true;
      }}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Skip" }));
  expect(await screen.findByRole("heading", { name: /Step 7 of 7 · Done/ })).toBeInTheDocument();
  expect(screen.getByText(/mapping: skipped/)).toBeInTheDocument();
  expect(finished).toBe(false);
  const open = screen.getByRole("button", { name: "Open PR-Nexus" });
  expect(open).toBeDisabled();
  fireEvent.click(open);
  expect(finished).toBe(false);
  await waitFor(() => expect(open).toBeEnabled(), { timeout: 2000 });
});

test("the mapping step shows a CSV example and Next imports pasted rows", async () => {
  let imported = "";
  render(
    <Wizard
      api={api({
        importMapping: (csv) => {
          imported = csv;
          return Promise.resolve({ rows: 2 });
        },
      })}
      boot={{ ...boot, resume_step: "chat_mapping" }}
      onFinished={() => undefined}
    />,
  );
  expect(screen.getByRole("button", { name: "Next" })).toBeEnabled();
  expect(screen.getAllByText(/C0123ABCD/).length).toBeGreaterThan(0);
  fireEvent.change(screen.getByRole("textbox", { name: /Mapping CSV/ }), {
    target: { value: "github_org,github_team_slug,github_team_name,slack_channel_name,slack_channel_id\nacme,web,Web,#web,C1" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Import & next" }));
  expect(await screen.findByRole("heading", { name: /Done/ })).toBeInTheDocument();
  expect(imported).toContain("acme,web");
});

test("a disabled Next on Connect GitHub says why", () => {
  render(<Wizard api={api()} boot={{ ...boot, resume_step: "connect_vcs" }} onFinished={() => undefined} />);
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
  expect(screen.getByText("Connect GitHub above to continue.")).toBeInTheDocument();
});

test("orgs hidden behind SSO are listed with an authorization hint", () => {
  render(
    <Wizard
      api={api()}
      boot={{ ...boot, resume_step: "choose_vcs_scope", org_choices: ["acme", "acme-sso"], orgs_needing_sso: ["acme-sso"] }}
      onFinished={() => undefined}
    />,
  );
  expect(screen.getByRole("checkbox", { name: /acme-sso/ })).toBeInTheDocument();
  expect(screen.getAllByText("SSO authorization needed")).toHaveLength(1);
  expect(screen.getByText(/at least one org/)).toBeInTheDocument();
});
