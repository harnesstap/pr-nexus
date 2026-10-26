import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { Api, Bootstrap, ProjectDto, ReviewTarget } from "../api";
import { ListScreen } from "./ListScreen";
import { memoryProjectApi } from "../memoryProjects";

const boot: Bootstrap = {
  onboarding: { step: "done", completed: [], mapping_skipped: true, slack_skipped: true, finished_at: "2026-10-26T00:00:00Z" },
  resume_step: "done",
  steps: ["choose_vcs", "choose_chat", "connect_vcs", "choose_vcs_scope", "connect_chat", "chat_mapping", "done"],
  keyring_warning: null,
  keyring_available: true,
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
  mapping_count: 0,
  sync: { phase: "idle", message: "Synced", synced_at: "2026-10-26T00:00:00Z", paused: false },
  env_token: false,
  oauth_client_configured: false,
  gh_ready: false,
  gh_detail: "",
};

function api(projects: ProjectDto[]): Api {
  return {
    bootstrap: () => Promise.resolve(boot),
    saveProviders: () => Promise.resolve(),
    saveOnboarding: () => Promise.resolve(),
    connectGithub: () => Promise.reject(new Error("unused")),
    startDevice: () => Promise.reject(new Error("unused")),
    pollDevice: () => Promise.reject(new Error("unused")),
    saveOrgs: () => Promise.resolve(),
    saveSlack: () => Promise.resolve(),
    slackManifestUrl: () => Promise.resolve(""),
    slackWorkspaces: () => Promise.resolve([]),
    slackConnect: () =>
      Promise.resolve({ id: 9, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true }),
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
    listProjects: () => Promise.resolve([...projects]),
    createProject: vi.fn((name: string) => {
      const project = { id: projects.length + 1, name, color: "#7c5cbf", archived: false, description: "", sort_order: projects.length };
      projects.push(project);
      return Promise.resolve(project);
    }),
    renameProject: () => Promise.resolve(),
    archiveProject: () => Promise.resolve(),
    deleteProject: () => Promise.resolve(),
    ...memoryProjectApi([], []),
    loadViewState: () => Promise.resolve(null),
    saveViewState: () => Promise.resolve(),
    openExternal: () => Promise.resolve(),
    prDetail: () => Promise.reject(new Error("unused")),
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
