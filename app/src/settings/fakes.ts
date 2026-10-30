import type { Api, McpActivityEntry, McpClientId, McpClientState, McpOutcome, McpSettings, McpView, Preferences, SlackProposal, SlackWorkspace } from "../api";

/** The §7.12 defaults, as the backend returns them on a fresh database. */
export const DEFAULT_PREFERENCES: Preferences = {
  review_requester: "",
  min_bump_interval_hours: 24,
  bulk_bump_max_threads: 3,
  bump_broadcast: true,
  blocked_review_required_is_ready: true,
  ticket_key_regex: "([A-Z][A-Z0-9]+-\\d+)",
  focused_interval_s: 180,
  unfocused_interval_s: 900,
  keep_closed_days: 7,
  live_updates: true,
  jira_base_url: "",
};

type SettingsApi = Pick<
  Api,
  | "loadPreferences"
  | "savePreferences"
  | "listMappings"
  | "saveMapping"
  | "resetMapping"
  | "previewMappingImport"
  | "applyMappingImport"
  | "exportMapping"
  | "appInfo"
  | "checkUpdate"
  | "slackWorkspaces"
  | "pickMappingFile"
  | "mcpSettings"
  | "saveMcpSettings"
  | "mcpToken"
  | "regenerateMcpToken"
  | "mcpClientStates"
  | "mcpAddToClient"
  | "mcpOpenClientConfig"
  | "mcpActivity"
  | "clearMcpActivity"
  | "mcpSlackProposals"
  | "decideMcpSlackProposal"
>;

export const FAKE_MCP_TOKEN = "pnx_0123456789abcdef0123456789abcdef0123456789abcdef0123456789ab7f3e";

/** An in-memory MCP server: enabling it "runs" on port 43117 unless a port is set. */
type FakeMcp = Pick<
  Api,
  | "mcpSettings"
  | "saveMcpSettings"
  | "mcpToken"
  | "regenerateMcpToken"
  | "mcpClientStates"
  | "mcpAddToClient"
  | "mcpOpenClientConfig"
  | "mcpActivity"
  | "clearMcpActivity"
  | "openExternal"
  | "slackWorkspaces"
  | "slackConnect"
  | "mcpSlackProposals"
  | "decideMcpSlackProposal"
>;

/** Made-up config paths under a fake home, one per client. */
export const FAKE_CLIENT_PATHS: Record<McpClientId, string> = {
  "claude-desktop": "~/Library/Application Support/Claude/claude_desktop_config.json",
  "claude-code": "~/.claude.json",
  cursor: "~/.cursor/mcp.json",
  vscode: "~/Library/Application Support/Code/User/mcp.json",
  codex: "~/.codex/config.toml",
  opencode: "~/.config/opencode/opencode.json",
};

/** Two made-up workspaces: `acme`, and `acme-labs` on an Enterprise Grid. */
export const FAKE_SLACK_WORKSPACES: SlackWorkspace[] = [
  { id: 1, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false, team_id: "T0ACME0001", enabled: true, priority: 0 },
  { id: 2, team_name: "Acme Labs", domain: "acme-labs", user_name: "ada", signed_in: true, needs_sign_in: false, team_id: "T0ACMELAB1", enabled: true, priority: 1 },
];

/** Settings screens other than Agents bring their own Slack sign-in fake. */
function withoutConnect(mcp: FakeMcp): Omit<FakeMcp, "slackConnect"> {
  const rest: Partial<FakeMcp> = { ...mcp };
  delete rest.slackConnect;
  return rest as Omit<FakeMcp, "slackConnect">;
}

/** A few minutes of an agent tidying up, newest first, relative to `now`. */
export function sampleActivity(now = new Date()): McpActivityEntry[] {
  const at = (minutes: number) => new Date(now.getTime() - minutes * 60_000).toISOString();
  const row = (id: number, minutes: number, tool: string, outcome: McpOutcome, summary: string, error: string | null = null): McpActivityEntry => ({
    id,
    at: at(minutes),
    client: "claude-code",
    tool,
    capability: tool.startsWith("slack_") ? "slack_reads" : tool.startsWith("list") || tool.startsWith("get") ? "reads" : "local_writes",
    outcome,
    summary,
    error,
  });
  return [
    row(7, 0, "set_pr_note", "allowed", "set note on acme/api#482"),
    row(6, 1, "set_pr_starred", "allowed", "starred acme/api#482"),
    row(5, 2, "add_prs_to_project", "allowed", "added 3 PRs to project 2"),
    row(4, 3, "remove_jira_link", "error", "removed a JIRA link from project 2", "Project 2 has no JIRA link ACME-9"),
    row(3, 26, "set_pr_starred", "denied", "starred PR id 482", "set_pr_starred is turned off in PR Nexus Settings → Agents (MCP)."),
    row(2, 27, "list_prs", "allowed", "listed pull requests"),
    row(1, 95, "list_projects", "allowed", "listed projects"),
  ];
}

/** Two Slack writes in `acme`: a message, then a reply in its thread. */
export function sampleSlackActivity(now = new Date()): McpActivityEntry[] {
  const at = (minutes: number) => new Date(now.getTime() - minutes * 60_000).toISOString();
  const base = { client: "claude-code", tool: "slack_send_message", capability: "slack_sends", outcome: "allowed" as const, error: null };
  return [
    {
      ...base,
      id: 12,
      at: at(0),
      summary: "replied in a thread in #release (auto-send)",
      link: "https://acme.slack.com/archives/C0123456789/p1700000500000200?thread_ts=1700000000.000100&cid=C0123456789",
    },
    { ...base, id: 11, at: at(4), summary: "sent a Slack message (auto-send)", link: "https://acme.slack.com/archives/C0123456789/p1700000000000100" },
    { ...base, id: 10, at: at(30), tool: "list_prs", capability: "reads", summary: "listed pull requests" },
  ];
}

export function fakeMcp(
  initial: Partial<McpSettings> = {},
  activity: McpActivityEntry[] = [],
  initialWorkspaces: SlackWorkspace[] = [],
  initialProposals: SlackProposal[] = [],
  initialClients: Partial<Record<McpClientId, McpClientState["status"]>> = {},
): FakeMcp {
  let clients = { ...initialClients };
  let log = [...activity];
  let proposals = [...initialProposals];
  let workspaces = [...initialWorkspaces];
  let settings: McpSettings = {
    enabled: false,
    reads: true,
    local_writes: false,
    slack_sends: false,
    slack_reads: false,
    slack_private_search: false,
    slack_workspaces: [],
    slack_auto_send_channels: [],
    slack_send_mode: "allowlist",
    slack_elicit: true,
    port: null,
    ...initial,
  };
  let token: string | null = settings.enabled ? FAKE_MCP_TOKEN : null;
  const view = (): McpView => {
    const port = settings.port ?? 43117;
    return {
      settings,
      status: settings.enabled
        ? { state: "running", url: `http://127.0.0.1:${port}/mcp`, port, error: null }
        : { state: "off", url: null, port: null, error: null },
      token_masked: token ? `pnx_••••••••••••${token.slice(-4)}` : null,
    };
  };
  const clientState = (client: McpClientId): McpClientState => {
    const status = clients[client] ?? "not_added";
    return { client, status, path: FAKE_CLIENT_PATHS[client], exists: status !== "not_added" };
  };
  return {
    mcpSettings: () => Promise.resolve(view()),
    slackWorkspaces: () => Promise.resolve(workspaces),
    // Signing in again grants every scope to the first workspace missing some.
    slackConnect: () => {
      const stale = workspaces.find((ws) => (ws.scopes_missing ?? []).length > 0) ?? workspaces[0];
      if (!stale) {
        return Promise.reject(new Error("No workspace"));
      }
      const fresh = { ...stale, scopes_missing: [] };
      workspaces = workspaces.map((ws) => (ws.team_id === fresh.team_id ? fresh : ws));
      return Promise.resolve(fresh);
    },
    saveMcpSettings: (next) => {
      settings = { ...next };
      token ??= settings.enabled ? FAKE_MCP_TOKEN : null;
      return Promise.resolve(view());
    },
    mcpToken: () => {
      token ??= FAKE_MCP_TOKEN;
      return Promise.resolve(token);
    },
    mcpClientStates: () =>
      settings.enabled
        ? Promise.resolve(
            (Object.keys(FAKE_CLIENT_PATHS) as McpClientId[]).map((client) => clientState(client)),
          )
        : Promise.reject(new Error("Turn the server on first")),
    mcpAddToClient: (client) => {
      if (clients[client] === "unreadable") {
        return Promise.reject(new Error(`${FAKE_CLIENT_PATHS[client]} is not plain JSON; left unchanged.`));
      }
      clients = { ...clients, [client]: "added" };
      return Promise.resolve(clientState(client));
    },
    mcpOpenClientConfig: () => Promise.resolve(),
    regenerateMcpToken: () => {
      token = FAKE_MCP_TOKEN.slice(0, -4) + "c0de";
      clients = Object.fromEntries(Object.entries(clients).map(([id, status]) => [id, status === "added" ? "outdated" : status]));
      return Promise.resolve(view());
    },
    openExternal: () => Promise.resolve(),
    mcpActivity: (outcome) => Promise.resolve(log.filter((entry) => !outcome || entry.outcome === outcome)),
    clearMcpActivity: () => {
      const removed = log.length;
      log = [];
      return Promise.resolve(removed);
    },
    mcpSlackProposals: () => Promise.resolve([...proposals].reverse()),
    decideMcpSlackProposal: (id, approve) => {
      const found = proposals.find((p) => p.id === id);
      if (!found || found.status !== "pending") {
        return Promise.reject(new Error(`Proposal ${id} is not waiting`));
      }
      const decided: SlackProposal = approve
        ? { ...found, status: "sent", result: { ok: true, ts: "1767225600.000100" }, decided_at: new Date().toISOString() }
        : { ...found, status: "rejected", decided_at: new Date().toISOString() };
      proposals = proposals.map((p) => (p.id === id ? decided : p));
      return Promise.resolve(decided);
    },
  };
}

/** In-memory Settings endpoints for tests and the browser preview. */
export function fakeSettingsApi(overrides: Partial<SettingsApi> = {}): SettingsApi {
  return {
    loadPreferences: () => Promise.resolve(DEFAULT_PREFERENCES),
    savePreferences: (prefs) => Promise.resolve(prefs),
    listMappings: () => Promise.resolve([]),
    saveMapping: () => Promise.resolve(),
    resetMapping: () => Promise.resolve(true),
    previewMappingImport: () => Promise.resolve({ added: [], changed: [], unchanged: 0, user_shadowed: [] }),
    applyMappingImport: () => Promise.resolve({ rows: 0 }),
    exportMapping: () => Promise.resolve(""),
    appInfo: () =>
      Promise.resolve({
        version: "0.1.0",
        repository: "acme/pr-nexus",
        db_path: "/Users/ada/Library/Application Support/pr-nexus/pr-nexus.db",
        log_dir: "/Users/ada/Library/Logs/pr-nexus",
        keyring_in_memory: false,
        os: "macos",
      }),
    checkUpdate: () => Promise.resolve(null),
    pickMappingFile: () => Promise.resolve(null),
    ...withoutConnect(fakeMcp()),
    slackWorkspaces: () => Promise.resolve([]),
    ...overrides,
  };
}
