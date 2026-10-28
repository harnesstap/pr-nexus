import type { ReactNode } from "react";
import type { Api, Preferences } from "../api";
import { Switch } from "../Switch";
import { SaveBar, useSaveBar } from "./SaveBar";
import { toMinutes } from "./tabs";
import { plainText } from "../textInput";
import { Tooltip } from "../Tooltip";

interface Props {
  api: Api;
  prefs: Preferences;
  section: "review" | "sync";
  onSaved: (prefs: Preferences) => void;
}

const input = "mt-1 w-28 rounded border border-slate-700 bg-slate-900 px-2 py-1";

/** Settings → Review and Settings → Sync: the §7.12 keys, saved together. */
export function PreferencesForm({ api, prefs, section, onSaved }: Props) {
  const { draft, setDraft, bar } = useSaveBar(prefs, async (next) => {
    const saved = await api.savePreferences(next);
    onSaved(saved);
    return saved;
  });

  function patch(next: Partial<Preferences>) {
    setDraft((current) => ({ ...current, ...next }));
  }

  return (
    <form
      className="space-y-5"
      onSubmit={(event) => {
        event.preventDefault();
        bar.onSave();
      }}
    >
      {section === "review" ? (
        <>
          <Field label="Requester" help='Added in brackets after the PR in review requests, e.g. "Ada Acme". Never on reminders. Empty adds nothing.'>
            <input {...plainText}
              className="mt-1 w-full rounded border border-slate-700 bg-slate-900 px-2 py-1"
              value={draft.review_requester}
              onChange={(event) => patch({ review_requester: event.target.value })}
            />
          </Field>
          <Field label="Minimum hours between bumps" help="A PR bumped more recently is skipped, with a warning in the preview.">
            <input {...plainText}
              className={input}
              type="number"
              min={1}
              max={168}
              value={draft.min_bump_interval_hours}
              onChange={(event) => patch({ min_bump_interval_hours: Number(event.target.value) })}
            />
          </Field>
          <Field label="Threads per bulk bump" help="Above this many threads in one channel, a bulk bump becomes one reminder message.">
            <input {...plainText}
              className={input}
              type="number"
              min={1}
              max={20}
              value={draft.bulk_bump_max_threads}
              onChange={(event) => patch({ bulk_bump_max_threads: Number(event.target.value) })}
            />
          </Field>
          <Check
            label="Also post bumps to the channel"
            help="A bump is always a reply in the request's thread. With this on, Slack also shows it in the channel so reviewers who left the thread see it."
            checked={draft.bump_broadcast}
            onChange={(on) => patch({ bump_broadcast: on })}
          />
          <Check
            label="Count blocked PRs that only need a review as ready"
            help="A PR blocked only by a missing approval shows as Ready to review instead of Blocked."
            checked={draft.blocked_review_required_is_ready}
            onChange={(on) => patch({ blocked_review_required_is_ready: on })}
          />
          <Field label="Ticket key pattern" help="Regular expression for ticket keys in branch names and titles, used by project rules. Empty restores the default.">
            <input {...plainText}
              className="mt-1 w-full rounded border border-slate-700 bg-slate-900 px-2 py-1 font-mono text-sm"
              value={draft.ticket_key_regex}
              onChange={(event) => patch({ ticket_key_regex: event.target.value })}
            />
          </Field>
          <Field label="JIRA base URL" help="Your JIRA site, like https://acme-labs.atlassian.net. With it set, a bare key such as ACME-123 on a project links to its ticket. Empty turns bare keys into plain text.">
            <input {...plainText}
              aria-label="JIRA base URL"
              type="url"
              className="mt-1 w-full rounded border border-slate-700 bg-slate-900 px-2 py-1 font-mono text-sm"
              placeholder="https://acme-labs.atlassian.net"
              value={draft.jira_base_url}
              onChange={(event) => patch({ jira_base_url: event.target.value })}
            />
          </Field>
        </>
      ) : (
        <>
          <div>
            <Tooltip
              label="Live updates"
              detail="Watch your GitHub notifications and refresh only the PRs they mention, about once a minute. Unchanged checks cost nothing against the rate limit. Notifications are never marked as read."
              align="start"
            >
              <Switch label="Live updates" className="text-sm" checked={draft.live_updates} onChange={(on) => patch({ live_updates: on })}>
                Live updates
              </Switch>
            </Tooltip>
            <p className="mt-1 text-xs text-slate-500">
              Near-real-time refresh between syncs. Needs a classic token or the GitHub CLI; other tokens fall back to the periodic sync.
            </p>
          </div>
          <Field label="Sync every (minutes, window focused)" help="1 to 60 minutes.">
            <input {...plainText}
              className={input}
              type="number"
              min={1}
              max={60}
              value={toMinutes(draft.focused_interval_s)}
              onChange={(event) => patch({ focused_interval_s: Number(event.target.value) * 60 })}
            />
          </Field>
          <Field label="Sync every (minutes, in the background)" help="1 minute to 24 hours.">
            <input {...plainText}
              className={input}
              type="number"
              min={1}
              max={1440}
              value={toMinutes(draft.unfocused_interval_s)}
              onChange={(event) => patch({ unfocused_interval_s: Number(event.target.value) * 60 })}
            />
          </Field>
          <Field label="Keep closed PRs (days)" help="Merged and closed PRs leave the open list at the next sync. Filters → Archived and closed still shows them this long. 0 drops them right away.">
            <input {...plainText}
              className={input}
              type="number"
              min={0}
              max={90}
              value={draft.keep_closed_days}
              onChange={(event) => patch({ keep_closed_days: Number(event.target.value) })}
            />
          </Field>
        </>
      )}
      <SaveBar {...bar} submit />
    </form>
  );
}

function Field({ label, help, children }: { label: string; help: string; children: ReactNode }) {
  return (
    <label className="block text-sm">
      {label}
      <span className="mt-0.5 block text-xs text-slate-500">{help}</span>
      {children}
    </label>
  );
}

function Check({ label, help, checked, onChange }: { label: string; help: string; checked: boolean; onChange: (on: boolean) => void }) {
  return (
    <div>
      <Switch label={label} className="text-sm" checked={checked} onChange={onChange}>
        {label}
      </Switch>
      <p className="mt-1 text-xs text-slate-500">{help}</p>
    </div>
  );
}
