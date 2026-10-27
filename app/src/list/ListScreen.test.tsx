import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { Api, Bootstrap, ProjectDto, ReviewTarget } from "../api";
import type { SearchRow } from "./paging";
import { ListScreen } from "./ListScreen";
import { memoryProjectApi } from "../memoryProjects";
import { fakeSettingsApi } from "../settings/fakes";

const boot: Bootstrap = {
  onboarding: { step: "done", completed: [], mapping_skipped: true, slack_skipped: true, finished_at: "2026-10-26T00:00:00Z" },
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
  slack_redirect_uri: "",
  slack_secret_saved: false,
  slack_workspaces: [],
  slack_mentions: "off",
  bump_broadcast: true,
  mapping_count: 0,
  sync: { phase: "idle", message: "Synced", synced_at: "2026-10-26T00:00:00Z", paused: false },
  env_token: false,
  oauth_client_configured: false,
  gh_ready: false,
  gh_detail: "",
  gh_probed: true,
};

function api(projects: ProjectDto[], rows: SearchRow[] = []): Api {
  return {
    bootstrap: () => Promise.resolve(boot),
    keyringStatus: () => Promise.resolve({ state: "ready", message: null, can_retry: false, unsaved: false }),
    keyringRetry: () => Promise.resolve({ state: "ready", message: null, can_retry: false, unsaved: false }),
    saveProviders: () => Promise.resolve(),
    saveOnboarding: () => Promise.resolve(),
    connectGithub: () => Promise.reject(new Error("unused")),
    startDevice: () => Promise.reject(new Error("unused")),
    pollDevice: () => Promise.reject(new Error("unused")),
    saveOrgs: () => Promise.resolve(),
    saveSlack: () => Promise.resolve(),
    slackManifestUrl: () => Promise.resolve(""),
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
    probeGh: () => Promise.resolve({ gh_ready: false, gh_detail: "" }),
    listProjects: () => Promise.resolve([...projects]),
    createProject: vi.fn((name: string) => {
      const project = { id: projects.length + 1, name, color: "#7c5cbf", archived: false, description: "", sort_order: projects.length };
      projects.push(project);
      return Promise.resolve(project);
    }),
    renameProject: () => Promise.resolve(),
    archiveProject: () => Promise.resolve(),
    deleteProject: () => Promise.resolve(),
    ...memoryProjectApi(rows, projects),
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

test("an empty Projects section offers to create one, and Settings lives in the footer", async () => {
  const fake = api([]);
  render(<ListScreen api={fake} boot={boot} onBoot={() => undefined} />);
  const projects = screen.getByRole("region", { name: "Projects" });
  expect(within(projects).getByText(/No projects yet/)).toBeInTheDocument();
  expect(within(projects).queryByRole("button", { name: /Settings/ })).not.toBeInTheDocument();
  expect(within(screen.getByRole("contentinfo")).getByRole("button", { name: /Settings/ })).toBeInTheDocument();

  fireEvent.click(within(projects).getByRole("button", { name: "Create project" }));
  const input = screen.getByRole("textbox", { name: "Project name" });
  fireEvent.change(input, { target: { value: "  " } });
  fireEvent.submit(input);
  expect(await screen.findByText(/cannot be empty/)).toBeInTheDocument();
  fireEvent.change(input, { target: { value: "Checkout" } });
  fireEvent.submit(input);
  await waitFor(() => expect(fake.createProject).toHaveBeenCalledWith("Checkout"));
  expect(await within(projects).findByRole("button", { name: "Checkout" })).toHaveAttribute("aria-current", "page");
  expect(within(projects).queryByText(/No projects yet/)).not.toBeInTheDocument();
});

test("dropping PRs on a sidebar project links them and confirms", async () => {
  const fake = api([{ id: 1, name: "Checkout", color: "#7c5cbf", archived: false, description: "", sort_order: 0 }]);
  fake.linkPrs = vi.fn(() => Promise.resolve(2));
  render(<ListScreen api={fake} boot={boot} onBoot={() => undefined} />);
  const target = await within(screen.getByRole("region", { name: "Projects" })).findByRole("button", { name: "Checkout" });
  const data = new Map([["application/x-pr-nexus-prs", "[4,5]"]]);
  const dataTransfer = { types: [...data.keys()], getData: (type: string) => data.get(type) ?? "", dropEffect: "none" };
  fireEvent.dragOver(target, { dataTransfer });
  fireEvent.drop(target, { dataTransfer });
  await waitFor(() => expect(fake.linkPrs).toHaveBeenCalledWith(1, [4, 5]));
  expect(await screen.findByRole("status")).toHaveTextContent("Added 2 PRs to Checkout");
});

test("a failed first load shows the error with Try again", async () => {
  let fail = true;
  const base = api([]);
  const fake: Api = {
    ...base,
    listPage: (request) => (fail ? Promise.reject(new Error("database is locked")) : base.listPage(request)),
  };
  render(<ListScreen api={fake} boot={boot} onBoot={() => undefined} />);
  expect(await screen.findByRole("alert")).toHaveTextContent(/database is locked/);
  fail = false;
  fireEvent.click(screen.getByRole("button", { name: "Try again" }));
  await waitFor(() => expect(screen.queryByText(/database is locked/)).not.toBeInTheDocument());
});

test("the skipped-mapping hint opens Settings → Mappings", async () => {
  window.localStorage.clear();
  const slack = { ...boot, slack_workspaces: [{ id: 1, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false }] };
  render(<ListScreen api={api([])} boot={slack} onBoot={() => undefined} />);
  fireEvent.click(await screen.findByRole("button", { name: "Map team channels" }));
  expect(await screen.findByRole("tab", { name: "Mappings" })).toHaveAttribute("aria-selected", "true");
  expect(await screen.findByText("No team channels mapped yet.")).toBeInTheDocument();
});

function pr(id: number, title: string): SearchRow {
  return {
    id,
    number: id,
    title,
    url: `https://github.com/acme/widgets/pull/${id}`,
    body: "",
    owner: "acme",
    repo: "acme/widgets",
    author_login: "ada",
    head_ref: `feature/pr-${id}`,
    is_draft: false,
    state: "open",
    checks: "success",
    review_decision: null,
    unresolved_threads: 0,
    total_threads: 0,
    involvement: 1,
    updated_at: "2026-10-26T11:00:00Z",
    created_at: "2026-10-20T11:00:00Z",
    hidden: false,
    project_ids: [],
    readiness: "ready",
    readiness_notes: [],
    slack: null,
  };
}

test("the sidebar's new-project field can take the current selection", async () => {
  const fake = api([{ id: 1, name: "Checkout", color: "#7c5cbf", archived: false, description: "", sort_order: 0 }], [pr(7, "Add passkeys"), pr(8, "Fix tax rounding")]);
  const create = vi.spyOn(fake, "createProjectWithPrs");
  render(<ListScreen api={fake} boot={boot} onBoot={() => undefined} />);
  fireEvent.click(await screen.findByRole("checkbox", { name: /Select Add passkeys/ }));
  const projects = screen.getByRole("region", { name: "Projects" });
  fireEvent.click(within(projects).getByRole("button", { name: "New project" }));
  const withSelection = screen.getByRole("checkbox", { name: "Add 1 selected PR" });
  expect(withSelection).toBeChecked();
  const input = screen.getByRole("textbox", { name: "Project name" });
  fireEvent.change(input, { target: { value: "Launch" } });
  fireEvent.submit(input);
  await waitFor(() => expect(create).toHaveBeenCalledWith("Launch", [7]));
  expect(await screen.findByRole("status")).toHaveTextContent("Created Launch with 1 PR");
  expect(await within(projects).findByRole("button", { name: "Launch" })).toHaveAttribute("aria-current", "page");
});

test("Add to project → New project from selection creates it, confirms, and highlights it", async () => {
  const fake = api([{ id: 1, name: "Checkout", color: "#7c5cbf", archived: false, description: "", sort_order: 0 }], [pr(7, "Add passkeys"), pr(8, "Fix tax rounding")]);
  const create = vi.spyOn(fake, "createProjectWithPrs");
  render(<ListScreen api={fake} boot={boot} onBoot={() => undefined} />);
  fireEvent.click(await screen.findByRole("checkbox", { name: /Select Add passkeys/ }));
  fireEvent.click(screen.getByRole("checkbox", { name: /Select Fix tax rounding/ }));
  fireEvent.click(screen.getByRole("button", { name: "Add to project" }));
  fireEvent.click(screen.getByRole("menuitem", { name: /New project from selection/ }));
  const input = screen.getByRole("textbox", { name: "New project name" });
  fireEvent.change(input, { target: { value: "Launch" } });
  fireEvent.submit(input);
  await waitFor(() => expect(create).toHaveBeenCalledWith("Launch", [7, 8]));
  expect(await screen.findByRole("status")).toHaveTextContent("Created Launch with 2 PRs");
  const entry = await within(screen.getByRole("region", { name: "Projects" })).findByRole("button", { name: "Launch" });
  expect(entry).toHaveAttribute("data-project-id");
  expect(entry.className).toMatch(/ring-violet-400/);
});
