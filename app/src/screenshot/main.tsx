import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "../App";
import "../index.css";
import { pageFromRows, type PageRequest } from "../list/paging";
import type { ReviewTarget } from "../api";
import { boot, detail, draftOverview, draftRowId, mergeableOverview, mergeableRowId, overview, projects, reviewChoices, reviewText, rows, searchTargets, slackThreads } from "./fixtures";

const overviewFor = (id: unknown) => (id === draftRowId ? draftOverview : id === mergeableRowId ? mergeableOverview : overview);

/**
 * The real app over mocked Tauri IPC, for the README screenshot
 * (`app/scripts/readme-screenshot.mjs`). Commands not listed return null.
 */
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
  load_preferences: () => ({ min_bump_interval_hours: 4 }),
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
