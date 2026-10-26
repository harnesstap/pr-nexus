import { useEffect, useState } from "react";
import type { Api, ContextThread, MentionGroup, MentionsScope } from "../api";
import { channelLabel, errorMessage } from "../api";
import { ago, requestAction } from "./model";
import { shortTime, slackText } from "./slackText";

interface Props {
  api: Api;
  prId: number;
  mentions: MentionsScope;
  onOpen: (url: string) => void;
}

/**
 * The PR detail's Slack panel (§7.10): threads PR-Nexus posted for this PR,
 * read through Slack's MCP server and cached for 5 minutes. Read-only.
 */
export function SlackContextPanel({ api, prId, mentions, onOpen }: Props) {
  const [threads, setThreads] = useState<ContextThread[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [found, setFound] = useState<MentionGroup[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [searchError, setSearchError] = useState<string | null>(null);

  useEffect(() => {
    let cancel = false;
    api
      .prSlackContext(prId, false)
      .then((next) => !cancel && setThreads(next))
      .catch((err: unknown) => !cancel && setError(errorMessage(err)));
    return () => {
      cancel = true;
    };
  }, [api, prId]);

  function refresh() {
    setBusy(true);
    setError(null);
    api
      .prSlackContext(prId, true)
      .then(setThreads)
      .catch((err: unknown) => setError(errorMessage(err)))
      .finally(() => setBusy(false));
  }

  function search() {
    setSearching(true);
    setSearchError(null);
    api
      .prSlackMentions(prId)
      .then(setFound)
      .catch((err: unknown) => setSearchError(errorMessage(err)))
      .finally(() => setSearching(false));
  }

  return (
    <aside aria-labelledby="slack-panel-heading" className="min-w-0 rounded border border-slate-800 bg-slate-950/40 p-3">
      <div className="flex items-center justify-between gap-2">
        <h3 id="slack-panel-heading" className="text-xs font-medium uppercase tracking-wide text-slate-500">
          Slack
        </h3>
        {threads && threads.length > 0 && (
          <button type="button" className="rounded px-2 py-0.5 text-xs text-slate-400 hover:bg-slate-800 disabled:opacity-50" disabled={busy} onClick={refresh}>
            {busy ? "Refreshing…" : "Refresh"}
          </button>
        )}
      </div>
      {error && <p className="mt-2 text-rose-300">{error}</p>}
      {!threads && !error && <p className="mt-2 text-slate-400">Reading Slack threads…</p>}
      {threads && threads.length === 0 && <p className="mt-2 text-slate-400">No Slack request for this PR yet.</p>}
      {threads?.map((thread) => (
        <Thread key={`${thread.channel_id}:${thread.root_ts}`} thread={thread} onOpen={onOpen} />
      ))}
      {mentions !== "off" && (
        <section aria-label="Mentions" className="mt-4 border-t border-slate-800 pt-3">
          <button type="button" className="rounded border border-slate-700 px-2 py-1 text-xs text-slate-300 hover:bg-slate-800 disabled:opacity-50" disabled={searching} onClick={search}>
            {searching ? "Searching…" : `Find mentions (${mentions === "all" ? "public, private, DMs" : "public channels"})`}
          </button>
          {searchError && <p className="mt-2 text-rose-300">{searchError}</p>}
          {found && <Mentions groups={found} onOpen={onOpen} />}
        </section>
      )}
    </aside>
  );
}

/** The stored state of a thread, from the DB: it does not need Slack to be readable. */
function threadStatus(thread: ContextThread, now: Date): { label: string; title?: string } {
  const at = thread.requested_at;
  if (thread.status === "posted") {
    return { label: at ? `Posted · ${ago(at, now)}` : "Posted" };
  }
  const action = at ? requestAction({ requested_at: at, last_bumped_at: thread.last_bumped_at, bump_count: thread.bump_count }, now) : null;
  if (thread.status === "closed") {
    const closed = `Closed: ${thread.close_reason ?? "closed"}`;
    return action ? { label: `${closed} · ${action.label}`, title: action.title } : { label: closed };
  }
  return action ?? { label: "Active" };
}

function Thread({ thread, onOpen }: { thread: ContextThread; onOpen: (url: string) => void }) {
  const label = channelLabel(thread);
  const replies = Math.max(thread.messages.length - 1, 0);
  const status = threadStatus(thread, new Date());
  return (
    <article aria-label={`${label} thread`} className={`mt-3 rounded border p-2 ${thread.status === "active" ? "border-violet-500/40" : "border-slate-800 opacity-80"}`}>
      <header className="flex items-baseline justify-between gap-2 text-xs">
        <span className="min-w-0 truncate">
          <span className="font-medium text-slate-200">{label}</span>
          <span className="text-slate-500" title={status.title}>
            {" · "}
            {status.label}
            {thread.messages.length > 0 && ` · ${replies} ${replies === 1 ? "reply" : "replies"}${thread.has_more ? "+" : ""}`}
          </span>
        </span>
        {thread.permalink && (
          <button type="button" className="shrink-0 text-violet-300 hover:underline" onClick={() => onOpen(thread.permalink ?? "")}>
            Open ↗
          </button>
        )}
      </header>
      {thread.error && (
        <p className="mt-1 text-xs text-amber-200">
          {thread.error}
          {thread.messages.length > 0 ? " Showing the cached copy." : thread.permalink ? " Open the thread in Slack to read the replies." : ""}
        </p>
      )}
      <ol className="mt-1 space-y-2">
        {thread.messages.map((message, index) => (
          <li key={message.ts} className={index === 0 ? "" : "border-l border-slate-700 pl-2"}>
            <p className="text-xs text-slate-500">
              <span className="text-slate-300">{message.author}</span>
              {message.is_bot && " (app)"}
              {" · "}
              {message.permalink ? (
                <button type="button" className="hover:underline" title={message.at ?? undefined} onClick={() => onOpen(message.permalink ?? "")}>
                  {shortTime(message.at)}
                </button>
              ) : (
                <span title={message.at ?? undefined}>{shortTime(message.at)}</span>
              )}
            </p>
            <p className="whitespace-pre-wrap break-words text-slate-200">{slackText(message.text)}</p>
          </li>
        ))}
      </ol>
      {thread.has_more && <p className="mt-1 text-xs text-slate-500">Only the first 50 replies. Open the thread in Slack for the rest.</p>}
      {thread.fetched_at && (
        <p className="mt-1 text-[11px] text-slate-600">
          {thread.from_cache ? "Cached" : "Read"} {shortTime(thread.fetched_at)}
        </p>
      )}
    </article>
  );
}

function Mentions({ groups, onOpen }: { groups: MentionGroup[]; onOpen: (url: string) => void }) {
  const total = groups.reduce((sum, group) => sum + group.hits.length, 0);
  return (
    <div className="mt-2 space-y-2">
      {groups.map((group) => group.error && (
        <p key={`${group.workspace}-error`} className="text-xs text-amber-200">
          {group.workspace}: {group.error}
        </p>
      ))}
      {total === 0 && groups.every((group) => !group.error) && <p className="text-xs text-slate-400">No other messages link this PR.</p>}
      <ul className="space-y-2">
        {groups.flatMap((group) =>
          group.hits.map((hit) => (
            <li key={`${group.workspace}:${hit.channel_id}:${hit.ts}`} className="text-xs">
              <p className="text-slate-500">
                <span className="text-slate-300">{channelLabel(hit)}</span>
                {" · "}
                {hit.author ?? "Unknown"}
                {hit.is_bot && " (app)"}
                {hit.permalink && (
                  <button type="button" className="ml-2 text-violet-300 hover:underline" onClick={() => onOpen(hit.permalink ?? "")}>
                    Open ↗
                  </button>
                )}
              </p>
              <p className="whitespace-pre-wrap break-words text-slate-200">{slackText(hit.text)}</p>
            </li>
          )),
        )}
      </ul>
    </div>
  );
}
