import { useEffect, useRef, useState } from "react";
import type { Api, McpClientState } from "../api";
import { errorMessage } from "../api";
import { Tooltip } from "../Tooltip";
import { MCP_CLIENTS, isMcpClient, mcpClientSnippet, type McpClient } from "./mcpClients";

const CLIENT_KEY = "pr-nexus.mcp-client-tab";

/** Clients that only read their MCP config at launch. */
const NEEDS_RESTART: ReadonlySet<McpClient> = new Set(["claude-desktop", "cursor", "codex"]);

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
  /** Opens Server, where the port is set. Omit when that field is on this page. */
  onFixPort?: () => void;
  /** Copies the snippet built with the real token. */
  onCopy: (label: string, build: (token: string) => string) => void;
  /** Select this AI agent when a notice opens the page. */
  focusClient?: McpClient | null;
  /** The agent's config was written. */
  onChanged?: () => void;
}

const STATUS_TEXT: Record<McpClientState["status"], string> = {
  not_added: "Not added",
  added: "Added",
  outdated: "Added, but outdated",
  unreadable: "Can't read the config file. Fix it, or copy the snippet.",
};

/** "Add to your agent" in Settings → MCP server: one tab per AI agent, each with its own snippet. */
export function McpClientConfig({ api, url, tokenMasked, tokenVersion, fixedPort, onFixPort, onCopy, focusClient, onChanged }: Props) {
  const [client, setClient] = useState<McpClient>(readClient);
  const [states, setStates] = useState<McpClientState[]>([]);
  const inflight = useRef(new Set<McpClient>());
  const [pending, setPending] = useState<ReadonlySet<McpClient>>(() => new Set());
  const [error, setError] = useState<string | null>(null);
  const [restartFor, setRestartFor] = useState<McpClient | null>(null);
  const [appliedFocus, setAppliedFocus] = useState<McpClient | null>(null);
  const label = MCP_CLIENTS.find((c) => c.id === client)?.label ?? client;
  const state = states.find((s) => s.client === client);
  const found = state?.exists ? state.display_path : undefined;
  const shown = mcpClientSnippet(client, url, tokenMasked, found);

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

  if (focusClient && focusClient !== appliedFocus) {
    setAppliedFocus(focusClient);
    setClient(focusClient);
    writeClient(focusClient);
    setError(null);
  }

  function select(next: McpClient) {
    setClient(next);
    writeClient(next);
    setError(null);
  }

  function markPending(target: McpClient, on: boolean) {
    if (on) {
      inflight.current.add(target);
    } else {
      inflight.current.delete(target);
    }
    setPending(new Set(inflight.current));
  }

  function add() {
    const target = client;
    if (inflight.current.has(target)) {
      return;
    }
    markPending(target, true);
    setError(null);
    api
      .mcpAddToClient(target)
      .then((next) => {
        setStates((prev) => [...prev.filter((item) => item.client !== next.client), next]);
        setRestartFor(NEEDS_RESTART.has(next.client) ? next.client : null);
        onChanged?.();
      })
      .catch((err: unknown) => setError(errorMessage(err)))
      .finally(() => markPending(target, false));
  }

  function open() {
    setError(null);
    api.mcpOpenClientConfig(client).catch((err: unknown) => setError(errorMessage(err)));
  }

  const current = state?.status === "added";
  const adding = pending.has(client);
  const addLabel = current ? "Added ✓" : state?.status === "outdated" ? `Update ${label}` : `Add to ${label}`;

  return (
    <div className="mt-3">
      <div className="flex items-center justify-between">
        <span className="text-slate-400">AI agent</span>
        <button
          type="button"
          aria-label="Copy config"
          className="rounded px-2 py-0.5 text-xs hover:bg-slate-800"
          onClick={() => onCopy(`${label} config`, (token) => mcpClientSnippet(client, url, token, found).text)}
        >
          Copy with token
        </button>
      </div>
      <div role="tablist" aria-label="AI agent" className="mt-1 flex flex-wrap gap-1">
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
              disabled={adding || current || state.status === "unreadable"}
              onClick={add}
            >
              {adding ? "Adding…" : addLabel}
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
            The port is picked at random, so this config breaks when it changes.{" "}
            {onFixPort ? (
              <>
                Set a fixed port on{" "}
                <button type="button" className="text-sky-300 hover:underline" onClick={onFixPort}>
                  Server
                </button>
                .
              </>
            ) : (
              "Set a fixed port above."
            )}
          </p>
        )}
      </div>
    </div>
  );
}
