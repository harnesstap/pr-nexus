import { useState, type ReactNode } from "react";
import type { Api, Preferences } from "../api";
import { errorMessage } from "../api";
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
  const [draft, setDraft] = useState(prefs);
  const [status, setStatus] = useState<{ text: string; error: boolean } | null>(null);
  const [saving, setSaving] = useState(false);

  function patch(next: Partial<Preferences>) {
    setDraft((current) => ({ ...current, ...next }));
    setStatus(null);
  }

  async function save() {
    setSaving(true);
    setStatus(null);
    try {
      const saved = await api.savePreferences(draft);
      setDraft(saved);
      onSaved(saved);
      setStatus({ text: "Saved.", error: false });
    } catch (err) {
      setStatus({ text: errorMessage(err), error: true });
    } finally {
      setSaving(false);
    }
  }

  const dirty = JSON.stringify(draft) !== JSON.stringify(prefs);

  return (
    <form
      className="space-y-5"
      onSubmit={(event) => {
        event.preventDefault();
        void save();
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
        </>
      ) : (
        <>
          <div>
            <Tooltip
              label="Live updates"
              detail="Watch your GitHub notifications and refresh only the PRs they mention, about once a minute. Unchanged checks cost nothing against the rate limit. Notifications are never marked as read."
              align="start"
            >
              <label className="flex w-fit items-center gap-2 text-sm">
                <input type="checkbox" checked={draft.live_updates} onChange={(event) => patch({ live_updates: event.target.checked })} />
                Live updates
              </label>
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
      <div className="flex items-center gap-3">
        <button type="submit" className="rounded bg-violet-500 px-3 py-2 text-sm disabled:opacity-50" disabled={saving || !dirty}>
          {saving ? "Saving…" : "Save"}
        </button>
        {status && (
          <span role="status" className={`text-sm ${status.error ? "text-rose-300" : "text-slate-300"}`}>
            {status.text}
          </span>
        )}
      </div>
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
      <label className="flex items-center gap-2 text-sm">
        <input type="checkbox" checked={checked} onChange={(event) => onChange(event.target.checked)} />
        {label}
      </label>
      <p className="mt-1 text-xs text-slate-500">{help}</p>
    </div>
  );
}
