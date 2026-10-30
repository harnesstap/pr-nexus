import { describe, expect, it } from "vitest";
import { MCP_CLIENTS, mcpClientSnippet } from "./mcpClients";

const URL = "http://127.0.0.1:47900/mcp";
const TOKEN = "pnx_example";

describe("mcpClientSnippet", () => {
  it("Claude Desktop: stdio bridge through mcp-remote", () => {
    const parsed = JSON.parse(mcpClientSnippet("claude-desktop", URL, TOKEN).text);
    const server = parsed.mcpServers["pr-nexus"];
    expect(server.command).toBe("npx");
    expect(server.args).toEqual(["-y", "mcp-remote", URL, "--transport", "http-only", "--header", "Authorization:${PR_NEXUS_AUTH}"]);
    expect(server.env).toEqual({ PR_NEXUS_AUTH: "Bearer pnx_example" });
    expect(server.url).toBeUndefined();
  });

  it("Claude Code: claude mcp add command", () => {
    expect(mcpClientSnippet("claude-code", URL, TOKEN).text).toBe(
      'claude mcp add --transport http pr-nexus http://127.0.0.1:47900/mcp --header "Authorization: Bearer pnx_example"',
    );
  });

  it("Cursor: mcpServers with url and headers", () => {
    expect(JSON.parse(mcpClientSnippet("cursor", URL, TOKEN).text)).toEqual({
      mcpServers: { "pr-nexus": { url: URL, headers: { Authorization: "Bearer pnx_example" } } },
    });
  });

  it("VS Code: top-level servers with type http", () => {
    expect(JSON.parse(mcpClientSnippet("vscode", URL, TOKEN).text)).toEqual({
      servers: { "pr-nexus": { type: "http", url: URL, headers: { Authorization: "Bearer pnx_example" } } },
    });
  });

  it("Codex: config.toml table with url and http_headers", () => {
    const lines = mcpClientSnippet("codex", URL, TOKEN).text.split("\n");
    expect(lines).toEqual([
      "[mcp_servers.pr-nexus]",
      'url = "http://127.0.0.1:47900/mcp"',
      'http_headers = { "Authorization" = "Bearer pnx_example" }',
    ]);
  });

  it("OpenCode: mcp entry of type remote", () => {
    const parsed = JSON.parse(mcpClientSnippet("opencode", URL, TOKEN).text);
    expect(parsed.mcp["pr-nexus"]).toEqual({
      type: "remote",
      url: URL,
      enabled: true,
      headers: { Authorization: "Bearer pnx_example" },
    });
  });

  it("every client says where the snippet goes", () => {
    for (const { id } of MCP_CLIENTS) {
      expect(mcpClientSnippet(id, URL, TOKEN).where).not.toBe("");
    }
  });
});
