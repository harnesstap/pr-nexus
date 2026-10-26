import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { expect, test } from "vitest";
import type { Api, Bootstrap, MappingEntry, Preferences } from "../api";
import { DEFAULT_PREFERENCES, fakeSettingsApi } from "./fakes";
import { SettingsScreen } from "./SettingsScreen";
import type { SettingsTab } from "./tabs";
import { useState } from "react";

const boot = {
  bump_broadcast: true,
  mapping_count: 0,
  onboarding: { mapping_skipped: true },
} as unknown as Bootstrap;

function Harness({ api, start = "accounts", onBoot = () => undefined, onRerun = () => undefined }: { api: Api; start?: SettingsTab; onBoot?: (boot: Bootstrap) => void; onRerun?: () => void }) {
  const [tab, setTab] = useState<SettingsTab>(start);
  return (
    <SettingsScreen api={api} boot={boot} onBoot={onBoot} accounts={<p>Accounts body</p>} tab={tab} onTabChange={setTab} onRerunSetup={onRerun} />
  );
}

const entry = (partial: Partial<MappingEntry> & { slug: string }): MappingEntry => ({
  row: {
    github_org: "acme",
    team_slug: partial.slug,
    team_name: "",
    slack_workspace: "acme.slack.com",
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
  const toggle = await screen.findByRole("checkbox", { name: "Also post bumps to the channel" });
  expect(toggle).toBeChecked();
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  fireEvent.click(toggle);
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByText("Saved.")).toBeInTheDocument();
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
  expect(await screen.findByRole("status")).toHaveTextContent(/between 60 and 3600/);
  expect(saved[0]?.focused_interval_s).toBe(2700);
});

test("Mappings: empty state names the skipped setup step", async () => {
  render(<Harness api={fakeSettingsApi() as Api} start="mappings" />);
  expect(await screen.findByText("No team channels mapped yet.")).toBeInTheDocument();
  expect(screen.getByText(/You skipped this step in setup/)).toBeInTheDocument();
});

test("Mappings: badges, filter, inline edit and reset to imported", async () => {
  let entries = [entry({ slug: "web" }), entry({ slug: "api", source: "user", has_imported: true }), entry({ slug: "ops", source: "user", has_imported: false, disabled: true })];
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
  }) as Api;
  render(<Harness api={api} start="mappings" />);
  const csv = await screen.findByRole("textbox", { name: "Mapping CSV" });
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
  expect(applied).toEqual(["replace  36"]);
});

test("About shows paths, the keychain explainer, update check and Re-run setup", async () => {
  let reruns = 0;
  const api = fakeSettingsApi({
    checkUpdate: () => Promise.resolve({ version: "0.2.0", url: "https://github.com/acme/pr-nexus/releases/tag/v0.2.0", command: "gh release download v0.2.0" }),
  }) as Api;
  render(<Harness api={api} start="about" onRerun={() => (reruns += 1)} />);
  expect(await screen.findByRole("heading", { name: "PR-Nexus 0.1.0" })).toBeInTheDocument();
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
