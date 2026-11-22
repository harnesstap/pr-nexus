import type { McpClientState, SlackWorkspace } from "../api";
import type { McpClient } from "../settings/mcpClients";
import { MCP_CLIENTS } from "../settings/mcpClients";
import type { SettingsSection } from "../settings/tabs";

/** One setup problem. The id stays stable until the problem is fixed. */
export interface SetupProblem {
  id: string;
  message: string;
  action: string;
  target: SettingsSection;
  /** AI agent tab to open, when the problem is an outdated MCP config. */
  client?: McpClient;
}

const DISMISS_KEY = "pr-nexus.setup-notices-dismissed";

/**
 * Problems worth a notice outside Settings. GitHub token expiry stays on the
 * sync banner. An outdated MCP config and a Slack workspace with no grant
 * each produce one notice.
 */
export function setupProblems(input: { clients: readonly McpClientState[]; workspaces: readonly SlackWorkspace[] }): SetupProblem[] {
  const problems: SetupProblem[] = [];
  for (const state of input.clients) {
    if (state.status !== "outdated") {
      continue;
    }
    const label = MCP_CLIENTS.find((client) => client.id === state.client)?.label ?? state.client;
    problems.push({
      id: `mcp-outdated:${state.client}`,
      message: `${label} has an outdated PR Nexus config.`,
      action: "Update",
      target: "client",
      client: state.client,
    });
  }
  for (const workspace of input.workspaces) {
    if (!workspace.enabled || workspace.check !== "missing") {
      continue;
    }
    const name = workspace.team_name || workspace.domain;
    problems.push({
      id: `slack-missing:${workspace.team_id}`,
      message: `${name} Slack is missing a grant.`,
      action: "Reconnect",
      target: "slack",
    });
  }
  return problems;
}

function readIds(): string[] {
  try {
    const raw = window.localStorage.getItem(DISMISS_KEY);
    if (!raw) {
      return [];
    }
    const parsed = JSON.parse(raw) as unknown;
    return Array.isArray(parsed) ? parsed.filter((item): item is string => typeof item === "string") : [];
  } catch {
    return [];
  }
}

function writeIds(ids: readonly string[]) {
  try {
    window.localStorage.setItem(DISMISS_KEY, JSON.stringify(ids));
  } catch {
    // Private mode or a full disk: the notice comes back next launch.
  }
}

export function dismissSetupNotice(id: string) {
  const ids = readIds();
  if (!ids.includes(id)) {
    writeIds([...ids, id]);
  }
}

/**
 * Drop dismissals for problems that are fixed, so the same problem shows
 * again if it comes back. Problems that are still active stay dismissed.
 */
export function reconcileDismissed(active: readonly string[]): Set<string> {
  const live = new Set(active);
  const next = readIds().filter((id) => live.has(id));
  writeIds(next);
  return new Set(next);
}
