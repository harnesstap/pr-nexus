import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "../App";
import "../index.css";
import { pageFromRows, type PageRequest } from "../list/paging";
import type { JiraLinkDto, JiraLinkInput, ReviewTarget } from "../api";
import { boot, detail, draftOverview, draftRowId, jiraBaseUrl, mergeableOverview, mergeableRowId, notes, overview, projects, reviewChoices, reviewText, rows, searchTargets, slackThreads } from "./fixtures";

const overviewFor = (id: unknown) => (id === draftRowId ? draftOverview : id === mergeableRowId ? mergeableOverview : overview);

/**
 * The real app over mocked Tauri IPC, for the README screenshot
 * (`app/scripts/readme-screenshot.mjs`). Commands not listed return null.
 */
const stamp = () => new Date().toISOString();
let nextLink = 100;

/** Mirrors the backend: a browse URL keeps its key, a bare key links once a base URL is set. */
function jiraLink(input: JiraLinkInput): JiraLinkDto {
  const text = input.text.trim();
  const fromUrl = /\/browse\/([A-Z][A-Z0-9]*-\d+)/.exec(text);
  nextLink += 1;
  if (text.startsWith("http")) {
    return { id: nextLink, key: fromUrl?.[1] ?? "", url: text, href: text };
  }
  return { id: nextLink, key: text.toUpperCase(), url: "", href: `${jiraBaseUrl}/browse/${text.toUpperCase()}` };
}

const handlers: Record<string, (args: Record<string, unknown>) => unknown> = {
  bootstrap: () => boot,
  sync_status: () => boot.sync,
  list_projects: () => projects,
  list_page: (args) => pageFromRows(rows, projects, args.request as PageRequest),
  load_view_state: () => null,
  slack_workspaces: () => boot.slack_workspaces,
  check_update: () => null,
  pr_detail: () => detail,
  pr_overview: (args) => ({ ...overviewFor(args.id), fetched_at: new Date().toISOString() }),
  merge_pr: (args) => ({ ...overviewFor(args.id), state: "merged", fetched_at: new Date().toISOString() }),
  load_preferences: () => ({
    review_requester: "Ada Park",
    min_bump_interval_hours: 4,
    bulk_bump_max_threads: 5,
    bump_broadcast: true,
    blocked_review_required_is_ready: true,
    ticket_key_regex: "[A-Z][A-Z0-9]+-\\d+",
    focused_interval_s: 300,
    unfocused_interval_s: 900,
    keep_closed_days: 14,
    live_updates: true,
    jira_base_url: jiraBaseUrl,
  }),
  set_pr_starred: (args) => {
    const row = rows.find((item) => item.id === args.prId);
    if (row) {
      row.starred = args.starred === true;
    }
    return args.starred === true;
  },
  pr_note: (args) => ({ pr_id: args.prId, note: notes[Number(args.prId)] ?? "", updated_at: notes[Number(args.prId)] ? stamp() : null }),
  set_pr_note: (args) => {
    notes[Number(args.prId)] = String(args.note ?? "").trim();
    return { pr_id: args.prId, note: notes[Number(args.prId)], updated_at: stamp() };
  },
  set_project_description: (args) => {
    const project = projects.find((item) => item.id === args.projectId)!;
    project.description = String(args.description ?? "").trim();
    return project;
  },
  add_jira_link: (args) => {
    const project = projects.find((item) => item.id === args.projectId)!;
    const link = jiraLink(args.link as JiraLinkInput);
    project.jira_links = [...project.jira_links, link];
    return link;
  },
  set_jira_links: (args) => {
    const project = projects.find((item) => item.id === args.projectId)!;
    project.jira_links = (args.links as JiraLinkInput[]).map(jiraLink);
    return project.jira_links;
  },
  remove_jira_link: (args) => {
    for (const project of projects) {
      project.jira_links = project.jira_links.filter((link) => link.id !== args.linkId);
    }
    return null;
  },
  list_project_rules: () => [],
  list_project_links: (args) => rows.filter((row) => row.project_ids.includes(Number(args.projectId))).map((row) => ({ pr_id: row.id, source: "manual", rule_id: null })),
  rows_by_id: (args) => rows.filter((row) => (args.ids as number[]).includes(row.id)),
  review_targets: () => reviewChoices,
  review_preview: (args) => {
    const target = args.target as ReviewTarget;
    const row = rows.find((item) => item.id === args.prId);
    return { target, action: "request", text: reviewText(row?.title ?? "", row ? `https://github.com/${row.repo}/pull/${row.number}` : ""), warnings: [], signed_in: true, bump_count: 0, permalink: null, broadcast: true };
  },
  slack_search_targets: (args) => searchTargets(String(args.query ?? "")),
  set_pr_draft: (args) => ({ ...draftOverview, is_draft: args.draft === true, merge_state: args.draft === true ? "draft" : "blocked", fetched_at: new Date().toISOString() }),
  pr_slack_context: (args) => (args.prId === draftRowId || args.prId === mergeableRowId ? [] : slackThreads),
  pr_slack_mentions: () => [],
};

mockWindows("main");
mockIPC((cmd, args) => handlers[cmd]?.((args ?? {}) as Record<string, unknown>) ?? null);

const root = document.getElementById("root");
if (!root) {
  throw new Error("root element missing");
}
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
