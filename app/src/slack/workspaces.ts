import type { SlackWorkspace } from "../api";

/** The last workspace picked for a mapping import, per machine like the banner flags. */
const LAST_KEY = "pr-nexus.mapping-workspace";

/**
 * Mappings store the workspace as its domain (`acme` for acme.slack.com),
 * which is what review targets match against. Older rows may hold the host.
 */
export function workspaceDomain(value: string): string {
  if (isTeamId(value)) {
    return value.trim().toUpperCase();
  }
  return value
    .trim()
    .replace(/^https?:\/\//i, "")
    .replace(/\/.*$/, "")
    .replace(/\.slack\.com$/i, "")
    .toLowerCase();
}

/** A Slack team ID (`T…`, or `E…` for a grid), which routes exactly. */
export function isTeamId(value: string): boolean {
  return /^[TE][A-Z0-9]{8,}$/.test(value.trim().toUpperCase()) && !/[.:/]/.test(value);
}

/** The name shown for a target or request: the team name, else the domain. */
export function targetWorkspaceName(target: { workspace: string; workspace_name?: string }): string {
  return target.workspace_name || target.workspace;
}

export function workspaceLabel(workspace: SlackWorkspace): string {
  const host = `${workspace.domain}.slack.com`;
  const name = workspace.team_name && workspace.team_name !== workspace.domain ? `${workspace.team_name} (${host})` : host;
  return workspace.needs_sign_in ? `${name} · needs sign-in` : name;
}

/** The connected workspace whose domain matches `value`, if any. */
export function findWorkspace(workspaces: SlackWorkspace[], value: string): SlackWorkspace | undefined {
  if (isTeamId(value)) {
    const team = value.trim().toUpperCase();
    return workspaces.find((item) => item.team_id === team);
  }
  const domain = workspaceDomain(value);
  return domain ? workspaces.find((item) => item.domain.toLowerCase() === domain) : undefined;
}

/**
 * The select's value: the user's pick while it is still connected, else the
 * only workspace, else the last one used, else the first. Empty when none.
 */
export function chosenWorkspace(workspaces: SlackWorkspace[], picked: string, last: string | null = lastWorkspace()): string {
  return (findWorkspace(workspaces, picked) ?? findWorkspace(workspaces, last ?? "") ?? workspaces[0])?.domain ?? "";
}

export function lastWorkspace(): string | null {
  try {
    return window.localStorage.getItem(LAST_KEY);
  } catch {
    return null;
  }
}

export function rememberWorkspace(domain: string) {
  if (!domain) {
    return;
  }
  try {
    window.localStorage.setItem(LAST_KEY, domain);
  } catch {
    // Private mode or a full disk: the first workspace is preselected next time.
  }
}

/** Whether the CSV's header row carries a `slack_workspace` column. */
export function csvHasWorkspaceColumn(csv: string): boolean {
  const header = csv.trimStart().split(/\r?\n/, 1)[0] ?? "";
  return header
    .split(",")
    .some((cell) => cell.trim().replace(/^"|"$/g, "").trim().toLowerCase() === "slack_workspace");
}
