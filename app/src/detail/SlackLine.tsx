import { useEffect, useState } from "react";
import type { Api, ContextThread, SlackWorkspace } from "../api";
import { channelLabel, errorMessage } from "../api";
import { SlackIcon } from "../icons";
import { shortTime, slackText } from "../list/slackText";
import { slackActionBlock } from "../list/SyncBanners";
import { Tooltip } from "../Tooltip";

interface Props {
  api: Api;
  prId: number;
  onOpen: (url: string) => void;
  /** Opens the request or bump preview. */
  onRequest?: () => void;
  /** Connected workspaces; one that needs sign-in disables its action. */
  workspaces?: SlackWorkspace[];
}

/** Replies shown under the request before the rest fold into "+N more". */
export const MAX_REPLIES = 2;

/** The newest request, by request time, then by root message. */
export function latestThread(threads: ContextThread[]): ContextThread | null {
  return (
    [...threads].sort((a, b) => (b.requested_at ?? "").localeCompare(a.requested_at ?? "") || b.root_ts.localeCompare(a.root_ts))[0] ?? null
  );
}

/**
 * The panel's Slack action (§7.10): one big "Request review on Slack" button until a
 * request exists, then the latest request with its newest replies and a Bump button.
 * Both open the preview, where another channel can still get a new request.
 */
export function SlackLine({ api, prId, onOpen, onRequest, workspaces = [] }: Props) {
  const [threads, setThreads] = useState<ContextThread[] | null>(null);
  const [error, setError] = useState<string | null>(null);

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

  if (!threads && !error) {
    return <div className="h-10 animate-pulse rounded-lg bg-slate-800/70" aria-label="Loading Slack" />;
  }
  const thread = threads ? latestThread(threads) : null;
  const blocked = slackActionBlock(workspaces, thread?.workspace);
  if (!thread) {
    return (
      <div className="space-y-1.5" data-testid="slack-empty">
        {onRequest && (
          <Tooltip label={blocked ?? "Request review on Slack"} shortcut="R" detail={blocked ? "Reconnect it in Settings › Accounts" : "Shows the exact message first; nothing is sent until you confirm"} side="bottom" className="w-full">
            <button
              type="button"
              onClick={onRequest}
              disabled={blocked !== null}
              className="flex w-full items-center justify-center gap-2 rounded-lg bg-violet-600 px-4 py-2.5 text-sm font-semibold text-white shadow-sm hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-300"
            >
              <SlackIcon size={16} />
              Request review on Slack
            </button>
          </Tooltip>
        )}
        {error ? (
          <p className="text-xs text-slate-500">Slack: couldn't read the threads ({error})</p>
        ) : (
          !onRequest && <p className="text-xs text-slate-500">Slack: no request yet</p>
        )}
      </div>
    );
  }
  const [root, ...replies] = thread.messages;
  const shown = replies.slice(-MAX_REPLIES);
  const more = replies.length - shown.length;
  return (
    <section aria-label="Slack request" className="rounded-lg border border-slate-800 bg-slate-900/50 p-3 text-xs">
      <header className="flex items-center gap-2">
        <SlackIcon size={13} />
        <span className="font-medium text-slate-200">{channelLabel(thread)}</span>
        <span className="min-w-0 truncate text-slate-500">
          · requested {shortTime(thread.requested_at)}
          {thread.bump_count > 0 && ` · bumped ${thread.bump_count}×`}
          {thread.status === "closed" && " · closed"}
          {threads && threads.length > 1 && ` · ${threads.length} threads`}
        </span>
        <span className="ml-auto flex shrink-0 items-center gap-2">
          {thread.permalink && (
            <button type="button" className="text-violet-300 hover:underline" onClick={() => onOpen(thread.permalink ?? "")}>
              Open ↗
            </button>
          )}
          {onRequest && (
            <Tooltip label={blocked ?? "Bump on Slack"} shortcut="R" detail={blocked ? "Reconnect it in Settings › Accounts" : "Preview the bump first; pick another channel there to send a new request instead"} side="bottom" align="end">
              <button type="button" onClick={onRequest} disabled={blocked !== null} className="flex items-center gap-1.5 rounded-md bg-violet-600 px-2.5 py-1 font-medium text-white hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50">
                <SlackIcon size={12} />
                Bump
              </button>
            </Tooltip>
          )}
        </span>
      </header>
      {thread.error && <p className="mt-1 text-amber-200">{thread.error}</p>}
      {root && <p className="mt-2 line-clamp-2 whitespace-pre-line text-slate-300">{slackText(root.text)}</p>}
      {shown.length > 0 && (
        <ol className="mt-2 space-y-1.5 border-l border-slate-700 pl-2.5">
          {more > 0 && <li className="text-slate-500">+{more} earlier repl{more === 1 ? "y" : "ies"}</li>}
          {shown.map((message) => (
            <li key={message.ts}>
              <span className="text-slate-200">{message.author}</span>
              <span className="text-slate-500"> · {shortTime(message.at)}</span>
              <p className="line-clamp-2 text-slate-400">{slackText(message.text)}</p>
            </li>
          ))}
        </ol>
      )}
      {root && replies.length === 0 && <p className="mt-2 text-slate-500">No replies yet.</p>}
    </section>
  );
}
