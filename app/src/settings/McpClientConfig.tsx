import { useState } from "react";
import { MCP_CLIENTS, isMcpClient, mcpClientSnippet, type McpClient } from "./mcpClients";

const CLIENT_KEY = "pr-nexus.mcp-client-tab";

function readClient(): McpClient {
  try {
    const stored = window.localStorage.getItem(CLIENT_KEY);
    return isMcpClient(stored) ? stored : "claude-desktop";
  } catch {
    return "claude-desktop";
  }
}

function writeClient(client: McpClient) {
  try {
    window.localStorage.setItem(CLIENT_KEY, client);
  } catch {
    // Private mode or a full disk: the first tab comes back next launch.
  }
}

interface Props {
  url: string;
  tokenMasked: string;
  /** False when the server picks any free port: the snippet breaks once it moves. */
  fixedPort: boolean;
  /** Copies the snippet built with the real token. */
  onCopy: (label: string, build: (token: string) => string) => void;
}

/** "Client config" in Settings → Agents: one tab per MCP client, each with its own snippet. */
export function McpClientConfig({ url, tokenMasked, fixedPort, onCopy }: Props) {
  const [client, setClient] = useState<McpClient>(readClient);
  const shown = mcpClientSnippet(client, url, tokenMasked);
  const label = MCP_CLIENTS.find((c) => c.id === client)?.label ?? client;

  function select(next: McpClient) {
    setClient(next);
    writeClient(next);
  }

  return (
    <div className="mt-3">
      <div className="flex items-center justify-between">
        <span className="text-slate-400">Client config</span>
        <button
          type="button"
          aria-label="Copy client config"
          className="rounded px-2 py-0.5 text-xs hover:bg-slate-800"
          onClick={() => onCopy(`${label} config`, (token) => mcpClientSnippet(client, url, token).text)}
        >
          Copy with token
        </button>
      </div>
      <div role="tablist" aria-label="MCP client" className="mt-1 flex flex-wrap gap-1">
        {MCP_CLIENTS.map((c) => (
          <button
            key={c.id}
            type="button"
            role="tab"
            id={`mcp-client-${c.id}`}
            aria-selected={c.id === client}
            aria-controls="mcp-client-panel"
            className={`rounded px-2 py-0.5 text-xs ${
              c.id === client ? "bg-slate-700 text-slate-100" : "text-slate-400 hover:bg-slate-800"
            }`}
            onClick={() => select(c.id)}
          >
            {c.label}
          </button>
        ))}
      </div>
      <div id="mcp-client-panel" role="tabpanel" aria-labelledby={`mcp-client-${client}`}>
        <p className="mt-1 text-xs text-slate-400">{shown.where}</p>
        <pre className="mt-1 overflow-x-auto rounded border border-slate-800 bg-slate-900/60 p-3 font-mono text-xs text-slate-300">
          {shown.text}
        </pre>
        {!fixedPort && (
          <p className="mt-1 text-xs text-amber-200">
            The port is picked at random, so this config breaks when it changes. Set a fixed port above.
          </p>
        )}
      </div>
    </div>
  );
}
