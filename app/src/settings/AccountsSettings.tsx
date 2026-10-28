import type { Api, Bootstrap, MentionsScope, SlackWorkspace } from "../api";
import { OrgOption } from "../orgs";
import { DEFAULT_REDIRECT, type SlackFields } from "../slack/SlackPanel";
import { SlackSettings } from "../slack/SlackSettings";
import { Switch } from "../Switch";
import { SaveBar, useSaveBar } from "./SaveBar";

interface Props {
  api: Api;
  boot: Bootstrap;
  /** After orgs are saved: the list re-syncs. */
  onOrgsSaved: (orgs: string[], personal: boolean) => Promise<void>;
  onMentionsChange: (scope: MentionsScope) => void;
  onWorkspacesChange?: (workspaces: SlackWorkspace[]) => void;
}

export interface AccountsDraft {
  orgs: string[];
  personal: boolean;
  slack: SlackFields;
  mentions: MentionsScope;
}

export function accountsFromBoot(boot: Bootstrap): AccountsDraft {
  return {
    orgs: [...boot.tracked_orgs].sort(),
    personal: boot.include_personal,
    slack: { clientId: boot.slack_client_id, clientSecret: "", redirectUri: boot.slack_redirect_uri || DEFAULT_REDIRECT },
    mentions: boot.slack_mentions,
  };
}

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

/** Settings → Accounts: providers, tracked orgs and Slack, saved together from one save bar. */
export function AccountsSettings({ api, boot, onOrgsSaved, onMentionsChange, onWorkspacesChange }: Props) {
  const { draft, setDraft, bar } = useSaveBar(accountsFromBoot(boot), async (next, saved) => {
    if (!same(next.slack, saved.slack)) {
      await api.saveSlack(next.slack.clientId.trim(), next.slack.clientSecret, next.slack.redirectUri.trim());
    }
    if (next.mentions !== saved.mentions) {
      await api.saveSlackMentions(next.mentions);
      onMentionsChange(next.mentions);
    }
    if (next.orgs.join() !== saved.orgs.join() || next.personal !== saved.personal) {
      await api.saveOrgs(next.orgs, next.personal);
      await onOrgsSaved(next.orgs, next.personal);
    }
    return { ...next, slack: { ...next.slack, clientSecret: "" } };
  });
  const toggleOrg = (org: string) =>
    setDraft((current) => ({
      ...current,
      orgs: (current.orgs.includes(org) ? current.orgs.filter((item) => item !== org) : [...current.orgs, org]).sort(),
    }));

  return (
    <div className="space-y-8">
      <section aria-labelledby="accounts-providers">
        <h2 id="accounts-providers" className="text-base font-semibold">
          Providers
        </h2>
        <p className="mt-2 text-sm text-slate-300">GitHub{boot.github_login ? ` · signed in as ${boot.github_login}` : ""} · Slack</p>
        <p className="mt-1 text-xs text-slate-500">To change the GitHub account, use About → Re-run setup.</p>
      </section>
      <section aria-labelledby="accounts-orgs">
        <h2 id="accounts-orgs" className="text-base font-semibold">
          Tracked orgs
        </h2>
        <p className="mt-2 text-sm text-slate-400">
          Nothing is selected by default. The list stays empty until you pick at least one org or include personal repos.
        </p>
        <ul className="mt-4 space-y-2">
          {boot.org_choices.map((org) => (
            <li key={org}>
              <OrgOption
                org={org}
                checked={draft.orgs.includes(org)}
                needsSso={boot.orgs_needing_sso.includes(org)}
                onToggle={() => toggleOrg(org)}
                onOpen={(url) => void api.openExternal(url)}
              />
            </li>
          ))}
        </ul>
        <Switch className="mt-4" label="Include my personal repos" checked={draft.personal} onChange={(personal) => setDraft((current) => ({ ...current, personal }))}>
          Include my personal repos
        </Switch>
      </section>
      <SlackSettings
        api={api}
        fields={draft.slack}
        onFieldsChange={(slack) => setDraft((current) => ({ ...current, slack }))}
        mentions={draft.mentions}
        onMentionsChange={(mentions) => setDraft((current) => ({ ...current, mentions }))}
        secretSaved={boot.slack_secret_saved}
        savedClientId={boot.slack_client_id}
        initialWorkspaces={boot.slack_workspaces}
        onWorkspacesChange={onWorkspacesChange}
      />
      <SaveBar {...bar} />
    </div>
  );
}
