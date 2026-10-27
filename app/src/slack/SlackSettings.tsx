import { useState } from "react";
import type { Api, Bootstrap, MentionsScope } from "../api";
import { errorMessage } from "../api";
import { DEFAULT_REDIRECT, SlackPanel, type SlackFields } from "./SlackPanel";

interface Props {
  api: Api;
  boot: Bootstrap;
  /** The saved scope, so the PR detail's Slack panel picks it up without a restart. */
  onMentionsChange?: (scope: MentionsScope) => void;
}

/** Settings → Slack: the same fields and sign-in as wizard step 5, plus Save. */
export function SlackSettings({ api, boot, onMentionsChange }: Props) {
  const [fields, setFields] = useState<SlackFields>({
    clientId: boot.slack_client_id,
    clientSecret: "",
    redirectUri: boot.slack_redirect_uri || DEFAULT_REDIRECT,
  });
  const [workspaces, setWorkspaces] = useState(boot.slack_workspaces);
  const [status, setStatus] = useState<string | null>(null);
  const [mentions, setMentions] = useState<MentionsScope>(boot.slack_mentions);
  const [mentionsStatus, setMentionsStatus] = useState<string | null>(null);
  function changeMentions(scope: MentionsScope) {
    const previous = mentions;
    setMentions(scope);
    setMentionsStatus(null);
    api
      .saveSlackMentions(scope)
      .then(() => {
        setMentionsStatus("Saved.");
        onMentionsChange?.(scope);
      })
      .catch((err: unknown) => {
        setMentions(previous);
        setMentionsStatus(errorMessage(err));
      });
  }

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
        PR Nexus posts review requests as you through Slack's MCP server. Sign in to each workspace you post in.
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
      <div className="mt-6">
        <label className="flex items-center gap-2 text-sm">
          Find mentions in the PR detail
          <select
            className="rounded border border-slate-700 bg-slate-900 px-2 py-1"
            value={mentions}
            onChange={(event) => changeMentions(event.target.value as MentionsScope)}
          >
            <option value="off">Off</option>
            <option value="public">Public channels</option>
            <option value="all">Public, private channels and DMs</option>
          </select>
          {mentionsStatus && <span className="text-slate-300">{mentionsStatus}</span>}
        </label>
        <p className="mt-1 text-xs text-slate-500">
          Searches Slack for the PR link when you click Find mentions, one search per signed-in workspace.
          {mentions === "all" && " Results can include private channels and DMs you are in; they stay on this computer."}
        </p>
      </div>
    </section>
  );
}
