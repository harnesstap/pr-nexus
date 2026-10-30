/** Per-client MCP config snippets for Settings → Agents → Connect an agent. Pure: no DOM, no IPC. */

export const MCP_CLIENTS = [
  { id: "claude-desktop", label: "Claude Desktop" },
  { id: "claude-code", label: "Claude Code" },
  { id: "cursor", label: "Cursor" },
  { id: "vscode", label: "VS Code" },
  { id: "codex", label: "Codex" },
  { id: "opencode", label: "OpenCode" },
] as const;

export type McpClient = (typeof MCP_CLIENTS)[number]["id"];

export interface McpClientSnippet {
  /** Text to show (masked token) or copy (real token). */
  text: string;
  /** One line: which file or command it goes into. */
  where: string;
}

const NAME = "pr-nexus";

function json(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

/** TOML basic string: JSON string escaping is a valid subset for our inputs. */
function tomlString(value: string): string {
  return JSON.stringify(value);
}

export function mcpClientSnippet(client: McpClient, url: string, token: string): McpClientSnippet {
  const bearer = `Bearer ${token}`;
  switch (client) {
    case "claude-desktop":
      return {
        where:
          "Merge into ~/Library/Application Support/Claude/claude_desktop_config.json (Windows: %APPDATA%\\Claude\\claude_desktop_config.json), then restart Claude Desktop. Needs Node.js (npx) for the mcp-remote bridge.",
        text: json({
          mcpServers: {
            [NAME]: {
              command: "npx",
              args: ["-y", "mcp-remote", url, "--transport", "http-only", "--header", "Authorization:${PR_NEXUS_AUTH}"],
              env: { PR_NEXUS_AUTH: bearer },
            },
          },
        }),
      };
    case "claude-code":
      return {
        where: "Run in a terminal. Add --scope user to use it in every project.",
        text: `claude mcp add --transport http ${NAME} ${url} --header "Authorization: ${bearer}"`,
      };
    case "cursor":
      return {
        where: "Merge into ~/.cursor/mcp.json (or .cursor/mcp.json in a project).",
        text: json({ mcpServers: { [NAME]: { url, headers: { Authorization: bearer } } } }),
      };
    case "vscode":
      return {
        where: "Merge into .vscode/mcp.json in a workspace, or the user mcp.json (MCP: Open User Configuration).",
        text: json({ servers: { [NAME]: { type: "http", url, headers: { Authorization: bearer } } } }),
      };
    case "codex":
      return {
        where: "Append to ~/.codex/config.toml.",
        text: [
          `[mcp_servers.${NAME}]`,
          `url = ${tomlString(url)}`,
          `http_headers = { "Authorization" = ${tomlString(bearer)} }`,
        ].join("\n"),
      };
    case "opencode":
      return {
        where: "Merge into opencode.json (project root, or ~/.config/opencode/opencode.json).",
        text: json({
          $schema: "https://opencode.ai/config.json",
          mcp: { [NAME]: { type: "remote", url, enabled: true, headers: { Authorization: bearer } } },
        }),
      };
  }
}

export function isMcpClient(value: string | null): value is McpClient {
  return MCP_CLIENTS.some((client) => client.id === value);
}
