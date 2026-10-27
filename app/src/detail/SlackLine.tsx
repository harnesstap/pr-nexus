import { useEffect, useState } from "react";
import type { Api, ContextThread } from "../api";
import { channelLabel, errorMessage } from "../api";
import { SlackIcon } from "../icons";
import { shortTime, slackText } from "../list/slackText";

interface Props {
  api: Api;
  prId: number;
  onOpen: (url: string) => void;
  /** Opens the request or bump preview. */
  onRequest?: () => void;
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
 * The panel's Slack section (§7.10): one quiet line until a request exists, then the
 * latest request with its newest replies.
 */
export function SlackLine({ api, prId, onOpen, onRequest }: Props) {
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

  if (error) {
    return <p className="flex items-center gap-2 text-xs text-slate-500"><SlackIcon size={13} active={false} />Slack: couldn't read the threads ({error})</p>;
  }
  if (!threads) {
    return <div className="h-4 w-48 animate-pulse rounded bg-slate-800" aria-label="Loading Slack" />;
  }
  const thread = latestThread(threads);
  if (!thread) {
    return (
      <p className="flex items-center gap-2 text-xs text-slate-500" data-testid="slack-empty">
        <SlackIcon size={13} active={false} />
        <span>Slack: no request yet</span>
        {onRequest && (
          <>
            <span aria-hidden="true">·</span>
            <button type="button" className="text-violet-300 hover:underline" onClick={onRequest}>
              Request
            </button>
          </>
        )}
      </p>
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
        <span className="text-slate-500">
          · requested {shortTime(thread.requested_at)}
          {thread.bump_count > 0 && ` · bumped ${thread.bump_count}×`}
          {thread.status === "closed" && " · closed"}
        </span>
        {threads.length > 1 && <span className="text-slate-500">· {threads.length} threads</span>}
        {thread.permalink && (
          <button type="button" className="ml-auto text-violet-300 hover:underline" onClick={() => onOpen(thread.permalink ?? "")}>
            Open ↗
          </button>
        )}
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
