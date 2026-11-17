import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, test } from "vitest";
import type { Api, Bootstrap, MappingEntry, Preferences, SlackWorkspace } from "../api";
import { DEFAULT_PREFERENCES, fakeMcp, fakeSettingsApi } from "./fakes";
import { AccountsSettings } from "./AccountsSettings";
import { SettingsScreen } from "./SettingsScreen";
import { isSettingsSection, readSettingsSection, sectionAfterKey, settingsNavIsCompact, type SettingsSection } from "./tabs";
import { useState, type ReactNode } from "react";

const boot = {
  bump_broadcast: true,
  mapping_count: 0,
  onboarding: { mapping_skipped: true },
} as unknown as Bootstrap;

function Harness({
  api,
  start,
  onBoot = () => undefined,
  onRerun = () => undefined,
  boot: bootOverride = boot,
  accounts,
}: {
  api: Api;
  start?: SettingsSection;
  onBoot?: (boot: Bootstrap) => void;
  onRerun?: () => void;
  boot?: Bootstrap;
  accounts?: ReactNode;
}) {
  const [tab, setTab] = useState<SettingsSection>(() => start ?? readSettingsSection());
  return (
    <SettingsScreen
      api={api}
      boot={bootOverride}
      onBoot={onBoot}
      accounts={accounts ?? <p>GitHub body</p>}
      tab={tab}
      onTabChange={setTab}
      onRerunSetup={onRerun}
    />
  );
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

test("sections follow the vertical tab pattern: one tab stop, up and down, Home and End", () => {
  render(<Harness api={fakeSettingsApi() as Api} />);
  const github = screen.getByRole("tab", { name: "GitHub" });
  expect(github).toHaveAttribute("aria-selected", "true");
  expect(github).toHaveClass("bg-violet-500/20");
  expect(github.className).toMatch(/focus-visible:outline-violet-400/);
  expect(screen.getByRole("tab", { name: "Mappings" })).toHaveAttribute("tabindex", "-1");
  expect(screen.getByRole("tablist", { name: "Connections" })).toBeInTheDocument();
  expect(screen.getByRole("heading", { name: "Connections" })).toHaveClass("uppercase");
  expect(screen.queryByRole("tab", { name: "Connections" })).not.toBeInTheDocument();
  expect(screen.getByRole("tabpanel")).toHaveTextContent("GitHub body");
  fireEvent.keyDown(github, { key: "ArrowLeft" });
  expect(github).toHaveAttribute("aria-selected", "true");
  fireEvent.keyDown(github, { key: "ArrowUp" });
  const setup = screen.getByRole("tab", { name: "Setup" });
  expect(setup).toHaveAttribute("aria-selected", "true");
  expect(setup).toHaveFocus();
  fireEvent.keyDown(setup, { key: "Home" });
  expect(github).toHaveFocus();
  fireEvent.keyDown(github, { key: "ArrowDown" });
  expect(screen.getByRole("tab", { name: "Slack" })).toHaveAttribute("aria-selected", "true");
  fireEvent.keyDown(screen.getByRole("tab", { name: "Slack" }), { key: "End" });
  expect(setup).toHaveAttribute("aria-selected", "true");
  expect(setup).toHaveFocus();
});

test("arrow keys, Home and End walk the section list", () => {
  expect(sectionAfterKey("github", "ArrowDown")).toBe("slack");
  expect(sectionAfterKey("github", "ArrowUp")).toBe("setup");
  expect(sectionAfterKey("github", "ArrowLeft")).toBeNull();
  expect(sectionAfterKey("github", "ArrowRight")).toBeNull();
  expect(sectionAfterKey("slack", "Home")).toBe("github");
  expect(sectionAfterKey("requests", "End")).toBe("setup");
  expect(isSettingsSection("slack")).toBe(true);
  expect(isSettingsSection("accounts")).toBe(false);
  expect(isSettingsSection("agents")).toBe(false);
});

test("the rail collapses under 960px or when the pane is too narrow", () => {
  expect(settingsNavIsCompact(959, 0)).toBe(true);
  expect(settingsNavIsCompact(960, 759)).toBe(true);
  expect(settingsNavIsCompact(960, 760)).toBe(false);
  expect(settingsNavIsCompact(1440, 0)).toBe(false);
  expect(settingsNavIsCompact(0, 0)).toBe(false);
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
  render(<Harness api={api} start="requests" onBoot={(next) => booted.push(next)} />);
  const toggle = await screen.findByRole("switch", { name: "Also post bumps to the channel" });
  expect(toggle).toBeChecked();
  expect(screen.queryByRole("button", { name: "Save" })).not.toBeInTheDocument();
  fireEvent.click(toggle);
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByText("Saved")).toBeInTheDocument();
  expect(saved).toEqual([{ ...DEFAULT_PREFERENCES, bump_broadcast: false }]);
  expect(booted.map((next) => next.bump_broadcast)).toEqual([false]);
});

test("Review saves the review wait and the open PR limit", async () => {
  const saved: Preferences[] = [];
  const api = fakeSettingsApi({
    savePreferences: (prefs) => {
      saved.push(prefs);
      return Promise.resolve(prefs);
    },
  }) as Api;
  render(<Harness api={api} start="requests" />);
  const hours = await screen.findByRole("spinbutton", { name: "Review now after (hours)" });
  expect(hours).toHaveValue(48);
  fireEvent.change(hours, { target: { value: "24" } });
  fireEvent.click(screen.getByRole("switch", { name: "Count only weekdays" }));
  fireEvent.change(screen.getByRole("spinbutton", { name: "Open PR limit" }), { target: { value: "8" } });
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByText("Saved")).toBeInTheDocument();
  expect(saved).toEqual([{ ...DEFAULT_PREFERENCES, review_sla_hours: 24, review_sla_working_days: true, my_prs_wip_limit: 8 }]);
});

test("Sync edits intervals in minutes and shows an error from the backend", async () => {
  const saved: Preferences[] = [];
  const api = fakeSettingsApi({
    savePreferences: (prefs) => {
      saved.push(prefs);
      return Promise.reject(new Error("sync.focused_interval_s must be between 60 and 3600"));
    },
  }) as Api;
  render(<Harness api={api} start="schedule" />);
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
    slackChannelNames: (channels) => Promise.resolve(channels.map((channel) => (channel.channel_id === "C0ACME0002" ? "web-team" : null))),
    saveMapping: (row, disabled) => {
      calls.push(`save ${row.team_slug} #${row.slack_channel_name} ${row.slack_channel_id} ${disabled}`);
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
  // "Enter an ID" takes the id alone; the name comes from Slack with it.
  fireEvent.click(screen.getByRole("button", { name: "Enter an ID" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Channel ID" }), { target: { value: "C0ACME0002" } });
  fireEvent.click(screen.getByRole("button", { name: "Use ID" }));
  await waitFor(() => expect(screen.getByRole("combobox", { name: "Slack channel" })).toHaveValue("#web-team"));
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByText("Saved acme/web.")).toBeInTheDocument();
  expect(calls).toEqual(["save web #web-team C0ACME0002 false"]);
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
  expect(screen.getByRole("combobox", { name: "Slack channel" })).toBeInTheDocument();
});

test("Mappings: a saved name that is not the id's channel is flagged, not rewritten", async () => {
  const saved: string[] = [];
  const api = fakeSettingsApi({
    listMappings: () => Promise.resolve([entry({ slug: "web" }), entry({ slug: "api" })]),
    // Slack names the shared id "api-reviews": the web row's saved name is stale.
    slackChannelNames: (channels) => Promise.resolve(channels.map(() => "api-reviews")),
    saveMapping: (row) => {
      saved.push(row.team_slug);
      return Promise.resolve();
    },
  }) as Api;
  render(<Harness api={api} start="mappings" />);
  const warning = await screen.findByText(/Saved name #web-reviews doesn't match this channel \(#api-reviews\)/);
  expect(warning).toBeInTheDocument();
  expect(screen.getAllByText(/doesn't match this channel/)).toHaveLength(1);
  expect(screen.getByText("#web-reviews")).toBeInTheDocument();
  expect(saved).toEqual([]);
  fireEvent.click(screen.getByRole("button", { name: "Pick the channel again for acme/web" }));
  expect(screen.getByRole("combobox", { name: "Slack channel" })).toBeInTheDocument();
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

test("Import: with no workspace connected the select is disabled and links to Slack", async () => {
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
  fireEvent.click(screen.getByRole("button", { name: "Connect Slack" }));
  expect(screen.getByRole("tab", { name: "Slack" })).toHaveAttribute("aria-selected", "true");
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
      Promise.resolve({
        kind: "release",
        current: "0.1.0",
        version: "0.2.0",
        url: "https://github.com/acme/pr-nexus/releases/tag/v0.2.0",
        command: "gh release download v0.2.0",
        behind_by: null,
      }),
  }) as Api;
  render(<Harness api={api} start="app" onRerun={() => (reruns += 1)} />);
  expect(await screen.findByRole("heading", { name: "PR Nexus 0.1.0" })).toBeInTheDocument();
  expect(screen.getByText(/Open Anyway/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Copy database" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Check for updates" }));
  expect(await screen.findByText(/0.2.0 is available/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("tab", { name: "Keychain" }));
  expect(await screen.findByText(/one keychain item/)).toBeInTheDocument();
  expect(screen.getByText(/Always Allow/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("tab", { name: "Setup" }));
  fireEvent.click(await screen.findByRole("button", { name: "Re-run setup" }));
  expect(reruns).toBe(1);
});

test("About warns when tokens only live in memory", async () => {
  const api = fakeSettingsApi({
    appInfo: () => Promise.resolve({ version: "0.1.0", repository: "", db_path: "/tmp/a.db", log_dir: "/tmp/logs", keyring_in_memory: true, os: "linux" }),
    checkUpdate: () => Promise.resolve(null),
  }) as Api;
  render(<Harness api={api} start="keychain" />);
  expect(await screen.findByRole("alert")).toHaveTextContent(/in memory only/);
  expect(screen.queryByText(/Open Anyway/)).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("tab", { name: "App & updates" }));
  fireEvent.click(await screen.findByRole("button", { name: "Check for updates" }));
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
  render(<Harness api={api} start="schedule" />);
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
  render(<Harness api={api} start="schedule" />);
  const toggle = await screen.findByRole("switch", { name: "Live updates" });
  expect(toggle).toBeChecked();
  fireEvent.click(toggle);
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByText("Saved")).toBeInTheDocument();
  expect(saved).toEqual([{ ...DEFAULT_PREFERENCES, live_updates: false }]);
});

test("Sync notification toggles save, and a denial is a grey note", async () => {
  const saved: Preferences[] = [];
  const api = fakeSettingsApi({
    savePreferences: (prefs) => {
      saved.push(prefs);
      return Promise.resolve(prefs);
    },
    notificationPermission: () => Promise.resolve("denied"),
  }) as Api;
  render(<Harness api={api} start="notifications" />);
  expect(await screen.findByRole("switch", { name: "New comments" })).not.toBeChecked();
  const hint = await screen.findByText("Notifications are off for PR Nexus in system settings.");
  expect(hint).not.toHaveAttribute("role", "alert");
  expect(hint.className).toMatch(/text-slate-500/);
  fireEvent.click(screen.getByRole("switch", { name: "New comments" }));
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByText("Saved")).toBeInTheDocument();
  expect(saved[0]?.notify_comment).toBe(true);
  expect(saved[0]?.notify_enabled).toBe(true);
});

test("the rail scrolls the active section into view", () => {
  const scrolled: string[] = [];
  const original = HTMLElement.prototype.scrollIntoView;
  HTMLElement.prototype.scrollIntoView = function (this: HTMLElement) {
    scrolled.push(this.textContent ?? "");
  };
  try {
    render(<Harness api={fakeSettingsApi() as Api} start="activity" />);
    expect(screen.getByRole("navigation", { name: "Settings" })).toHaveClass("overflow-y-auto");
    expect(scrolled).toContain("Activity");
    fireEvent.click(screen.getByRole("tab", { name: "Mappings" }));
    expect(scrolled.at(-1)).toBe("Mappings");
  } finally {
    HTMLElement.prototype.scrollIntoView = original;
  }
});

test("a narrow window uses a section menu instead of the rail", async () => {
  const original = window.matchMedia;
  window.matchMedia = ((query: string) => ({
    matches: query.includes("959"),
    media: query,
    onchange: null,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
    dispatchEvent: () => false,
    addListener: () => undefined,
    removeListener: () => undefined,
  })) as typeof window.matchMedia;
  try {
    render(<Harness api={fakeSettingsApi() as Api} start="rules" />);
    const menu = await screen.findByRole("combobox", { name: "Settings section" });
    expect(screen.queryByRole("tablist")).not.toBeInTheDocument();
    expect(menu).toHaveValue("rules");
    expect(await screen.findByRole("heading", { name: "Rules" })).toBeInTheDocument();
    fireEvent.change(menu, { target: { value: "requests" } });
    expect(await screen.findByRole("heading", { name: "Requests & bumps" })).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "Settings section" })).toHaveValue("requests");
  } finally {
    window.matchMedia = original;
  }
});

test("the last opened section is remembered", () => {
  const { unmount } = render(<Harness api={fakeSettingsApi() as Api} />);
  fireEvent.click(screen.getByRole("tab", { name: "Rules" }));
  expect(window.localStorage.getItem("pr-nexus.settings-section")).toBe("rules");
  unmount();
  render(<Harness api={fakeSettingsApi() as Api} />);
  expect(screen.getByRole("tab", { name: "Rules" })).toHaveAttribute("aria-selected", "true");
});

test("a programmatic section opens that section", async () => {
  render(<Harness api={fakeSettingsApi() as Api} start="changelog" />);
  expect(screen.getByRole("tab", { name: "Changelog" })).toHaveAttribute("aria-selected", "true");
  expect(await screen.findByRole("heading", { name: "Recent changes" })).toBeInTheDocument();
  expect(window.localStorage.getItem("pr-nexus.settings-section")).toBe("changelog");
});

test("Agents opens on Permissions without the server controls", async () => {
  render(<Harness api={fakeSettingsApi() as Api} start="permissions" />);
  expect(screen.getByRole("tab", { name: "Permissions" })).toHaveAttribute("aria-selected", "true");
  expect(await screen.findByRole("heading", { name: "What agents may do" })).toBeInTheDocument();
  expect(screen.queryByRole("switch", { name: "Enable MCP server" })).not.toBeInTheDocument();
});

test("Activity and Server links open those sections", async () => {
  render(<Harness api={fakeSettingsApi(fakeMcp({ enabled: true, local_writes: true })) as Api} start="permissions" />);
  fireEvent.click(await screen.findByRole("button", { name: "Activity" }));
  expect(screen.getByRole("tab", { name: "Activity" })).toHaveAttribute("aria-selected", "true");
  expect(await screen.findByRole("heading", { name: "Activity" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("tab", { name: "Connect a client" }));
  fireEvent.click(await screen.findByRole("button", { name: "Server" }));
  expect(screen.getByRole("tab", { name: "Server" })).toHaveAttribute("aria-selected", "true");
  expect(await screen.findByRole("heading", { name: "Local MCP server" })).toBeInTheDocument();
});

test("an unsaved preferences edit stays across sections of that form", async () => {
  const saved: Preferences[] = [];
  const api = fakeSettingsApi({
    savePreferences: (prefs) => {
      saved.push(prefs);
      return Promise.resolve(prefs);
    },
  }) as Api;
  render(<Harness api={api} start="requests" />);
  const requester = await screen.findByRole("textbox", { name: /^Requester/ });
  fireEvent.change(requester, { target: { value: "Ada" } });
  expect(screen.getByRole("region", { name: "Unsaved changes" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("tab", { name: "Rules" }));
  fireEvent.change(await screen.findByRole("textbox", { name: "JIRA base URL" }), { target: { value: "https://acme-labs.atlassian.net" } });
  fireEvent.click(screen.getByRole("tab", { name: "Mappings" }));
  expect(await screen.findByText("No team channels mapped yet.")).toBeInTheDocument();
  expect(screen.queryByRole("region", { name: "Unsaved changes" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("tab", { name: "Requests & bumps" }));
  expect(screen.getByRole("textbox", { name: /^Requester/ })).toHaveValue("Ada");
  fireEvent.click(screen.getByRole("tab", { name: "Schedule" }));
  expect(screen.getByRole("region", { name: "Unsaved changes" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByText("Saved")).toBeInTheDocument();
  expect(saved[0]?.review_requester).toBe("Ada");
  expect(saved[0]?.jira_base_url).toBe("https://acme-labs.atlassian.net");
});

test("GitHub and Slack share one draft", async () => {
  const rich = {
    ...boot,
    github_login: "ada",
    org_choices: ["acme"],
    tracked_orgs: ["acme"],
    include_personal: false,
    orgs_needing_sso: [],
    slack_client_id: "111.222",
    slack_redirect_uri: "http://127.0.0.1:47821/mcp/oauth/callback",
    slack_secret_saved: true,
    slack_mentions: "off",
    slack_workspaces: [],
  } as Bootstrap;
  render(
    <Harness
      api={fakeSettingsApi() as Api}
      start="github"
      boot={rich}
      accounts={<AccountsSettings api={fakeSettingsApi() as Api} boot={rich} onOrgsSaved={() => Promise.resolve()} onMentionsChange={() => undefined} />}
    />,
  );
  const personal = await screen.findByRole("switch", { name: "Include my personal repos" });
  fireEvent.click(personal);
  expect(screen.getByRole("region", { name: "Unsaved changes" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("tab", { name: "Slack" }));
  expect(await screen.findByRole("heading", { name: "Slack" })).toBeInTheDocument();
  expect(screen.getByRole("region", { name: "Unsaved changes" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("tab", { name: "Profiles" }));
  fireEvent.click(screen.getByRole("tab", { name: "GitHub" }));
  expect(screen.getByRole("switch", { name: "Include my personal repos" })).toBeChecked();
});
