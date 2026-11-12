export const SETTINGS_TABS = [
  { id: "accounts", label: "Accounts" },
  { id: "profiles", label: "Profiles" },
  { id: "mappings", label: "Mappings" },
  { id: "review", label: "Review" },
  { id: "sync", label: "Sync" },
  { id: "agents", label: "Agents (MCP)" },
  { id: "changelog", label: "Changelog" },
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

const EXTRA_HOST_ERROR = "Use a hostname, or a hostname and port such as host.docker.internal:49708.";

/**
 * One Extra allowed hosts entry: a hostname or IPv4 address, optionally
 * with a port. Wildcards, schemes and paths are refused. The returned
 * host is the canonical form the server stores.
 */
export function parseExtraHost(text: string): { host: string } | { error: string } {
  const raw = text.trim();
  if (raw.length === 0 || raw.length > 300 || /[*\\/?#@\s]/.test(raw) || raw.includes("://")) {
    return { error: EXTRA_HOST_ERROR };
  }
  const colon = raw.lastIndexOf(":");
  let name = raw;
  let portText: string | undefined;
  if (colon !== -1) {
    name = raw.slice(0, colon);
    portText = raw.slice(colon + 1);
    if (name.length === 0 || name.includes(":")) {
      return { error: EXTRA_HOST_ERROR };
    }
  }
  const host = canonicalHost(name);
  if (!host) {
    return { error: EXTRA_HOST_ERROR };
  }
  if (portText === undefined) {
    return { host };
  }
  if (!/^[1-9][0-9]{0,4}$/.test(portText)) {
    return { error: EXTRA_HOST_ERROR };
  }
  const port = Number(portText);
  if (port > 65535) {
    return { error: EXTRA_HOST_ERROR };
  }
  return { host: `${host}:${port}` };
}

function canonicalHost(name: string): string | null {
  const parts = name.split(".");
  if (parts.length === 4 && parts.every((part) => /^[0-9]+$/.test(part))) {
    return canonicalIpv4(parts);
  }
  const lower = name.toLowerCase();
  return isDnsName(lower) ? lower : null;
}

function canonicalIpv4(parts: string[]): string | null {
  const octets: number[] = [];
  for (const part of parts) {
    if (!/^[0-9]{1,3}$/.test(part) || (part.length > 1 && part.startsWith("0"))) {
      return null;
    }
    const octet = Number(part);
    if (octet > 255) {
      return null;
    }
    octets.push(octet);
  }
  return octets.join(".");
}

function isDnsName(host: string): boolean {
  if (host.length === 0 || host.length > 253) {
    return false;
  }
  return host.split(".").every((label) => {
    if (label.length === 0 || label.length > 63 || label.startsWith("-") || label.endsWith("-")) {
      return false;
    }
    return /^[a-z0-9-]+$/.test(label);
  });
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
