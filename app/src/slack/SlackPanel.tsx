import { useEffect, useState } from "react";
import type { Api, SlackWorkspace } from "../api";
import { errorMessage } from "../api";
import { plainText } from "../textInput";
import { Switch } from "../Switch";
import { Tooltip } from "../Tooltip";

export const DEFAULT_REDIRECT = "http://127.0.0.1:47821/mcp/oauth/callback";

/** Mirrors `CALLBACK_TIMEOUT` in the providers crate (§7.11 step 3). */
export const SIGN_IN_TIMEOUT_SECONDS = 600;
/** After this many seconds the status switches to "Still waiting". */
const STILL_WAITING_AFTER_SECONDS = 30;

function formatLeft(seconds: number): string {
  const clamped = Math.max(0, seconds);
  return `${Math.floor(clamped / 60)}:${String(clamped % 60).padStart(2, "0")}`;
}

function errorKind(err: unknown): unknown {
  return err && typeof err === "object" && "kind" in err ? (err as { kind?: unknown }).kind : null;
}

export interface SlackFields {
  clientId: string;
  clientSecret: string;
  redirectUri: string;
}

interface Props {
  api: Api;
  fields: SlackFields;
  onFieldsChange: (fields: SlackFields) => void;
  /** A client secret is already in the keychain for this client ID. */
  secretSaved: boolean;
  workspaces: SlackWorkspace[];
  onWorkspacesChange: (workspaces: SlackWorkspace[]) => void;
  /** The client ID already saved, so sign-in works while the field is untouched. */
  savedClientId?: string;
}

/**
 * Slack app fields, connected workspaces, and browser sign-in (§7.11). Shared
 * by wizard step 5 and Settings. Connect saves the fields first, so the
 * redirect URI typed here is the one the loopback listener binds.
 */
export function SlackPanel({ api, fields, onFieldsChange, secretSaved, workspaces, onWorkspacesChange, savedClientId = "" }: Props) {
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [timedOut, setTimedOut] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [dragging, setDragging] = useState<number | null>(null);
  const [over, setOver] = useState<number | null>(null);
  const set = (patch: Partial<SlackFields>) => onFieldsChange({ ...fields, ...patch });

  useEffect(() => {
    if (!connecting) {
      return;
    }
    const started = Date.now();
    const timer = window.setInterval(() => setElapsed(Math.floor((Date.now() - started) / 1000)), 1000);
    return () => window.clearInterval(timer);
  }, [connecting]);

  async function openManifest() {
    setError(null);
    try {
      const url = await api.slackManifestUrl(fields.redirectUri);
      await api.openExternal(url);
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  /** Signs in; `expected` is the workspace row being reconnected, if any. */
  async function connect(expected?: SlackWorkspace) {
    setError(null);
    setNotice(null);
    setTimedOut(false);
    setElapsed(0);
    setConnecting(true);
    try {
      await api.saveSlack(fields.clientId.trim() || savedClientId.trim(), fields.clientSecret, fields.redirectUri);
      const workspace = await api.slackConnect();
      onWorkspacesChange([...workspaces.filter((item) => item.id !== workspace.id), workspace]);
      setNotice(
        expected && expected.id !== workspace.id
          ? `Connected to ${workspace.team_name}. ${expected.team_name} still needs sign-in: pick it in the Slack browser page.`
          : `${expected ? "Reconnected" : "Connected"} to ${workspace.team_name}.`,
      );
    } catch (err) {
      const kind = errorKind(err);
      if (kind === "timeout") {
        setTimedOut(true);
      } else if (kind !== "cancelled") {
        setError(errorMessage(err));
      }
    } finally {
      setConnecting(false);
    }
  }

  async function remove(workspace: SlackWorkspace) {
    setError(null);
    try {
      await api.slackRemoveWorkspace(workspace.id);
      onWorkspacesChange(await api.slackWorkspaces());
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  async function setEnabled(workspace: SlackWorkspace, enabled: boolean) {
    setError(null);
    try {
      onWorkspacesChange(await api.slackSetWorkspaceEnabled(workspace.team_id, enabled));
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  /** Moves `from` to `to` in the priority list and saves the new order. */
  async function move(from: number, to: number) {
    if (to < 0 || to >= ordered.length || from === to) {
      return;
    }
    const next = [...ordered];
    const [item] = next.splice(from, 1);
    next.splice(to, 0, item);
    setError(null);
    onWorkspacesChange(next.map((workspace, priority) => ({ ...workspace, priority })));
    try {
      onWorkspacesChange(await api.slackSetWorkspaceOrder(next.map((workspace) => workspace.team_id)));
    } catch (err) {
      setError(errorMessage(err));
      onWorkspacesChange(ordered);
    }
  }

  const ordered = [...workspaces].sort((a, b) => a.priority - b.priority);
  const several = ordered.length > 1;
  const clientId = fields.clientId.trim() || savedClientId.trim();
  const canSignIn = !connecting && clientId !== "";
  const input = "mt-1 w-full rounded border border-slate-700 bg-slate-900 px-3 py-2";
  const appFields = (
    <div className="space-y-4">
      <button type="button" className="text-sm text-violet-300 underline" onClick={() => void openManifest()}>
        Create your Slack app
      </button>
      <p className="text-sm text-slate-400">
        The link opens Slack with the PR Nexus manifest (name, user scopes, redirect URI, MCP on). No client id ships
        with the app. After creating it, copy the client ID (and secret) from Basic Information.
      </p>
      <label className="block text-sm">
        Client ID
        <input {...plainText}
          className={input}
          value={fields.clientId}
          disabled={connecting}
          onChange={(event) => set({ clientId: event.target.value })}
        />
      </label>
      <label className="block text-sm">
        Client secret
        <input {...plainText}
          className={input}
          type="password"
          placeholder={secretSaved ? "saved in the keychain · type to replace" : "optional · keychain"}
          value={fields.clientSecret}
          disabled={connecting}
          onChange={(event) => set({ clientSecret: event.target.value })}
        />
        <span className="mt-0.5 block text-xs text-slate-400">
          Needed for apps created from the link. Leave empty only for an app with PKCE enabled.
        </span>
      </label>
      <label className="block text-sm">
        Redirect URI
        <input {...plainText}
          className={input}
          value={fields.redirectUri}
          disabled={connecting}
          placeholder={DEFAULT_REDIRECT}
          onChange={(event) => set({ redirectUri: event.target.value })}
        />
        <span className="mt-0.5 block text-xs text-slate-400">
          Must match a redirect URL registered on the Slack app. PR Nexus listens on this loopback address during
          sign-in.
        </span>
      </label>
    </div>
  );
  return (
    <div className="space-y-4">
      <div>
        <h3 className="text-sm font-medium">Connected workspaces</h3>
        {several && (
          <p className="mt-0.5 text-xs text-slate-400">
            Sends try workspace 1 first, then the next enabled one when Slack says the channel or person is not there.
            Drag the handle or use the arrows to reorder.
          </p>
        )}
        {workspaces.length === 0 ? (
          <p className="mt-1 text-sm text-slate-400">None yet.</p>
        ) : (
          <ul className="mt-2 space-y-2" aria-label="Connected workspaces">
            {ordered.map((workspace, index) => {
              const name = workspace.team_name || workspace.domain;
              const stale = workspace.needs_sign_in || !workspace.signed_in;
              return (
                <li
                  key={workspace.id}
                  aria-label={workspace.team_name}
                  data-state={!workspace.enabled ? "disabled" : stale ? "needs-sign-in" : "connected"}
                  draggable={several}
                  onDragStart={(event) => {
                    setDragging(index);
                    event.dataTransfer?.setData("text/plain", workspace.team_id);
                  }}
                  onDragOver={(event) => {
                    if (dragging !== null) {
                      event.preventDefault();
                      setOver(index);
                    }
                  }}
                  onDragLeave={() => setOver((current) => (current === index ? null : current))}
                  onDrop={(event) => {
                    event.preventDefault();
                    if (dragging !== null) {
                      void move(dragging, index);
                    }
                    setDragging(null);
                    setOver(null);
                  }}
                  onDragEnd={() => {
                    setDragging(null);
                    setOver(null);
                  }}
                  className={`flex items-center gap-3 rounded border px-3 py-2 text-sm ${
                    over === index && dragging !== index ? "border-violet-400" : stale && workspace.enabled ? "border-amber-500/50 bg-amber-500/5" : "border-slate-700"
                  } ${dragging === index ? "opacity-50" : ""}`}
                >
                  {several && (
                    <span aria-hidden="true" title="Drag to reorder" data-testid="drag-handle" className="cursor-grab select-none text-base leading-none text-slate-400 hover:text-slate-200">
                      ⠿
                    </span>
                  )}
                  <span
                    aria-label={`Priority ${index + 1}`}
                    className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-slate-800 text-xs font-semibold text-slate-200"
                  >
                    {index + 1}
                  </span>
                  <Tooltip
                    label={workspace.enabled ? "Enabled" : "Disabled"}
                    detail="Off keeps the sign-in but skips this workspace for search, sends, bumps and status reads."
                    align="start"
                  >
                    <Switch label={`Use ${workspace.team_name}`} checked={workspace.enabled} onChange={(on) => void setEnabled(workspace, on)} />
                  </Tooltip>
                  <span className={`min-w-0 flex-1 ${workspace.enabled ? "" : "opacity-60"}`}>
                    <span className="font-medium">{name}</span>{" "}
                    <span className="text-slate-400">
                      · {workspace.domain}.slack.com · @{workspace.user_name}
                      {workspace.signed_in || workspace.needs_sign_in ? "" : " · signed out"}
                    </span>
                    {workspace.needs_sign_in && <span className="text-amber-300"> · needs sign-in</span>}
                    {!workspace.enabled && <span className="text-slate-400"> · disabled, never used</span>}
                  </span>
                  <span className="flex shrink-0 items-center gap-3">
                    {stale && (
                      <button
                        type="button"
                        className="rounded bg-amber-400 px-2.5 py-1 text-xs font-semibold text-slate-950 hover:bg-amber-300 disabled:opacity-40"
                        aria-label={`Reconnect ${workspace.team_name}`}
                        disabled={!canSignIn}
                        onClick={() => void connect(workspace)}
                      >
                        Reconnect
                      </button>
                    )}
                    {several && (
                      <span className="flex items-center gap-1">
                        <Tooltip label="Move up" detail="Try this workspace earlier when sending.">
                          <button
                            type="button"
                            className="rounded px-1 text-xs text-slate-300 hover:bg-slate-800 disabled:opacity-30"
                            aria-label={`Move ${workspace.team_name} up`}
                            disabled={index === 0}
                            onClick={() => void move(index, index - 1)}
                          >
                            ↑
                          </button>
                        </Tooltip>
                        <Tooltip label="Move down" detail="Try this workspace later when sending.">
                          <button
                            type="button"
                            className="rounded px-1 text-xs text-slate-300 hover:bg-slate-800 disabled:opacity-30"
                            aria-label={`Move ${workspace.team_name} down`}
                            disabled={index === ordered.length - 1}
                            onClick={() => void move(index, index + 1)}
                          >
                            ↓
                          </button>
                        </Tooltip>
                      </span>
                    )}
                    <button
                      type="button"
                      className="text-xs text-slate-300 underline hover:text-white"
                      aria-label={`Remove ${workspace.team_name}`}
                      onClick={() => void remove(workspace)}
                    >
                      Remove
                    </button>
                  </span>
                </li>
              );
            })}
          </ul>
        )}
      </div>
      <div className="flex items-center gap-2">
        <button
          type="button"
          className="rounded bg-violet-500 px-3 py-2 text-sm font-medium text-white disabled:opacity-40"
          disabled={!canSignIn}
          onClick={() => void connect()}
        >
          {connecting ? "Waiting for Slack…" : workspaces.length > 0 ? "Add another workspace" : "Sign in with Slack"}
        </button>
        {connecting && (
          <button
            type="button"
            className="rounded border border-slate-600 px-3 py-2 text-sm"
            onClick={() => void api.slackCancelConnect()}
          >
            Cancel
          </button>
        )}
      </div>
      <p className="text-xs text-slate-400" role="status" aria-label="Slack sign-in status">
        {connecting
          ? `${elapsed >= STILL_WAITING_AFTER_SECONDS ? "Still waiting for Slack. " : ""}Finish signing in in your browser · ${formatLeft(SIGN_IN_TIMEOUT_SECONDS - elapsed)} left.`
          : clientId
            ? "Opens your default browser. Sign in to one workspace at a time."
            : "Enter the client ID under Slack app (advanced) to sign in."}
      </p>
      {timedOut && !connecting && (
        <button type="button" className="block text-sm text-amber-300 underline" onClick={() => void connect()}>
          Sign-in timed out, click to retry.
        </button>
      )}
      {notice && <p className="text-sm text-emerald-300">{notice}</p>}
      {error && <p className="text-sm text-rose-300">{error}</p>}
      <details open={!clientId} className="rounded border border-slate-800 px-3 py-2">
        <summary className="cursor-pointer text-sm font-medium text-slate-200">
          Slack app (advanced){clientId ? " · configured" : " · not configured"}
        </summary>
        <div className="mt-3">{appFields}</div>
      </details>
    </div>
  );
}
