/** Settings groups. The labels match the sidebar headings (CSS uppercases them). */
export const SETTINGS_GROUPS = [
  {
    id: "connections",
    label: "Connections",
    sections: [
      { id: "github", label: "GitHub" },
      { id: "slack", label: "Slack" },
      { id: "profiles", label: "Profiles" },
    ],
  },
  {
    id: "review",
    label: "Review",
    sections: [
      { id: "requests", label: "Requests & bumps" },
      { id: "rules", label: "Rules" },
      { id: "mappings", label: "Mappings" },
    ],
  },
  {
    id: "sync",
    label: "Sync",
    sections: [
      { id: "schedule", label: "Schedule" },
      { id: "notifications", label: "Notifications" },
    ],
  },
  {
    id: "agents",
    label: "Agents",
    sections: [
      { id: "server", label: "Server" },
      { id: "permissions", label: "Permissions" },
      { id: "client", label: "Connect a client" },
      { id: "proposals", label: "Proposals" },
      { id: "activity", label: "Activity" },
    ],
  },
  {
    id: "about",
    label: "About",
    sections: [
      { id: "app", label: "App & updates" },
      { id: "changelog", label: "Changelog" },
      { id: "keychain", label: "Keychain" },
      { id: "setup", label: "Setup" },
    ],
  },
] as const;

export type SettingsSection = (typeof SETTINGS_GROUPS)[number]["sections"][number]["id"];

const SECTION_IDS: readonly SettingsSection[] = SETTINGS_GROUPS.flatMap((group) => group.sections.map((section) => section.id));

export function isSettingsSection(value: string): value is SettingsSection {
  return (SECTION_IDS as readonly string[]).includes(value);
}

export function settingsSectionLabel(id: SettingsSection): string {
  for (const group of SETTINGS_GROUPS) {
    for (const section of group.sections) {
      if (section.id === id) {
        return section.label;
      }
    }
  }
  return id;
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

/** Up and down, Home and End move between sections (vertical tabs). */
export function sectionAfterKey(current: SettingsSection, key: string): SettingsSection | null {
  const index = SECTION_IDS.indexOf(current);
  switch (key) {
    case "ArrowDown":
      return SECTION_IDS[(index + 1) % SECTION_IDS.length];
    case "ArrowUp":
      return SECTION_IDS[(index - 1 + SECTION_IDS.length) % SECTION_IDS.length];
    case "Home":
      return SECTION_IDS[0];
    case "End":
      return SECTION_IDS[SECTION_IDS.length - 1];
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
  slack_team_id: "team id",
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
