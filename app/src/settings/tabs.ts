export const SETTINGS_TABS = [
  { id: "accounts", label: "Accounts" },
  { id: "mappings", label: "Mappings" },
  { id: "review", label: "Review" },
  { id: "sync", label: "Sync" },
  { id: "agents", label: "Agents (MCP)" },
  { id: "about", label: "About" },
] as const;

export type SettingsTab = (typeof SETTINGS_TABS)[number]["id"];

/** Arrow keys, Home and End move between tabs (WAI-ARIA tabs pattern). */
export function tabAfterKey(current: SettingsTab, key: string): SettingsTab | null {
  const ids = SETTINGS_TABS.map((tab) => tab.id);
  const index = ids.indexOf(current);
  switch (key) {
    case "ArrowRight":
      return ids[(index + 1) % ids.length];
    case "ArrowLeft":
      return ids[(index - 1 + ids.length) % ids.length];
    case "Home":
      return ids[0];
    case "End":
      return ids[ids.length - 1];
    default:
      return null;
  }
}

/** Seconds ↔ whole minutes for the Sync tab. */
export function toMinutes(seconds: number): number {
  return Math.max(1, Math.round(seconds / 60));
}

export const MAPPING_FIELDS: Record<string, string> = {
  github_team_name: "team name",
  slack_channel_name: "channel",
  slack_channel_id: "channel id",
  slack_workspace: "workspace",
};

/** Case-insensitive match on org, slug, team name, workspace or channel. */
export function matchesMapping(
  row: { github_org: string; team_slug: string; team_name: string; slack_workspace: string; slack_channel_name: string; slack_channel_id: string },
  query: string,
): boolean {
  const needle = query.trim().toLowerCase().replace(/^#/, "");
  if (!needle) {
    return true;
  }
  return [row.github_org, row.team_slug, row.team_name, row.slack_workspace, row.slack_channel_name, row.slack_channel_id].some((value) =>
    value.toLowerCase().includes(needle),
  );
}

/** A port typed in Settings → Agents: empty is "any free port". */
export function parsePort(text: string): { port: number | null } | { error: string } {
  const value = text.trim();
  if (!value) {
    return { port: null };
  }
  const port = Number(value);
  if (!Number.isInteger(port) || port < 1024 || port > 65535) {
    return { error: "Use a port from 1024 to 65535, or leave it empty." };
  }
  return { port };
}

/** A generic MCP client entry for the endpoint. */
export function mcpConfigSnippet(url: string, token: string): string {
  return JSON.stringify({ mcpServers: { "pr-nexus": { type: "http", url, headers: { Authorization: `Bearer ${token}` } } } }, null, 2);
}
