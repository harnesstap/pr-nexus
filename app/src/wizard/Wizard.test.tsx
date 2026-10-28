import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { Api, Bootstrap, MappingFile, ReviewTarget } from "../api";
import { Wizard } from "./Wizard";
import { memoryProjectApi } from "../memoryProjects";
import { fakeSettingsApi } from "../settings/fakes";

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
  keyring: { state: "ready", message: null, can_retry: false, unsaved: false },
  github_login: null,
  github_connected: false,
  tracked_orgs: [],
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
  sync: { phase: "idle", message: "Not synced yet", synced_at: null, paused: false, revision: 0 },
  env_token: false,
  oauth_client_configured: false,
  gh_ready: true,
  gh_detail: "gh is logged in",
  gh_probed: true,
};

function api(overrides: Partial<Api> = {}): Api {
  return {
    bootstrap: () => Promise.resolve(boot),
    keyringStatus: () => Promise.resolve({ state: "ready", message: null, can_retry: false, unsaved: false }),
    keyringRetry: () => Promise.resolve({ state: "ready", message: null, can_retry: false, unsaved: false }),
    saveProviders: () => Promise.resolve(),
    saveOnboarding: () => Promise.resolve(),
    connectGithub: () => Promise.resolve({ login: "ada", organizations: ["acme"], organizations_needing_sso: [] }),
    startDevice: () => Promise.reject(new Error("unused")),
    pollDevice: () => Promise.reject(new Error("unused")),
    saveOrgs: () => Promise.resolve(),
    saveSlack: () => Promise.resolve(),
    slackManifestUrl: () => Promise.resolve("https://api.slack.com/apps?new_app=1&manifest_json=%7B%7D"),
    slackConnect: () =>
      Promise.resolve({ id: 9, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false, team_id: "T0ACME0001", enabled: true, priority: 0 }),
    slackCancelConnect: () => Promise.resolve(),
    slackRemoveWorkspace: () => Promise.resolve(),
    slackSetWorkspaceEnabled: () => Promise.resolve([]),
    slackSetWorkspaceOrder: () => Promise.resolve([]),
    reviewTargets: () => Promise.resolve([]),
    searchSlackTargets: () => Promise.resolve([]),
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
    importMapping: () => Promise.resolve({ rows: 1 }),
    finish: () => Promise.resolve(boot.sync),
    syncNow: () => Promise.resolve(boot.sync),
    syncStatus: () => Promise.resolve(boot.sync),
    fullResync: () => Promise.resolve(boot.sync),
    probeGh: () => Promise.resolve({ gh_ready: false, gh_detail: "" }),
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
    prOverview: () => Promise.resolve(null),
    setAutoMerge: () => Promise.reject(new Error("unused")),
    setPrDraft: () => Promise.reject(new Error("unused")),
    mergePr: () => Promise.reject(new Error("unused")),
    prSlackContext: () => Promise.resolve([]),
    prSlackMentions: () => Promise.resolve([]),
    saveSlackMentions: () => Promise.resolve(),
    ...fakeSettingsApi(),
    ...overrides,
  };
}

test("the single-choice provider steps are skipped and recorded once", async () => {
  const providers: string[][] = [];
  render(
    <Wizard
      api={api({
        saveProviders: (vcs, chat) => {
          providers.push([vcs, chat]);
          return Promise.resolve();
        },
      })}
      boot={boot}
      onFinished={() => undefined}
    />,
  );
  expect(screen.getByRole("heading", { name: /Step 1 of 5 · / })).toBeInTheDocument();
  expect(screen.queryByRole("radio", { name: /^GitHub/ })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Back" })).toBeDisabled();
  await waitFor(() => expect(providers).toEqual([["github", "slack"]]));
});

test("a resume step that points at a hidden step opens the first shown step", () => {
  render(<Wizard api={api()} boot={{ ...boot, resume_step: "choose_chat" }} onFinished={() => undefined} />);
  expect(screen.getByRole("heading", { name: /Step 1 of 5 · / })).toBeInTheDocument();
});

test("Connect GitHub stays disabled until a token is pasted, and explains why", () => {
  render(<Wizard api={api()} boot={{ ...boot, gh_ready: false, resume_step: "connect_vcs" }} onFinished={() => undefined} />);
  fireEvent.click(screen.getByRole("radio", { name: /personal access token/ }));
  const connect = screen.getByRole("button", { name: "Connect GitHub" });
  expect(connect).toBeDisabled();
  expect(screen.getByText("Why is this disabled?")).toBeInTheDocument();
  fireEvent.change(screen.getByPlaceholderText("github_pat_…"), { target: { value: "github_pat_x" } });
  expect(connect).toBeEnabled();
  expect(screen.queryByText("Why is this disabled?")).not.toBeInTheDocument();
});

test("the device-flow env hint sits behind the disclosure", () => {
  render(<Wizard api={api()} boot={{ ...boot, oauth_client_configured: false, resume_step: "connect_vcs" }} onFinished={() => undefined} />);
  fireEvent.click(screen.getByRole("radio", { name: /device flow/ }));
  expect(screen.getByRole("button", { name: /Copy & open/ })).toBeDisabled();
  const why = screen.getByText("Why is this disabled?").closest("details");
  expect(why).not.toHaveAttribute("open");
  expect(why).toHaveTextContent(/PR_NEXUS_GITHUB_OAUTH_CLIENT_ID/);
});

test("the Slack step saves a custom redirect, connects in the browser, then allows Next", async () => {
  const saved: string[][] = [];
  let release: (() => void) | null = null;
  render(
    <Wizard
      api={api({
        saveSlack: (clientId, secret, redirect) => {
          saved.push([clientId, secret, redirect]);
          return Promise.resolve();
        },
        slackConnect: () =>
          new Promise((resolve) => {
            release = () => resolve({ id: 3, team_name: "Acme Corp", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false, team_id: "T0ACME0002", enabled: true, priority: 1 });
          }),
      })}
      boot={{ ...boot, resume_step: "connect_chat" }}
      onFinished={() => undefined}
    />,
  );
  expect(await screen.findByRole("heading", { name: /Connect Slack/ })).toBeInTheDocument();
  expect(screen.queryByText(/isn't available in this version/)).not.toBeInTheDocument();
  expect(screen.queryByText(/M\d+/)).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Skip" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Sign in with Slack" })).toBeDisabled();
  fireEvent.change(screen.getByRole("textbox", { name: /Client ID/ }), { target: { value: "111.222" } });
  fireEvent.change(screen.getByRole("textbox", { name: /Redirect URI/ }), {
    target: { value: "http://127.0.0.1:40001/cb" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Sign in with Slack" }));
  expect(await screen.findByRole("button", { name: "Waiting for Slack…" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled();
  expect(screen.getByRole("status", { name: "Slack sign-in status" })).toHaveTextContent("Finish signing in in your browser · 10:00 left.");
  expect(saved).toEqual([["111.222", "", "http://127.0.0.1:40001/cb"]]);
  await waitFor(() => expect(release).not.toBeNull());
  release!();
  expect(await screen.findByText("Acme Corp")).toBeInTheDocument();
  expect(screen.getByText(/Connected to Acme Corp/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Add another workspace" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "Next" })).toBeEnabled();
});

test("a cancelled Slack sign-in shows no error and a failure shows the message", async () => {
  let calls = 0;
  render(
    <Wizard
      api={api({
        slackConnect: () => {
          calls += 1;
          return Promise.reject(
            calls === 1
              ? { kind: "cancelled", message: "Slack sign-in was cancelled." }
              : { kind: "port_in_use", message: "Port 40001 is in use." },
          );
        },
      })}
      boot={{ ...boot, resume_step: "connect_chat", slack_client_id: "111.222" }}
      onFinished={() => undefined}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Sign in with Slack" }));
  await waitFor(() => expect(screen.getByRole("button", { name: "Sign in with Slack" })).toBeEnabled());
  expect(screen.queryByText(/cancelled/)).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Sign in with Slack" }));
  expect(await screen.findByText("Port 40001 is in use.")).toBeInTheDocument();
});

test("a long Slack sign-in counts down, says still waiting, and offers a retry after the timeout", async () => {
  let reject: ((err: unknown) => void) | null = null;
  let calls = 0;
  render(
    <Wizard
      api={api({
        slackConnect: () => {
          calls += 1;
          return new Promise((_, fail) => {
            reject = fail;
          });
        },
      })}
      boot={{ ...boot, resume_step: "connect_chat", slack_client_id: "111.222" }}
      onFinished={() => undefined}
    />,
  );
  vi.useFakeTimers();
  try {
    fireEvent.click(screen.getByRole("button", { name: "Sign in with Slack" }));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(95_000);
    });
    expect(screen.getByRole("status", { name: "Slack sign-in status" })).toHaveTextContent(
      "Still waiting for Slack. Finish signing in in your browser · 8:25 left.",
    );
    expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled();
    await act(async () => {
      reject!({ kind: "timeout", message: "Slack sign-in timed out after 10 minutes. Try again." });
      await vi.advanceTimersByTimeAsync(0);
    });
  } finally {
    vi.useRealTimers();
  }
  const retry = await screen.findByRole("button", { name: "Sign-in timed out, click to retry." });
  expect(screen.queryByRole("button", { name: "Cancel" })).not.toBeInTheDocument();
  fireEvent.click(retry);
  await waitFor(() => expect(calls).toBe(2));
  expect(screen.queryByRole("button", { name: /timed out/ })).not.toBeInTheDocument();
});

test("a connected workspace can be removed", async () => {
  let remaining = [{ id: 4, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false, team_id: "T0ACME0003", enabled: true, priority: 2 }];
  render(
    <Wizard
      api={api({
        slackRemoveWorkspace: () => {
          remaining = [];
          return Promise.resolve();
        },
        slackWorkspaces: () => Promise.resolve(remaining),
      })}
      boot={{ ...boot, resume_step: "connect_chat", slack_workspaces: remaining }}
      onFinished={() => undefined}
    />,
  );
  expect(screen.getByRole("button", { name: "Next" })).toBeEnabled();
  fireEvent.click(screen.getByRole("button", { name: "Remove Acme" }));
  expect(await screen.findByText("None yet.")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
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
  expect(await screen.findByRole("heading", { name: /Step 5 of 5 · Done/ })).toBeInTheDocument();
  expect(screen.getByText(/mapping: skipped/)).toBeInTheDocument();
  expect(finished).toBe(false);
  const open = screen.getByRole("button", { name: "Open PR Nexus" });
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

test("the mapping step loads a CSV from the file dialog and imports it", async () => {
  const csv = "github_org,github_team_slug,github_team_name,slack_channel_name,slack_channel_id\nacme,web,Web,#web,C0123456789";
  let imported = "";
  render(
    <Wizard
      api={api({
        pickMappingFile: () => Promise.resolve({ path: "/home/ada/acme-teams.csv", name: "acme-teams.csv", csv }),
        importMapping: (text) => {
          imported = text;
          return Promise.resolve({ rows: 1 });
        },
      })}
      boot={{
        ...boot,
        resume_step: "chat_mapping",
        slack_workspaces: [{ id: 4, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false, team_id: "T0ACME0004", enabled: true, priority: 3 }],
      }}
      onFinished={() => undefined}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Choose file…" }));
  expect(await screen.findByTitle("/home/ada/acme-teams.csv")).toHaveTextContent("Loaded acme-teams.csv");
  expect(screen.getByRole("textbox", { name: /Mapping CSV/ })).toHaveValue(csv);
  fireEvent.click(screen.getByRole("button", { name: "Import & next" }));
  expect(await screen.findByRole("heading", { name: /Done/ })).toBeInTheDocument();
  expect(imported).toBe(csv);
});

test("the mapping step shows a file read error and an import parse error like a paste", async () => {
  let pick: () => Promise<MappingFile> = () => Promise.reject({ kind: "validation", message: "/tmp/blob.csv is not UTF-8 text" });
  render(
    <Wizard
      api={api({
        pickMappingFile: () => pick(),
        importMapping: () => Promise.reject({ kind: "validation", message: "mapping csv: missing column slack_channel_id" }),
      })}
      boot={{ ...boot, resume_step: "chat_mapping", slack_workspaces: [{ id: 4, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false, team_id: "T0ACME0005", enabled: true, priority: 4 }] }}
      onFinished={() => undefined}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Choose file…" }));
  expect(await screen.findByText("/tmp/blob.csv is not UTF-8 text")).toBeInTheDocument();
  pick = () => Promise.resolve({ path: "/tmp/wrong.csv", name: "wrong.csv", csv: "name,channel\nweb,#web" });
  fireEvent.click(screen.getByRole("button", { name: "Choose file…" }));
  expect(await screen.findByText("Loaded")).toBeInTheDocument();
  expect(screen.queryByText("/tmp/blob.csv is not UTF-8 text")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Import & next" }));
  expect(await screen.findByText("mapping csv: missing column slack_channel_id")).toBeInTheDocument();
  fireEvent.change(screen.getByRole("textbox", { name: /Mapping CSV/ }), { target: { value: "edited" } });
  expect(screen.queryByText("Loaded")).not.toBeInTheDocument();
});

test("the mapping step preselects the only connected workspace for the import", async () => {
  window.localStorage.clear();
  const imported: string[] = [];
  render(
    <Wizard
      api={api({
        importMapping: (_csv, workspace) => {
          imported.push(workspace);
          return Promise.resolve({ rows: 1 });
        },
      })}
      boot={{
        ...boot,
        resume_step: "chat_mapping",
        slack_workspaces: [{ id: 4, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: false, needs_sign_in: true, team_id: "T0ACME0006", enabled: true, priority: 5 }],
      }}
      onFinished={() => undefined}
    />,
  );
  const select = screen.getByRole("combobox", { name: "Default Slack workspace" });
  expect(select).toHaveValue("acme");
  expect(screen.getByRole("option", { name: "Acme (acme.slack.com) · needs sign-in" })).toBeInTheDocument();
  fireEvent.change(screen.getByRole("textbox", { name: /Mapping CSV/ }), {
    target: { value: "github_org,github_team_slug,github_team_name,slack_channel_name,slack_channel_id\nacme,web,Web,#web,C0123456789" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Import & next" }));
  expect(await screen.findByRole("heading", { name: /Done/ })).toBeInTheDocument();
  expect(imported).toEqual(["acme"]);
});

test("the mapping step without a workspace disables the select and points back to Connect Slack", async () => {
  render(<Wizard api={api()} boot={{ ...boot, resume_step: "chat_mapping" }} onFinished={() => undefined} />);
  expect(screen.getByRole("combobox", { name: "Default Slack workspace" })).toBeDisabled();
  fireEvent.change(screen.getByRole("textbox", { name: /Mapping CSV/ }), { target: { value: "github_org,github_team_slug\nacme,web" } });
  expect(screen.getByText(/add a/)).toHaveTextContent("Connect a Slack workspace, or add a slack_workspace column to the CSV.");
  fireEvent.click(screen.getByRole("button", { name: "Back to Connect Slack" }));
  expect(await screen.findByRole("heading", { name: /Connect Slack/ })).toBeInTheDocument();
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

test("a failed first sync offers Retry sync, which runs the sync again", async () => {
  const failed = { phase: "error", message: "Sync failed · 502: GitHub timed out", synced_at: null, paused: false, revision: 0 };
  const synced = { phase: "ok", message: "Synced · 3 pull requests", synced_at: "2026-10-26T10:00:00Z", paused: false, revision: 0 };
  const syncNow = vi.fn().mockResolvedValueOnce(failed).mockResolvedValueOnce(synced);
  render(
    <Wizard
      api={api({ syncNow })}
      boot={{ ...boot, resume_step: "done", github_connected: true, github_login: "ada" }}
      onFinished={() => undefined}
    />,
  );
  expect(await screen.findByText(/First sync: Sync failed · 502: GitHub timed out/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Retry sync" }));
  expect(await screen.findByText(/First sync: Synced · 3 pull requests/)).toBeInTheDocument();
  expect(syncNow).toHaveBeenCalledTimes(2);
  expect(screen.queryByRole("button", { name: "Retry sync" })).not.toBeInTheDocument();
});

test("a successful first sync shows no Retry sync button", async () => {
  render(
    <Wizard
      api={api({ syncNow: () => Promise.resolve({ phase: "ok", message: "Synced · 0 pull requests", synced_at: null, paused: false, revision: 0 }) })}
      boot={{ ...boot, resume_step: "done", github_connected: true, github_login: "ada" }}
      onFinished={() => undefined}
    />,
  );
  expect(await screen.findByText(/First sync: Synced · 0 pull requests/)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Retry sync" })).not.toBeInTheDocument();
});
