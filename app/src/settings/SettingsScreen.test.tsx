import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, test } from "vitest";
import type { Api, Bootstrap, MappingEntry, Preferences, SlackWorkspace } from "../api";
import { DEFAULT_PREFERENCES, fakeSettingsApi } from "./fakes";
import { SettingsScreen } from "./SettingsScreen";
import type { SettingsTab } from "./tabs";
import { useState } from "react";

const boot = {
  bump_broadcast: true,
  mapping_count: 0,
  onboarding: { mapping_skipped: true },
} as unknown as Bootstrap;

function Harness({
  api,
  start = "accounts",
  onBoot = () => undefined,
  onRerun = () => undefined,
}: {
  api: Api;
  start?: SettingsTab;
  onBoot?: (boot: Bootstrap) => void;
  onRerun?: () => void;
}) {
  const [tab, setTab] = useState<SettingsTab>(start);
  return <SettingsScreen api={api} boot={boot} onBoot={onBoot} accounts={<p>Accounts body</p>} tab={tab} onTabChange={setTab} onRerunSetup={onRerun} />;
}

beforeEach(() => window.localStorage.clear());

const acme: SlackWorkspace = {
  id: 1,
  team_name: "Acme",
  domain: "acme",
  user_name: "ada",
  signed_in: true,
  needs_sign_in: false,
  team_id: "T0ACME0001",
  enabled: true,
  priority: 0,
};
const widgets: SlackWorkspace = {
  id: 2,
  team_name: "Widgets",
  domain: "widgets",
  user_name: "bob",
  signed_in: false,
  needs_sign_in: true,
  team_id: "T0ACME0002",
  enabled: true,
  priority: 1,
};

const entry = (partial: Partial<MappingEntry> & { slug: string }): MappingEntry => ({
  row: {
    github_org: "acme",
    team_slug: partial.slug,
    team_name: "",
    slack_workspace: "acme",
    slack_channel_name: `${partial.slug}-reviews`,
    slack_channel_id: "C0123456789",
  },
  source: partial.source ?? "imported",
  disabled: partial.disabled ?? false,
  has_imported: partial.has_imported ?? true,
});

test("tabs follow the WAI-ARIA pattern: one tab stop, arrows and Home/End move focus", () => {
  render(<Harness api={fakeSettingsApi() as Api} />);
  const accounts = screen.getByRole("tab", { name: "Accounts" });
  expect(accounts).toHaveAttribute("aria-selected", "true");
  expect(screen.getByRole("tab", { name: "Mappings" })).toHaveAttribute("tabindex", "-1");
  expect(screen.getByRole("tabpanel")).toHaveTextContent("Accounts body");
  fireEvent.keyDown(accounts, { key: "ArrowLeft" });
  expect(screen.getByRole("tab", { name: "About" })).toHaveAttribute("aria-selected", "true");
  expect(screen.getByRole("tab", { name: "About" })).toHaveFocus();
  fireEvent.keyDown(screen.getByRole("tab", { name: "About" }), { key: "Home" });
  expect(accounts).toHaveFocus();
  fireEvent.keyDown(accounts, { key: "ArrowRight" });
  expect(screen.getByRole("tab", { name: "Mappings" })).toHaveAttribute("aria-selected", "true");
});

test("Review saves the bump broadcast setting and passes it up to the app", async () => {
  const saved: Preferences[] = [];
  const booted: Bootstrap[] = [];
  const api = fakeSettingsApi({
    savePreferences: (prefs) => {
      saved.push(prefs);
      return Promise.resolve(prefs);
    },
  }) as Api;
  render(<Harness api={api} start="review" onBoot={(next) => booted.push(next)} />);
  const toggle = await screen.findByRole("switch", { name: "Also post bumps to the channel" });
  expect(toggle).toBeChecked();
  expect(screen.queryByRole("button", { name: "Save" })).not.toBeInTheDocument();
  fireEvent.click(toggle);
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByText("Saved")).toBeInTheDocument();
  expect(saved).toEqual([{ ...DEFAULT_PREFERENCES, bump_broadcast: false }]);
  expect(booted.map((next) => next.bump_broadcast)).toEqual([false]);
});

test("Sync edits intervals in minutes and shows an error from the backend", async () => {
  const saved: Preferences[] = [];
  const api = fakeSettingsApi({
    savePreferences: (prefs) => {
      saved.push(prefs);
      return Promise.reject(new Error("sync.focused_interval_s must be between 60 and 3600"));
    },
  }) as Api;
  render(<Harness api={api} start="sync" />);
  const focused = await screen.findByRole("spinbutton", { name: /window focused/ });
  expect(focused).toHaveValue(3);
  fireEvent.change(focused, { target: { value: "45" } });
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(/between 60 and 3600/);
  expect(saved[0]?.focused_interval_s).toBe(2700);
});

test("Mappings: empty state names the skipped setup step", async () => {
  render(<Harness api={fakeSettingsApi() as Api} start="mappings" />);
  expect(await screen.findByText("No team channels mapped yet.")).toBeInTheDocument();
  expect(screen.getByText(/You skipped this step in setup/)).toBeInTheDocument();
});

test("Mappings: badges, filter, inline edit and reset to imported", async () => {
  let entries = [
    entry({ slug: "web" }),
    entry({ slug: "api", source: "user", has_imported: true }),
    entry({ slug: "ops", source: "user", has_imported: false, disabled: true }),
  ];
  const calls: string[] = [];
  const counts: number[] = [];
  const api = fakeSettingsApi({
    listMappings: () => Promise.resolve(entries),
    saveMapping: (row, disabled) => {
      calls.push(`save ${row.team_slug} #${row.slack_channel_name} ${disabled}`);
      entries = entries.map((item) => (item.row.team_slug === row.team_slug ? { ...item, row, disabled, source: "user" } : item));
      return Promise.resolve();
    },
    resetMapping: (org, slug) => {
      calls.push(`reset ${org}/${slug}`);
      return Promise.resolve(true);
    },
  }) as Api;
  render(<Harness api={api} start="mappings" onBoot={(next) => counts.push(next.mapping_count)} />);
  const table = await screen.findByRole("table");
  expect(within(table).getAllByText("Edited")).toHaveLength(2);
  expect(within(table).getByText("No channel (disabled)")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Reset acme/api to imported" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Reset acme/ops to imported" })).not.toBeInTheDocument();

  fireEvent.change(screen.getByRole("searchbox", { name: "Filter mappings" }), { target: { value: "#web" } });
  expect(within(table).queryByText("acme/api")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Edit acme/web" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Channel name" }), { target: { value: "web-team" } });
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByText("Saved acme/web.")).toBeInTheDocument();
  expect(calls).toEqual(["save web #web-team false"]);
  expect(counts).toEqual([3]);

  fireEvent.change(screen.getByRole("searchbox", { name: "Filter mappings" }), { target: { value: "" } });
  fireEvent.click(screen.getByRole("button", { name: "Reset acme/api to imported" }));
  await waitFor(() => expect(calls).toContain("reset acme/api"));
});

test("Mappings: a failed save keeps the row open with the error", async () => {
  const api = fakeSettingsApi({
    listMappings: () => Promise.resolve([entry({ slug: "web" })]),
    saveMapping: () => Promise.reject(new Error("slack_channel_id must look like C0123456789")),
  }) as Api;
  render(<Harness api={api} start="mappings" />);
  fireEvent.click(await screen.findByRole("button", { name: "Edit acme/web" }));
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(/must look like/);
  expect(screen.getByRole("textbox", { name: "Channel name" })).toBeInTheDocument();
});

test("Import: Apply stays disabled until a preview of the same CSV, and the diff lists changes", async () => {
  const applied: string[] = [];
  const api = fakeSettingsApi({
    previewMappingImport: () =>
      Promise.resolve({
        added: [entry({ slug: "web" }).row],
        changed: [{ row: entry({ slug: "api" }).row, changes: [{ field: "slack_channel_name", before: "old", after: "api-reviews" }] }],
        unchanged: 2,
        user_shadowed: [entry({ slug: "api" }).row],
      }),
    applyMappingImport: (csv, workspace, mode) => {
      applied.push(`${mode} ${workspace} ${csv.length}`);
      return Promise.resolve({ rows: 4 });
    },
    slackWorkspaces: () => Promise.resolve([acme]),
  }) as Api;
  render(<Harness api={api} start="mappings" />);
  const csv = await screen.findByRole("textbox", { name: "Mapping CSV" });
  await screen.findByRole("option", { name: "Acme (acme.slack.com)" });
  const apply = screen.getByRole("button", { name: "Apply import" });
  expect(screen.getByRole("button", { name: "Preview changes" })).toBeDisabled();
  fireEvent.change(csv, { target: { value: "github_org,github_team_slug\nacme,web" } });
  fireEvent.click(screen.getByRole("button", { name: "Preview changes" }));
  expect(await screen.findByText("1 new · 1 changed · 2 unchanged")).toBeInTheDocument();
  expect(screen.getByText(/acme\/api: channel old → api-reviews/)).toBeInTheDocument();
  expect(screen.getByText(/Your edits still win for acme\/api/)).toBeInTheDocument();
  expect(apply).toBeEnabled();
  fireEvent.change(csv, { target: { value: "github_org,github_team_slug\nacme,ops" } });
  expect(apply).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Preview changes" }));
  await waitFor(() => expect(apply).toBeEnabled());
  fireEvent.click(screen.getByRole("radio", { name: "Replace the imported rows" }));
  expect(apply).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Preview changes" }));
  await waitFor(() => expect(apply).toBeEnabled());
  fireEvent.click(apply);
  expect(await screen.findByText("Imported 4 rows.")).toBeInTheDocument();
  expect(applied).toEqual(["replace acme 36"]);
  expect(window.localStorage.getItem("pr-nexus.mapping-workspace")).toBe("acme");
});

test("Import: Choose file loads the picked CSV for the same preview", async () => {
  const previewed: string[] = [];
  const api = fakeSettingsApi({
    pickMappingFile: () => Promise.resolve({ path: "/home/ada/teams.csv", name: "teams.csv", csv: "github_org,github_team_slug\nacme,web" }),
    previewMappingImport: (csv) => {
      previewed.push(csv);
      return Promise.reject({ kind: "validation", message: "mapping csv: missing column slack_channel_id" });
    },
    slackWorkspaces: () => Promise.resolve([acme]),
  }) as Api;
  render(<Harness api={api} start="mappings" />);
  await screen.findByRole("option", { name: "Acme (acme.slack.com)" });
  fireEvent.click(screen.getByRole("button", { name: "Choose file…" }));
  expect(await screen.findByText((_, element) => element?.className === "truncate" && element.textContent === "Loaded teams.csv")).toHaveAccessibleDescription(
    "/home/ada/teams.csv",
  );
  expect(screen.getByRole("textbox", { name: "Mapping CSV" })).toHaveValue("github_org,github_team_slug\nacme,web");
  fireEvent.click(screen.getByRole("button", { name: "Preview changes" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("mapping csv: missing column slack_channel_id");
  expect(previewed).toEqual(["github_org,github_team_slug\nacme,web"]);
});

test("Import: with no workspace connected the select is disabled and links to Accounts", async () => {
  render(<Harness api={fakeSettingsApi() as Api} start="mappings" />);
  const select = await screen.findByRole("combobox", { name: "Default Slack workspace" });
  expect(select).toBeDisabled();
  expect(select).toHaveDisplayValue("No Slack workspace connected");
  const csv = screen.getByRole("textbox", { name: "Mapping CSV" });
  const preview = screen.getByRole("button", { name: "Preview changes" });
  fireEvent.change(csv, { target: { value: "github_org,github_team_slug\nacme,web" } });
  expect(preview).toBeDisabled();
  expect(screen.getByText(/add a/)).toHaveTextContent("Connect a Slack workspace, or add a slack_workspace column to the CSV.");
  fireEvent.change(csv, { target: { value: "github_org,github_team_slug,slack_workspace\nacme,web,acme" } });
  expect(preview).toBeEnabled();
  fireEvent.click(screen.getByRole("button", { name: "Connect Slack in Accounts" }));
  expect(screen.getByRole("tab", { name: "Accounts" })).toHaveAttribute("aria-selected", "true");
});

test("Import: several workspaces preselect the last used one and list expired sign-ins", async () => {
  window.localStorage.setItem("pr-nexus.mapping-workspace", "widgets");
  const previewed: string[] = [];
  const api = fakeSettingsApi({
    slackWorkspaces: () => Promise.resolve([acme, widgets]),
    previewMappingImport: (_csv, workspace) => {
      previewed.push(workspace);
      return Promise.resolve({ added: [], changed: [], unchanged: 0, user_shadowed: [] });
    },
  }) as Api;
  render(<Harness api={api} start="mappings" />);
  await screen.findByRole("option", { name: "Widgets (widgets.slack.com) · needs sign-in" });
  const select = screen.getByRole("combobox", { name: "Default Slack workspace" });
  expect(select).toBeEnabled();
  expect(select).toHaveValue("widgets");
  fireEvent.change(select, { target: { value: "acme" } });
  fireEvent.change(screen.getByRole("textbox", { name: "Mapping CSV" }), { target: { value: "github_org,github_team_slug\nacme,web" } });
  fireEvent.click(screen.getByRole("button", { name: "Preview changes" }));
  await waitFor(() => expect(previewed).toEqual(["acme"]));
});

test("Import: several workspaces without a stored one preselect the first", async () => {
  render(<Harness api={fakeSettingsApi({ slackWorkspaces: () => Promise.resolve([acme, widgets]) }) as Api} start="mappings" />);
  await screen.findByRole("option", { name: "Acme (acme.slack.com)" });
  expect(screen.getByRole("combobox", { name: "Default Slack workspace" })).toHaveValue("acme");
});

test("Editing a row picks the workspace from the connected list and keeps an unlisted one", async () => {
  const saved: string[] = [];
  const api = fakeSettingsApi({
    listMappings: () =>
      Promise.resolve([entry({ slug: "web" }), { ...entry({ slug: "ops" }), row: { ...entry({ slug: "ops" }).row, slack_workspace: "gone" } }]),
    slackWorkspaces: () => Promise.resolve([acme, widgets]),
    saveMapping: (row) => {
      saved.push(`${row.team_slug} ${row.slack_workspace}`);
      return Promise.resolve();
    },
  }) as Api;
  render(<Harness api={api} start="mappings" />);
  await screen.findByRole("option", { name: "Acme (acme.slack.com)" });
  fireEvent.click(await screen.findByRole("button", { name: "Edit acme/web" }));
  const select = screen.getByRole("combobox", { name: "Slack workspace" });
  expect(select).toHaveValue("acme");
  fireEvent.change(select, { target: { value: "widgets" } });
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(saved).toEqual(["web widgets"]));
  fireEvent.click(await screen.findByRole("button", { name: "Edit acme/ops" }));
  expect(screen.getByRole("combobox", { name: "Slack workspace" })).toHaveValue("gone");
  expect(screen.getByRole("option", { name: "gone · not connected" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  fireEvent.click(screen.getByRole("button", { name: "Add mapping" }));
  expect(screen.getByRole("combobox", { name: "Slack workspace" })).toHaveValue("acme");
});

test("About shows paths, the keychain explainer, update check and Re-run setup", async () => {
  let reruns = 0;
  const api = fakeSettingsApi({
    checkUpdate: () =>
      Promise.resolve({ version: "0.2.0", url: "https://github.com/acme/pr-nexus/releases/tag/v0.2.0", command: "gh release download v0.2.0" }),
  }) as Api;
  render(<Harness api={api} start="about" onRerun={() => (reruns += 1)} />);
  expect(await screen.findByRole("heading", { name: "PR Nexus 0.1.0" })).toBeInTheDocument();
  expect(screen.getByText(/one keychain item/)).toBeInTheDocument();
  expect(screen.getByText(/Always Allow/)).toBeInTheDocument();
  expect(screen.getByText(/Open Anyway/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Copy database" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Check for updates" }));
  expect(await screen.findByText(/0.2.0 is available/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Re-run setup" }));
  expect(reruns).toBe(1);
});

test("About warns when tokens only live in memory", async () => {
  const api = fakeSettingsApi({
    appInfo: () => Promise.resolve({ version: "0.1.0", repository: "", db_path: "/tmp/a.db", log_dir: "/tmp/logs", keyring_in_memory: true, os: "linux" }),
    checkUpdate: () => Promise.resolve(null),
  }) as Api;
  render(<Harness api={api} start="about" />);
  expect(await screen.findByRole("alert")).toHaveTextContent(/in memory only/);
  expect(screen.queryByText(/Open Anyway/)).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Check for updates" }));
  expect(await screen.findByText("You have the latest release.")).toBeInTheDocument();
});

test("Sync blocks out-of-range values before they reach the backend", async () => {
  const saved: Preferences[] = [];
  const api = fakeSettingsApi({
    savePreferences: (prefs) => {
      saved.push(prefs);
      return Promise.resolve(prefs);
    },
  }) as Api;
  render(<Harness api={api} start="sync" />);
  fireEvent.change(await screen.findByRole("spinbutton", { name: /window focused/ }), { target: { value: "90" } });
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await Promise.resolve();
  expect(saved).toEqual([]);
});

test("Sync has a Live updates toggle, on by default, that saves off", async () => {
  const saved: Preferences[] = [];
  const api = fakeSettingsApi({
    savePreferences: (prefs) => {
      saved.push(prefs);
      return Promise.resolve(prefs);
    },
  }) as Api;
  render(<Harness api={api} start="sync" />);
  const toggle = await screen.findByRole("switch", { name: "Live updates" });
  expect(toggle).toBeChecked();
  fireEvent.click(toggle);
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByText("Saved")).toBeInTheDocument();
  expect(saved).toEqual([{ ...DEFAULT_PREFERENCES, live_updates: false }]);
});

test("the tab row scrolls at narrow widths and brings the active tab into view", () => {
  const scrolled: string[] = [];
  const original = HTMLElement.prototype.scrollIntoView;
  HTMLElement.prototype.scrollIntoView = function (this: HTMLElement) {
    scrolled.push(this.textContent ?? "");
  };
  try {
    render(<Harness api={fakeSettingsApi() as Api} start="agents" />);
    expect(screen.getByRole("tablist")).toHaveClass("overflow-x-auto");
    expect(scrolled).toContain("Agents (MCP)");
    fireEvent.click(screen.getByRole("tab", { name: "Mappings" }));
    expect(scrolled.at(-1)).toBe("Mappings");
  } finally {
    HTMLElement.prototype.scrollIntoView = original;
  }
});
