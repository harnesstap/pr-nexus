import { useState } from "react";
import type { Api, Bootstrap } from "../api";
import { errorMessage } from "../api";
import { DEFAULT_REDIRECT, SlackPanel, type SlackFields } from "./SlackPanel";

/** Settings → Slack: the same fields and sign-in as wizard step 5, plus Save. */
export function SlackSettings({ api, boot }: { api: Api; boot: Bootstrap }) {
  const [fields, setFields] = useState<SlackFields>({
    clientId: boot.slack_client_id,
    clientSecret: "",
    redirectUri: boot.slack_redirect_uri || DEFAULT_REDIRECT,
  });
  const [workspaces, setWorkspaces] = useState(boot.slack_workspaces);
  const [status, setStatus] = useState<string | null>(null);

  async function save() {
    setStatus(null);
    try {
      await api.saveSlack(fields.clientId, fields.clientSecret, fields.redirectUri);
      setStatus("Slack settings saved.");
    } catch (err) {
      setStatus(errorMessage(err));
    }
  }

  return (
    <section className="mt-10 border-t border-slate-800 pt-6" aria-labelledby="slack-settings">
      <h2 id="slack-settings" className="text-lg font-semibold">
        Slack
      </h2>
      <p className="mb-4 mt-1 text-sm text-slate-400">
        PR-Nexus posts review requests as you through Slack's MCP server. Sign in to each workspace you post in.
      </p>
      <SlackPanel
        api={api}
        fields={fields}
        onFieldsChange={setFields}
        secretSaved={boot.slack_secret_saved}
        workspaces={workspaces}
        onWorkspacesChange={setWorkspaces}
      />
      <div className="mt-4 flex items-center gap-3">
        <button type="button" className="rounded border border-slate-600 px-3 py-2 text-sm" onClick={() => void save()}>
          Save Slack settings
        </button>
        {status && <span className="text-sm text-slate-300">{status}</span>}
      </div>
    </section>
  );
}
