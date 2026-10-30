import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "../App";
import "../index.css";
import { pageFromRows, type PageRequest } from "../list/paging";
import { DATA_CHANGED_EVENT } from "../api";
import type { BulkRequest, JiraLinkDto, JiraLinkInput, ListRow, McpClientId, McpOutcome, McpSettings, PrWarning, ReviewTarget } from "../api";
import { fakeMcp, sampleActivity } from "../settings/fakes";
import {
  boot,
  branchFixtures,
  detail,
  draftOverview,
  draftRowId,
  jiraBaseUrl,
  mergeableOverview,
  mergeableRowId,
  notes,
  overview,
  projects,
  reviewChoices,
  reviewText,
  rows,
  searchTargets,
  slackThreads,
  slackVariants,
} from "./fixtures";

const baseOverviewFor = (id: unknown) => (id === draftRowId ? draftOverview : id === mergeableRowId ? mergeableOverview : overview);
const overviewFor = (id: unknown) => ({ ...baseOverviewFor(id), ...(branchFixtures[Number(id)]?.overview ?? {}) });
const branchFor = (id: unknown) => branchFixtures[Number(id)]?.branch ?? branchFixtures[1].branch;

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

/** Script defaults (§7.6). The real preview comes from the Rust builder; this mock only fills the screenshot. */
const templateDefaults: Record<string, string> = {
  single_request: "Hello team\nCould someone please review {title}?{who}\n{url}\nThanks!",
  batch_request: "Hello team\nCould someone please review these pull requests?{who}\n{items}\nThanks!",
  single_remind: "Hello, I'm still looking for a review on {title} when you have a moment :pray:\n{url}\nThanks!",
  batch_remind: "Hello, I'm still looking for a review on these pull requests when you have a moment :pray:\n{items}\nThanks!",
  item: "- {title}\n{url}",
};
const samplePrs = [
  ["`fix: handle user_id in 'parser'`", "https://github.com/acme/widgets/pull/42"],
  ["`feat: add retry_count to the client`", "https://github.com/acme/widgets/pull/43"],
];
function mockTemplatePreview(kind: string, text: string): string {
  const batch = kind.startsWith("batch_") || kind === "item";
  const unknown = /\{(?!\{)([^}]*)\}/g;
  for (const match of text.matchAll(unknown)) {
    const name = match[1];
    const allowed = batch && kind !== "item" ? ["items", "who", "requester", "count"] : ["title", "url", "who", "requester", "count"];
    if (!allowed.includes(name)) throw { kind: "validation", message: `unknown placeholder \`{${name}}\`` };
  }
  const required = kind === "batch_request" || kind === "batch_remind" ? "{items}" : "{url}";
  if (!text.includes(required)) throw { kind: "validation", message: `the template must contain \`${required}\`` };
  const fill = (template: string, title: string, url: string, items = "") =>
    template.replace(/\{(\w+)\}/g, (_, name: string) => ({ title, url, who: " (Ada Park)", requester: "Ada Park", count: batch ? "2" : "1", items })[name] ?? "");
  if (!batch) return fill(text, samplePrs[0][0], samplePrs[0][1]).trimEnd() + "\n";
  const item = kind === "item" ? text : templateDefaults.item;
  const items = samplePrs.map(([title, url]) => fill(item, title, url)).join("\n");
  return fill(kind === "item" ? templateDefaults.batch_request : text, "", "", items).trimEnd() + "\n";
}

const mcp = fakeMcp({ enabled: true, local_writes: true }, sampleActivity(), [], [], { codex: "added", cursor: "outdated" });

const handlers: Record<string, (args: Record<string, unknown>) => unknown> = {
  mcp_settings: () => mcp.mcpSettings(),
  save_mcp_settings: (args) => mcp.saveMcpSettings(args.settings as McpSettings),
  mcp_token: () => mcp.mcpToken(),
  regenerate_mcp_token: () => mcp.regenerateMcpToken(),
  mcp_client_states: () => mcp.mcpClientStates(),
  mcp_add_to_client: (args) => mcp.mcpAddToClient(args.client as McpClientId),
  mcp_open_client_config: (args) => mcp.mcpOpenClientConfig(args.client as McpClientId),
  mcp_activity: (args) => mcp.mcpActivity((args.outcome as McpOutcome | null) ?? undefined),
  clear_mcp_activity: () => mcp.clearMcpActivity(),
  load_message_templates: () => Object.entries(templateDefaults).map(([kind, text]) => ({ kind, text, default_text: text })),
  preview_message_template: (args) => mockTemplatePreview(String(args.kind), String(args.text)),
  save_message_template: () => null,
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
  pr_branch_status: (args) => branchFor(args.id),
  update_pr_branch: (args) => ({ overview: { ...baseOverviewFor(args.id), fetched_at: new Date().toISOString() }, branch: branchFixtures[1].branch }),
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
  list_project_links: (args) =>
    rows.filter((row) => row.project_ids.includes(Number(args.projectId))).map((row) => ({ pr_id: row.id, source: "manual", rule_id: null })),
  rows_by_id: (args) => rows.filter((row) => (args.ids as number[]).includes(row.id)),
  review_targets: () => reviewChoices,
  bulk_preview: (args) => {
    const request = args.request as BulkRequest;
    const picked = rows.filter((item) => request.pr_ids.includes(item.id));
    const prs = picked.map((item) => ({ pr_id: item.id, number: item.number, title: item.title, repo: item.repo }));
    const text = `Hello team\nCould someone please review these pull requests? (Ada Park)\n${picked
      .map((item) => `- \`${item.title}\`\nhttps://github.com/${item.repo}/pull/${item.number}`)
      .join("\n")}\nThanks!\n`;
    const target = request.target ?? reviewChoices[0].target;
    return {
      cards: [
        {
          target,
          messages: [{ action: "request", prs, text, permalink: null }],
          warnings: [],
          signed_in: true,
          single_top_level: true,
          can_switch: true,
          fingerprint: "fp-screenshot",
        },
      ],
      skipped: [],
      pr_warnings: picked.flatMap(draftWarnings),
    };
  },
  review_preview: (args) => {
    const target = args.target as ReviewTarget;
    const row = rows.find((item) => item.id === args.prId);
    return {
      target,
      action: "request",
      text: reviewText(row?.title ?? "", row ? `https://github.com/${row.repo}/pull/${row.number}` : ""),
      warnings: [],
      pr_warnings: row ? draftWarnings(row) : [],
      signed_in: true,
      bump_count: 0,
      permalink: null,
      broadcast: true,
    };
  },
  slack_search_targets: (args) => searchTargets(String(args.query ?? "")),
  set_pr_draft: (args) => {
    const row = rows.find((item) => item.id === args.id);
    if (row) {
      row.is_draft = args.draft === true;
    }
    return {
      ...draftOverview,
      is_draft: args.draft === true,
      merge_state: args.draft === true ? "draft" : "blocked",
      fetched_at: new Date().toISOString(),
    };
  },
  pr_slack_context: (args) =>
    args.prId === draftRowId || args.prId === mergeableRowId
      ? []
      : (slackVariants[(window as { __slackVariant?: string }).__slackVariant ?? ""] ?? slackThreads),
  pr_slack_mentions: () => [],
};

/** The backend's `slack::pr_warnings`: open drafts only. */
function draftWarnings(row: ListRow): PrWarning[] {
  return row.state === "open" && row.is_draft
    ? [{ kind: "draft", pr_id: row.id, number: row.number, message: `#${row.number} is still a draft. Reviewers may skip it until it's marked ready.` }]
    : [];
}

// `--slack-draft` scenes turn extra rows into drafts before the app loads.
for (const id of (window as { __extraDrafts?: number[] }).__extraDrafts ?? []) {
  const row = rows.find((item) => item.id === id);
  if (row) {
    row.is_draft = true;
  }
}

/** Plays an agent's MCP `set_pr_starred` + `set_pr_note` from the page, for the live-refresh screenshot. */
(window as { __agentStarAndNote?: (id: number, note: string) => Promise<void> }).__agentStarAndNote = async (id, note) => {
  const row = rows.find((item) => item.id === id);
  if (row) {
    row.starred = true;
  }
  notes[id] = note;
  boot.sync.revision += 1; // the backend bumps it on every agent write
  await emit(DATA_CHANGED_EVENT);
};

mockWindows("main");
mockIPC((cmd, args) => handlers[cmd]?.((args ?? {}) as Record<string, unknown>) ?? null, { shouldMockEvents: true });

const root = document.getElementById("root");
if (!root) {
  throw new Error("root element missing");
}
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
