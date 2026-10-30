import { useEffect, useState } from "react";
import type { Api, McpSettings, McpView, SlackSendMode, SlackWorkspace } from "../api";
import { errorMessage } from "../api";
import { Switch } from "../Switch";
import { Tooltip } from "../Tooltip";
import { McpActivity } from "./McpActivity";
import { SlackProposals } from "./SlackProposals";
import { plainText } from "../textInput";
import { McpClientConfig } from "./McpClientConfig";
import { parsePort } from "./tabs";

interface Props {
  api: Pick<Api, "mcpSettings" | "saveMcpSettings" | "mcpToken" | "regenerateMcpToken" | "mcpClientStates" | "mcpAddToClient" | "mcpOpenClientConfig" | "mcpActivity" | "clearMcpActivity" | "slackWorkspaces" | "slackConnect" | "mcpSlackProposals" | "decideMcpSlackProposal">;
}

const button = "rounded border border-slate-700 px-3 py-1.5 text-sm hover:bg-slate-800 disabled:opacity-50";

/** Settings → Agents (MCP): the local server local AI agents connect to. */
export function AgentsSettings({ api }: Props) {
  const [view, setView] = useState<McpView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [port, setPort] = useState("");
  const [portError, setPortError] = useState<string | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [workspaces, setWorkspaces] = useState<SlackWorkspace[]>([]);
  const [reauthing, setReauthing] = useState<string | null>(null);

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
    api
      .slackWorkspaces()
      .then((list) => !cancel && setWorkspaces(list))
      .catch(() => undefined);
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

  function reauth(teamId: string) {
    setBusy(true);
    setError(null);
    setReauthing(teamId);
    api
      .slackConnect()
      .then(() => api.slackWorkspaces())
      .then((list) => setWorkspaces(list))
      .catch((err: unknown) => setError(errorMessage(err)))
      .finally(() => {
        setBusy(false);
        setReauthing(null);
      });
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
          Lets AI agents on this computer (Claude Code, Cursor, any MCP client) read and, if you allow it, organize your projects and pull requests. It listens
          on 127.0.0.1 only and every request needs the token below.
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
            <Switch
              label="Local writes"
              checked={settings.local_writes}
              disabled={busy}
              aria-describedby={settings.local_writes ? "agents-writes-note" : undefined}
              onChange={(local_writes) => save({ local_writes })}
            >
              Local writes <span className="text-slate-400">— create, edit and archive projects, link PRs and JIRA issues, set notes and stars</span>
            </Switch>
            {settings.local_writes && (
              <p id="agents-writes-note" className="mt-1 text-xs text-slate-400">
                Agents cannot delete projects. Every call shows under Activity below.
              </p>
            )}
          </li>
          <li>
            <Switch
              label="Slack reads"
              checked={settings.slack_reads}
              disabled={busy}
              aria-describedby="agents-slack-note"
              onChange={(slack_reads) => save({ slack_reads })}
            >
              Slack reads <span className="text-slate-400">— search and read public channels and your channels in the workspaces ticked below</span>
            </Switch>
            <p id="agents-slack-note" className="mt-1 max-w-prose text-xs text-slate-400">
              Each workspace answers through its own Slack sign-in. Slack replies reach agents as Slack returns them, marked as
              untrusted content.
            </p>
            {settings.slack_reads && (
              <SlackWorkspacePicker
                workspaces={workspaces}
                selected={settings.slack_workspaces}
                disabled={busy}
                reauthing={reauthing}
                onChange={(slack_workspaces) => save({ slack_workspaces })}
                onReauth={reauth}
              />
            )}
          </li>
          <li>
            <Switch
              label="Slack private search"
              checked={settings.slack_reads && settings.slack_private_search}
              disabled={busy || !settings.slack_reads}
              aria-describedby="agents-private-note"
              onChange={(slack_private_search) => save({ slack_private_search })}
            >
              Slack private search <span className="text-slate-400">— also search private channels, DMs and group DMs you are in</span>
            </Switch>
            <p id="agents-private-note" className="mt-1 max-w-prose text-xs text-slate-400">
              {settings.slack_reads ? "Off by default. Applies to the same ticked workspaces." : "Turn on Slack reads first."}
            </p>
          </li>
          <li>
            <Switch
              label="Slack sends"
              checked={settings.slack_reads && settings.slack_sends}
              disabled={busy || !settings.slack_reads}
              aria-describedby="agents-sends-note"
              onChange={(slack_sends) => save({ slack_sends })}
            >
              Slack sends <span className="text-slate-400">— agents send messages, drafts, schedules and reactions as you</span>
            </Switch>
            <p id="agents-sends-note" className="mt-1 max-w-prose text-xs text-slate-400">
              {settings.slack_reads
                ? "Each write names one workspace."
                : "Turn on Slack reads first."}
            </p>
            {settings.slack_reads && settings.slack_sends && (
              <>
                <SendModes mode={settings.slack_send_mode} disabled={busy} onChange={(slack_send_mode) => save({ slack_send_mode })} />
                {settings.slack_send_mode !== "automatic" && (
                  <label className="mt-2 ml-1 flex items-start gap-2 text-sm">
                    <input
                      type="checkbox"
                      className="mt-1"
                      checked={settings.slack_elicit}
                      disabled={busy}
                      onChange={(e) => save({ slack_elicit: e.target.checked })}
                    />
                    <span>
                      Ask in the agent when it supports it
                      <span className="block text-xs text-slate-400">A held write is shown in the agent to confirm; unanswered ones still wait here.</span>
                    </span>
                  </label>
                )}
                {settings.slack_send_mode === "automatic" && (
                  <p role="note" className="mt-1 ml-1 max-w-prose text-xs text-amber-300">
                    Agents&apos; messages are sent immediately as you, with no approval.
                  </p>
                )}
                {settings.slack_send_mode === "allowlist" && (
                  <AutoSendChannels channels={settings.slack_auto_send_channels} disabled={busy} onChange={(slack_auto_send_channels) => save({ slack_auto_send_channels })} />
                )}
              </>
            )}
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
          <McpClientConfig
            url={url}
            tokenMasked={view.token_masked}
            fixedPort={settings.port != null}
            api={api}
            tokenVersion={view.token_masked}
            onCopy={(label, build) => copy(label, api.mcpToken().then(build))}
          />
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

      {settings.slack_reads && <SlackProposals api={api} canSend={settings.slack_sends} />}

      <McpActivity api={api} />
    </div>
  );
}

interface PickerProps {
  workspaces: SlackWorkspace[];
  selected: string[];
  disabled: boolean;
  /** Team being signed in again, if any. */
  reauthing: string | null;
  onChange: (teamIds: string[]) => void;
  onReauth: (teamId: string) => void;
}

/** The Slack workspaces agents may see. None is ticked until the user picks. */
function SlackWorkspacePicker({ workspaces, selected, disabled, reauthing, onChange, onReauth }: PickerProps) {
  if (workspaces.length === 0) {
    return <p className="ml-11 mt-2 text-xs text-slate-400">No Slack workspace connected yet. Connect one in Settings → Slack.</p>;
  }
  const toggle = (teamId: string, on: boolean) =>
    onChange(on ? [...selected.filter((id) => id !== teamId), teamId] : selected.filter((id) => id !== teamId));
  return (
    <fieldset className="ml-11 mt-2">
      <legend className="text-xs text-slate-400">Workspaces agents may read</legend>
      <ul className="mt-1 space-y-1">
        {workspaces.map((ws) => {
          const unavailable = !ws.enabled ? "turned off in Settings → Slack" : ws.needs_sign_in ? "needs signing in again" : null;
          const missing = ws.scopes_missing ?? [];
          const noteId = `agents-scopes-${ws.team_id}`;
          return (
            <li key={ws.team_id} className="flex flex-wrap items-center gap-x-3">
              <label className="flex items-center gap-2">
                <input
                  type="checkbox"
                  className="accent-sky-500"
                  checked={selected.includes(ws.team_id)}
                  disabled={disabled}
                  onChange={(event) => toggle(ws.team_id, event.target.checked)}
                />
                <span>{ws.team_name}</span>
                <span className="text-xs text-slate-500">{ws.domain}.slack.com</span>
                {unavailable && <span className="text-xs text-amber-300">— {unavailable}</span>}
              </label>
              {!unavailable && missing.length > 0 && (
                <>
                  <Tooltip label="Missing permissions" detail={missing.join(", ")} align="start">
                    <span id={noteId} className="text-xs text-amber-300">
                      — needs new Slack permissions for reactions, files, canvases and lists
                    </span>
                  </Tooltip>
                  <button
                    type="button"
                    className="rounded px-2 py-0.5 text-xs text-sky-300 hover:bg-slate-800 disabled:opacity-50"
                    aria-label={`Sign in again to ${ws.team_name}`}
                    aria-describedby={noteId}
                    disabled={disabled}
                    onClick={() => onReauth(ws.team_id)}
                  >
                    {reauthing === ws.team_id ? "Waiting for Slack…" : "Sign in again"}
                  </button>
                </>
              )}
            </li>
          );
        })}
      </ul>
    </fieldset>
  );
}

const CHANNEL = /^[^\s/]+\/[CDG][A-Z0-9]+$/;

/** Channels where Slack writes go out without approval. */
function AutoSendChannels({ channels, disabled, onChange }: { channels: string[]; disabled: boolean; onChange: (next: string[]) => void }) {
  const [draft, setDraft] = useState("");
  const [invalid, setInvalid] = useState(false);
  function add() {
    const id = draft.trim();
    if (!CHANNEL.test(id)) {
      setInvalid(true);
      return;
    }
    setInvalid(false);
    setDraft("");
    if (!channels.includes(id)) {
      onChange([...channels, id]);
    }
  }
  return (
    <div className="mt-2 ml-1 space-y-1">
      <p className="text-xs text-slate-400">Auto-send channels {channels.length === 0 && "— none: every write waits for you"}</p>
      <ul aria-label="Auto-send channels" className="space-y-1">
        {channels.map((id) => (
          <li key={id} className="flex items-center gap-2">
            <span className="font-mono text-xs">{id}</span>
            <button type="button" disabled={disabled} aria-label={`Remove ${id}`} className="rounded px-1.5 text-xs hover:bg-slate-800" onClick={() => onChange(channels.filter((c) => c !== id))}>
              Remove
            </button>
          </li>
        ))}
      </ul>
      <form
        className="flex items-center gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          add();
        }}
      >
        <input
          aria-label="Add auto-send channel"
          aria-invalid={invalid}
          placeholder="workspace/C0123456789"
          value={draft}
          disabled={disabled}
          {...plainText}
          onChange={(event) => setDraft(event.target.value)}
          className="w-64 rounded border border-slate-700 bg-slate-900 px-2 py-1 font-mono text-xs"
        />
        <button type="submit" disabled={disabled || draft.trim() === ""} className={button}>
          Add
        </button>
      </form>
      {invalid && <p role="alert" className="text-xs text-rose-300">Use a qualified channel ID, as Slack tools return it: workspace/C…</p>}
    </div>
  );
}

const SEND_MODES: { mode: SlackSendMode; label: string; hint: string }[] = [
  { mode: "approve_all", label: "Approve everything", hint: "every write waits for you in the app" },
  { mode: "allowlist", label: "Channel allowlist", hint: "listed channels go out at once, unless the agent read another workspace first; others wait" },
  { mode: "automatic", label: "Fully automatic", hint: "every write goes out at once" },
];

function SendModes({ mode, disabled, onChange }: { mode: SlackSendMode; disabled: boolean; onChange: (mode: SlackSendMode) => void }) {
  return (
    <fieldset className="mt-2 ml-1 space-y-1 text-sm">
      <legend className="text-xs text-slate-400">Approval</legend>
      {SEND_MODES.map((m) => (
        <label key={m.mode} className="flex items-baseline gap-2">
          <input type="radio" name="slack-send-mode" value={m.mode} checked={mode === m.mode} disabled={disabled} onChange={() => onChange(m.mode)} />
          <span>
            {m.label} <span className="text-xs text-slate-400">— {m.hint}</span>
          </span>
        </label>
      ))}
    </fieldset>
  );
}
