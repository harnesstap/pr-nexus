import { useState } from "react";
import type { Api, SlackWorkspace } from "../api";
import { errorMessage } from "../api";

export const DEFAULT_REDIRECT = "http://127.0.0.1:47821/mcp/oauth/callback";

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
  const set = (patch: Partial<SlackFields>) => onFieldsChange({ ...fields, ...patch });
  const signedIn = workspaces.filter((workspace) => workspace.signed_in);

  async function openManifest() {
    setError(null);
    try {
      const url = await api.slackManifestUrl(fields.redirectUri);
      await api.openExternal(url);
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  async function connect() {
    setError(null);
    setNotice(null);
    setConnecting(true);
    try {
      await api.saveSlack(fields.clientId, fields.clientSecret, fields.redirectUri);
      const workspace = await api.slackConnect();
      onWorkspacesChange([...workspaces.filter((item) => item.id !== workspace.id), workspace]);
      setNotice(`Connected to ${workspace.team_name}.`);
    } catch (err) {
      const kind = err && typeof err === "object" && "kind" in err ? (err as { kind?: unknown }).kind : null;
      if (kind !== "cancelled") {
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
        The link opens Slack with the PR-Nexus manifest (name, user scopes, redirect URI, MCP on). No client id ships
        with the app. After creating it, copy the client ID (and secret) from Basic Information.
      </p>
      <label className="block text-sm">
        Client ID
        <input
          className={input}
          value={fields.clientId}
          disabled={connecting}
          onChange={(event) => set({ clientId: event.target.value })}
        />
      </label>
      <label className="block text-sm">
        Client secret
        <input
          className={input}
          type="password"
          autoComplete="off"
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
        <input
          className={input}
          value={fields.redirectUri}
          disabled={connecting}
          placeholder={DEFAULT_REDIRECT}
          onChange={(event) => set({ redirectUri: event.target.value })}
        />
        <span className="mt-0.5 block text-xs text-slate-400">
          Must match a redirect URL registered on the Slack app. PR-Nexus listens on this loopback address during
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
                    {workspace.signed_in ? "" : " · signed out"}
                  </span>
                </span>
                <button
                  type="button"
                  className="text-xs text-slate-300 underline"
                  aria-label={`Remove ${workspace.team_name}`}
                  onClick={() => void remove(workspace)}
                >
                  Remove
                </button>
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
          {connecting ? "Waiting for Slack…" : signedIn.length > 0 ? "Add another workspace" : "Sign in with Slack"}
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
      <p className="text-xs text-slate-400" role="status">
        {connecting
          ? "Finish signing in in your browser. This times out after 2 minutes."
          : fields.clientId.trim()
            ? "Opens your default browser. Sign in to one workspace at a time."
            : "Enter the client ID to sign in."}
      </p>
      {notice && <p className="text-sm text-emerald-300">{notice}</p>}
      {error && <p className="text-sm text-rose-300">{error}</p>}
    </div>
  );
}
