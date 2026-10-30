import { useState } from "react";
import type { Api, MentionsScope, SlackWorkspace } from "../api";
import { MessageTemplates } from "./MessageTemplates";
import { SlackPanel, type SlackFields } from "./SlackPanel";

interface Props {
  api: Api;
  fields: SlackFields;
  onFieldsChange: (fields: SlackFields) => void;
  mentions: MentionsScope;
  onMentionsChange: (scope: MentionsScope) => void;
  secretSaved: boolean;
  savedClientId: string;
  initialWorkspaces: SlackWorkspace[];
  /** Sign-in, switches and reorder save at once; reported up so banners follow. */
  onWorkspacesChange?: (workspaces: SlackWorkspace[]) => void;
}

/** Settings → Accounts → Slack. App fields and mentions save through the Settings save bar. */
export function SlackSettings({ api, fields, onFieldsChange, mentions, onMentionsChange, secretSaved, savedClientId, initialWorkspaces, onWorkspacesChange }: Props) {
  const [workspaces, setWorkspaces] = useState(initialWorkspaces);
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
        onFieldsChange={onFieldsChange}
        secretSaved={secretSaved}
        savedClientId={savedClientId}
        workspaces={workspaces}
        onWorkspacesChange={(next) => {
          setWorkspaces(next);
          onWorkspacesChange?.(next);
        }}
      />
      <div className="mt-6">
        <label className="flex items-center gap-2 text-sm">
          Find mentions in the PR detail
          <select
            className="rounded border border-slate-700 bg-slate-900 px-2 py-1"
            value={mentions}
            onChange={(event) => onMentionsChange(event.target.value as MentionsScope)}
          >
            <option value="off">Off</option>
            <option value="public">Public channels</option>
            <option value="all">Public, private channels and DMs</option>
          </select>
        </label>
        <p className="mt-1 text-xs text-slate-500">
          Searches Slack for the PR link when you click Find mentions, one search per signed-in workspace.
          {mentions === "all" && " Results can include private channels and DMs you are in; they stay on this computer."}
        </p>
      </div>
      <MessageTemplates api={api} onOpen={(url) => void api.openExternal(url)} />
    </section>
  );
}
