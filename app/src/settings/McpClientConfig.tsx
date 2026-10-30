import { useEffect, useState } from "react";
import type { Api, McpClientState } from "../api";
import { errorMessage } from "../api";
import { Tooltip } from "../Tooltip";
import { MCP_CLIENTS, isMcpClient, mcpClientSnippet, type McpClient } from "./mcpClients";

const CLIENT_KEY = "pr-nexus.mcp-client-tab";

/** Clients that only read their MCP config at launch. */
const NEEDS_RESTART: ReadonlySet<McpClient> = new Set(["claude-desktop", "cursor", "codex", "opencode"]);

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
  api: Pick<Api, "mcpClientStates" | "mcpAddToClient" | "mcpOpenClientConfig">;
  url: string;
  tokenMasked: string;
  /** Changes when the token is regenerated, so the statuses are read again. */
  tokenVersion: string;
  /** False when the server picks any free port: the snippet breaks once it moves. */
  fixedPort: boolean;
  /** Copies the snippet built with the real token. */
  onCopy: (label: string, build: (token: string) => string) => void;
}

const STATUS_TEXT: Record<McpClientState["status"], string> = {
  not_added: "Not added",
  added: "Added",
  outdated: "Added, but outdated",
  unreadable: "Config file can't be read; copy the snippet instead",
};

/** "Client config" in Settings → Agents: one tab per MCP client, each with its own snippet. */
export function McpClientConfig({ api, url, tokenMasked, tokenVersion, fixedPort, onCopy }: Props) {
  const [client, setClient] = useState<McpClient>(readClient);
  const [states, setStates] = useState<McpClientState[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [restartFor, setRestartFor] = useState<McpClient | null>(null);
  const shown = mcpClientSnippet(client, url, tokenMasked);
  const label = MCP_CLIENTS.find((c) => c.id === client)?.label ?? client;
  const state = states.find((s) => s.client === client);

  useEffect(() => {
    let live = true;
    api
      .mcpClientStates()
      .then((next) => live && setStates(next))
      .catch(() => live && setStates([]));
    return () => {
      live = false;
    };
  }, [api, url, tokenVersion]);

  function select(next: McpClient) {
    setClient(next);
    writeClient(next);
    setError(null);
  }

  function add() {
    setBusy(true);
    setError(null);
    api
      .mcpAddToClient(client)
      .then((next) => {
        setStates((prev) => [...prev.filter((s) => s.client !== next.client), next]);
        setRestartFor(NEEDS_RESTART.has(next.client) ? next.client : null);
      })
      .catch((err: unknown) => setError(errorMessage(err)))
      .finally(() => setBusy(false));
  }

  function open() {
    setError(null);
    api.mcpOpenClientConfig(client).catch((err: unknown) => setError(errorMessage(err)));
  }

  const current = state?.status === "added";
  const addLabel = current ? "Added ✓" : state?.status === "outdated" ? `Update ${label}` : `Add to ${label}`;

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
        {state && (
          <div className="mt-2 flex flex-wrap items-center gap-2 text-xs">
            <button
              type="button"
              className="rounded border border-slate-700 px-2 py-0.5 hover:bg-slate-800 disabled:opacity-60 disabled:hover:bg-transparent"
              disabled={busy || current || state.status === "unreadable"}
              onClick={add}
            >
              {busy ? "Adding…" : addLabel}
            </button>
            <Tooltip label={state.exists ? state.path : `${state.path} does not exist yet. Opens its folder.`}>
              <button type="button" className="rounded px-2 py-0.5 text-sky-300 hover:bg-slate-800" onClick={open}>
                {state.exists ? "Open config file" : "Open folder"}
              </button>
            </Tooltip>
            <span role="status" aria-label={`${label} status`} className={state.status === "added" ? "text-emerald-300" : state.status === "not_added" ? "text-slate-400" : "text-amber-200"}>
              {STATUS_TEXT[state.status]}
            </span>
          </div>
        )}
        {restartFor === client && current && <p className="mt-1 text-xs text-slate-300">Restart {label} to load it.</p>}
        {error && (
          <p role="alert" className="mt-1 text-xs text-rose-300">
            {error}
          </p>
        )}
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
