import { useEffect, useState } from "react";
import type { Api, McpSettings, McpView } from "../api";
import { errorMessage } from "../api";
import { Switch } from "../Switch";
import { Tooltip } from "../Tooltip";
import { plainText } from "../textInput";
import { mcpConfigSnippet, parsePort } from "./tabs";

interface Props {
  api: Pick<Api, "mcpSettings" | "saveMcpSettings" | "mcpToken" | "regenerateMcpToken">;
}

const button = "rounded border border-slate-700 px-3 py-1.5 text-sm hover:bg-slate-800 disabled:opacity-50";
const SOON = "Coming soon";

/** Settings → Agents (MCP): the local server local AI agents connect to. */
export function AgentsSettings({ api }: Props) {
  const [view, setView] = useState<McpView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [port, setPort] = useState("");
  const [portError, setPortError] = useState<string | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);

  useEffect(() => {
    let cancel = false;
    api
      .mcpSettings()
      .then((next) => {
        if (!cancel) {
          setView(next);
          setPort(next.settings.port?.toString() ?? "");
        }
      })
      .catch((err: unknown) => !cancel && setError(errorMessage(err)));
    return () => {
      cancel = true;
    };
  }, [api]);

  function run(work: () => Promise<McpView>) {
    setBusy(true);
    setError(null);
    work()
      .then((next) => setView(next))
      .catch((err: unknown) => setError(errorMessage(err)))
      .finally(() => setBusy(false));
  }

  function save(patch: Partial<McpSettings>) {
    if (view) {
      run(() => api.saveMcpSettings({ ...view.settings, ...patch }));
    }
  }

  function applyPort() {
    const parsed = parsePort(port);
    if ("error" in parsed) {
      setPortError(parsed.error);
      return;
    }
    setPortError(null);
    if (parsed.port !== view?.settings.port) {
      save({ port: parsed.port });
    }
  }

  function copy(label: string, text: Promise<string> | string) {
    void Promise.resolve(text)
      .then((value) => navigator.clipboard?.writeText(value))
      .then(() => setCopied(label))
      .catch((err: unknown) => setError(errorMessage(err)));
  }

  if (!view) {
    return error ? (
      <p role="alert" className="text-sm text-rose-300">
        Could not read the agent settings: {error}
      </p>
    ) : (
      <p className="text-sm text-slate-400">Loading…</p>
    );
  }

  const { settings, status } = view;
  const url = status.url;
  return (
    <div className="space-y-8 text-sm">
      <section aria-labelledby="agents-server">
        <h2 id="agents-server" className="text-base font-semibold">
          Local MCP server
        </h2>
        <p className="mt-2 max-w-prose text-slate-300">
          Lets AI agents on this computer (Claude Code, Cursor, any MCP client) read your projects and pull requests. It listens on 127.0.0.1 only and every
          request needs the token below.
        </p>
        <div className="mt-3 flex flex-wrap items-center gap-x-4 gap-y-2">
          <Switch label="Enable MCP server" checked={settings.enabled} disabled={busy} onChange={(enabled) => save({ enabled })}>
            Enable MCP server
          </Switch>
          <span role="status" className="text-xs">
            {status.state === "running" && <span className="text-emerald-300">● Running on port {status.port}</span>}
            {status.state === "off" && <span className="text-slate-400">○ Off</span>}
            {status.state === "error" && <span className="text-rose-300">Not running: {status.error}</span>}
          </span>
        </div>
        <div className="mt-3 flex flex-wrap items-end gap-2">
          <label className="block">
            <span className="text-slate-400">Port</span>
            <input
              {...plainText}
              className="mt-1 block w-36 rounded border border-slate-700 bg-slate-900 px-2 py-1"
              inputMode="numeric"
              placeholder="Any free port"
              value={port}
              aria-invalid={portError ? true : undefined}
              aria-describedby={portError ? "agents-port-error" : undefined}
              onChange={(event) => setPort(event.target.value)}
              onKeyDown={(event) => event.key === "Enter" && applyPort()}
            />
          </label>
          <button type="button" className={button} disabled={busy} onClick={applyPort}>
            Apply port
          </button>
        </div>
        {portError && (
          <p id="agents-port-error" role="alert" className="mt-1 text-xs text-rose-300">
            {portError}
          </p>
        )}
      </section>

      <section aria-labelledby="agents-capabilities">
        <h2 id="agents-capabilities" className="text-base font-semibold">
          What agents may do
        </h2>
        <ul className="mt-3 space-y-2">
          <li>
            <Switch label="Reads" checked={settings.reads} disabled={busy} onChange={(reads) => save({ reads })}>
              Reads <span className="text-slate-400">— list and read projects, pull requests, notes and stars</span>
            </Switch>
          </li>
          <li>
            <Tooltip label={SOON} detail="Edit projects, notes and stars from an agent." align="start">
              <Switch label="Local writes" checked={false} disabled onChange={() => undefined}>
                Local writes <span className="text-slate-500">— coming soon</span>
              </Switch>
            </Tooltip>
          </li>
          <li>
            <Tooltip label={SOON} detail="Send Slack review requests after you approve a preview." align="start">
              <Switch label="Slack sends" checked={false} disabled onChange={() => undefined}>
                Slack sends <span className="text-slate-500">— coming soon</span>
              </Switch>
            </Tooltip>
          </li>
        </ul>
      </section>

      <section aria-labelledby="agents-connect">
        <h2 id="agents-connect" className="text-base font-semibold">
          Connect an agent
        </h2>
        <dl className="mt-3 space-y-2">
          <div>
            <dt className="text-slate-400">Endpoint</dt>
            <dd className="flex min-w-0 items-center gap-2">
              <span className="min-w-0 truncate font-mono text-xs">{url ?? "Starts when the server is on"}</span>
              {url && (
                <button
                  type="button"
                  aria-label="Copy endpoint"
                  className="shrink-0 rounded px-2 py-0.5 text-xs hover:bg-slate-800"
                  onClick={() => copy("endpoint", url)}
                >
                  Copy
                </button>
              )}
            </dd>
          </div>
          <div>
            <dt className="text-slate-400">Token</dt>
            <dd className="flex min-w-0 items-center gap-2">
              <span className="min-w-0 truncate font-mono text-xs" aria-label="Token, masked">
                {view.token_masked ?? "Created when the server first starts"}
              </span>
              {view.token_masked && (
                <button
                  type="button"
                  aria-label="Copy token"
                  className="shrink-0 rounded px-2 py-0.5 text-xs hover:bg-slate-800"
                  onClick={() => copy("token", api.mcpToken())}
                >
                  Copy
                </button>
              )}
            </dd>
          </div>
        </dl>
        {url && view.token_masked && (
          <div className="mt-3">
            <div className="flex items-center justify-between">
              <span className="text-slate-400">Client config</span>
              <button
                type="button"
                aria-label="Copy client config"
                className="rounded px-2 py-0.5 text-xs hover:bg-slate-800"
                onClick={() =>
                  copy(
                    "client config",
                    api.mcpToken().then((token) => mcpConfigSnippet(url, token)),
                  )
                }
              >
                Copy with token
              </button>
            </div>
            <pre className="mt-1 overflow-x-auto rounded border border-slate-800 bg-slate-900/60 p-3 font-mono text-xs text-slate-300">
              {mcpConfigSnippet(url, view.token_masked)}
            </pre>
          </div>
        )}
        {copied && (
          <p role="status" className="mt-1 text-xs text-slate-300">
            Copied {copied}.
          </p>
        )}
        <div className="mt-4">
          {confirming ? (
            <div
              role="group"
              aria-label="Confirm new token"
              className="flex flex-wrap items-center gap-2 rounded border border-amber-700/60 bg-amber-950/30 p-2"
            >
              <span className="text-amber-100">Agents using the current token stop working until you give them the new one.</span>
              <button
                type="button"
                className={`${button} border-amber-600`}
                disabled={busy}
                onClick={() => {
                  setConfirming(false);
                  run(() => api.regenerateMcpToken());
                }}
              >
                Regenerate token
              </button>
              <button type="button" className={button} onClick={() => setConfirming(false)}>
                Cancel
              </button>
            </div>
          ) : (
            <button type="button" className={button} disabled={busy || !view.token_masked} onClick={() => setConfirming(true)}>
              Regenerate token…
            </button>
          )}
        </div>
        {error && (
          <p role="alert" className="mt-2 text-rose-300">
            {error}
          </p>
        )}
      </section>
    </div>
  );
}
