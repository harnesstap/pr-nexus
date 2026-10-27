import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "../App";
import "../index.css";
import { pageFromRows, type PageRequest } from "../list/paging";
import { boot, projects, rows } from "./fixtures";

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
