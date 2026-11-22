/** A page in the settings rail. Single-page areas have no section header. */
export type SettingsPage = "github" | "slack" | "profiles" | "review" | "sync" | "mcp" | "agent-activity" | "about";

/**
 * A rail page, or an older section id. Older ids still open the merged page
 * and scroll to that subsection (`requests` opens Review at Requests & bumps).
 */
export type SettingsSection =
  | SettingsPage
  | "requests"
  | "rules"
  | "mappings"
  | "schedule"
  | "notifications"
  | "server"
  | "permissions"
  | "client"
  | "proposals"
  | "activity"
  | "app"
  | "changelog"
  | "keychain"
  | "setup";

export interface SettingsNavPage {
  id: SettingsPage;
  label: string;
}

export type SettingsNavEntry =
  | { kind: "group"; id: string; label: string; pages: readonly SettingsNavPage[] }
  | ({ kind: "page" } & SettingsNavPage);

/** Rail order. A group label is a heading only when it gathers more than one page. */
export const SETTINGS_NAV: readonly SettingsNavEntry[] = [
  {
    kind: "group",
    id: "connections",
    label: "Connections",
    pages: [
      { id: "github", label: "GitHub" },
      { id: "slack", label: "Slack" },
      { id: "profiles", label: "Profiles" },
    ],
  },
  { kind: "page", id: "review", label: "Review" },
  { kind: "page", id: "sync", label: "Sync" },
  {
    kind: "group",
    id: "agents",
    label: "AI agents",
    pages: [
      { id: "mcp", label: "MCP server" },
      { id: "agent-activity", label: "Agent activity" },
    ],
  },
  { kind: "page", id: "about", label: "About" },
];

const PAGES: readonly SettingsNavPage[] = SETTINGS_NAV.flatMap((entry) => (entry.kind === "group" ? entry.pages : [entry]));

const PAGE_IDS: readonly SettingsPage[] = PAGES.map((page) => page.id);

const PAGE_LABELS: Record<SettingsPage, string> = {
  github: "GitHub",
  slack: "Slack",
  profiles: "Profiles",
  review: "Review",
  sync: "Sync",
  mcp: "MCP server",
  "agent-activity": "Agent activity",
  about: "About",
};

/** Older section ids. The anchor is the element scrolled into view. */
const LEGACY: Record<Exclude<SettingsSection, SettingsPage>, { page: SettingsPage; anchor: string }> = {
  requests: { page: "review", anchor: "requests" },
  rules: { page: "review", anchor: "rules" },
  mappings: { page: "review", anchor: "mappings" },
  schedule: { page: "sync", anchor: "schedule" },
  notifications: { page: "sync", anchor: "notifications" },
  server: { page: "mcp", anchor: "server" },
  permissions: { page: "mcp", anchor: "permissions" },
  client: { page: "mcp", anchor: "client" },
  proposals: { page: "agent-activity", anchor: "proposals" },
  activity: { page: "agent-activity", anchor: "activity" },
  app: { page: "about", anchor: "app" },
  changelog: { page: "about", anchor: "changelog" },
  keychain: { page: "about", anchor: "keychain" },
  setup: { page: "about", anchor: "setup" },
};

const SECTION_IDS: readonly string[] = [...PAGE_IDS, ...Object.keys(LEGACY)];

export function isSettingsPage(value: string): value is SettingsPage {
  return (PAGE_IDS as readonly string[]).includes(value);
}

export function isSettingsSection(value: string): value is SettingsSection {
  return SECTION_IDS.includes(value);
}

export function settingsPage(target: SettingsSection): SettingsPage {
  if (isSettingsPage(target)) {
    return target;
  }
  return LEGACY[target].page;
}

/** Subsection to scroll to, or null when the target is a whole page. */
export function settingsAnchor(target: SettingsSection): string | null {
  if (isSettingsPage(target)) {
    return null;
  }
  return LEGACY[target].anchor;
}

export function settingsSectionLabel(id: SettingsSection): string {
  return PAGE_LABELS[settingsPage(id)];
}

/** The window minimum. Below this, settings navigation is the section menu. */
export const SETTINGS_NAV_VIEWPORT_PX = 960;

/** Below this settings-pane width the 180px rail leaves too little room for a form. */
export const SETTINGS_NAV_PANE_PX = 760;

/**
 * Compact navigation: the viewport is under the window minimum, or the
 * settings pane itself is too narrow. A width of 0 means it has not been
 * measured yet, so the rail stays.
 */
export function settingsNavIsCompact(viewportWidth: number, paneWidth: number): boolean {
  if (viewportWidth > 0 && viewportWidth < SETTINGS_NAV_VIEWPORT_PX) {
    return true;
  }
  if (paneWidth > 0 && paneWidth < SETTINGS_NAV_PANE_PX) {
    return true;
  }
  return false;
}

/** Last section opened, per machine. Not a setting: it is UI state, like dismissed banners. */
export const SETTINGS_SECTION_KEY = "pr-nexus.settings-section";

export function readSettingsSection(): SettingsSection {
  try {
    const stored = window.localStorage.getItem(SETTINGS_SECTION_KEY);
    if (stored && isSettingsSection(stored)) {
      return stored;
    }
  } catch {
    // Private mode can throw on storage reads.
  }
  return "github";
}

export function writeSettingsSection(section: SettingsSection): void {
  try {
    window.localStorage.setItem(SETTINGS_SECTION_KEY, section);
  } catch {
    // Ignore storage failures.
  }
}

/** Up and down, Home and End move between pages (vertical tabs). */
export function sectionAfterKey(current: SettingsSection, key: string): SettingsPage | null {
  const index = PAGE_IDS.indexOf(settingsPage(current));
  switch (key) {
    case "ArrowDown":
      return PAGE_IDS[(index + 1) % PAGE_IDS.length];
    case "ArrowUp":
      return PAGE_IDS[(index - 1 + PAGE_IDS.length) % PAGE_IDS.length];
    case "Home":
      return PAGE_IDS[0];
    case "End":
      return PAGE_IDS[PAGE_IDS.length - 1];
    default:
      return null;
  }
}

/** Seconds ↔ whole minutes for the Schedule section. 0 is a cleared field. */
export function toMinutes(seconds: number): number | "" {
  return seconds === 0 ? "" : Math.max(1, Math.round(seconds / 60));
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
