import { useEffect, useState } from "react";
import type { Api, SlackWorkspace } from "../api";
import { errorMessage } from "../api";
import { plainText } from "../textInput";

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
}

/**
 * Slack app fields, connected workspaces, and browser sign-in (§7.11). Shared
 * by wizard step 5 and Settings. Connect saves the fields first, so the
 * redirect URI typed here is the one the loopback listener binds.
 */
export function SlackPanel({ api, fields, onFieldsChange, secretSaved, workspaces, onWorkspacesChange }: Props) {
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [timedOut, setTimedOut] = useState(false);
  const [elapsed, setElapsed] = useState(0);
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
      await api.saveSlack(fields.clientId, fields.clientSecret, fields.redirectUri);
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

  const input = "mt-1 w-full rounded border border-slate-700 bg-slate-900 px-3 py-2";
  return (
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
      <div>
        <h3 className="text-sm font-medium">Connected workspaces</h3>
        {workspaces.length === 0 ? (
          <p className="mt-1 text-sm text-slate-400">None yet.</p>
        ) : (
          <ul className="mt-2 space-y-2" aria-label="Connected workspaces">
            {workspaces.map((workspace) => (
              <li
                key={workspace.id}
                className="flex items-center justify-between rounded border border-slate-700 px-3 py-2 text-sm"
              >
                <span>
                  <span className="font-medium">{workspace.team_name}</span>{" "}
                  <span className="text-slate-400">
                    · {workspace.domain}.slack.com · @{workspace.user_name}
                    {workspace.signed_in || workspace.needs_sign_in ? "" : " · signed out"}
                  </span>
                  {workspace.needs_sign_in && <span className="text-amber-300"> · needs sign-in</span>}
                </span>
                <span className="flex items-center gap-3">
                  {!workspace.signed_in && (
                    <button
                      type="button"
                      className="text-xs text-violet-300 underline disabled:opacity-40"
                      aria-label={`Reconnect ${workspace.team_name}`}
                      disabled={connecting || !fields.clientId.trim()}
                      onClick={() => void connect(workspace)}
                    >
                      Reconnect
                    </button>
                  )}
                  <button
                    type="button"
                    className="text-xs text-slate-300 underline"
                    aria-label={`Remove ${workspace.team_name}`}
                    onClick={() => void remove(workspace)}
                  >
                    Remove
                  </button>
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>
      <div className="flex items-center gap-2">
        <button
          type="button"
          className="rounded bg-violet-500 px-3 py-2 text-sm font-medium text-white disabled:opacity-40"
          disabled={connecting || !fields.clientId.trim()}
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
          : fields.clientId.trim()
            ? "Opens your default browser. Sign in to one workspace at a time."
            : "Enter the client ID to sign in."}
      </p>
      {timedOut && !connecting && (
        <button type="button" className="block text-sm text-amber-300 underline" onClick={() => void connect()}>
          Sign-in timed out, click to retry.
        </button>
      )}
      {notice && <p className="text-sm text-emerald-300">{notice}</p>}
      {error && <p className="text-sm text-rose-300">{error}</p>}
    </div>
  );
}
