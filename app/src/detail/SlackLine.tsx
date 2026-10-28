import { useEffect, useState } from "react";
import type { Api, ContextThread, SlackWorkspace } from "../api";
import { channelLabel, errorMessage } from "../api";
import { ExternalIcon, SlackLogo } from "../icons";
import { ago } from "../list/model";
import { shortTime, slackText } from "../list/slackText";
import { slackActionBlock } from "../list/SyncBanners";
import { Tooltip } from "../Tooltip";

interface Props {
  api: Api;
  prId: number;
  /** Settings → minimum hours between bumps; 0 means no limit. */
  minBumpHours?: number;
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
  return [...threads].sort((a, b) => (b.requested_at ?? "").localeCompare(a.requested_at ?? "") || b.root_ts.localeCompare(a.root_ts))[0] ?? null;
}

export interface BumpView {
  /** "6h ago": the last bump, or the request when never bumped. */
  last: string | null;
  /** Set when the minimum interval hasn't passed: why Bump is disabled. */
  tooSoon: string | null;
  /** Whole hours until the next bump is allowed; null when it already is. */
  wait: number | null;
}

/** When the thread was last poked, and whether the settings allow another bump yet. */
export function bumpView(thread: Pick<ContextThread, "last_bumped_at" | "requested_at">, minHours: number, now: Date): BumpView {
  const at = thread.last_bumped_at ?? thread.requested_at;
  if (!at) {
    return { last: null, tooSoon: null, wait: null };
  }
  const elapsed = (now.getTime() - new Date(at).getTime()) / 3_600_000;
  if (minHours <= 0 || elapsed >= minHours) {
    return { last: ago(at, now), tooSoon: null, wait: null };
  }
  const wait = Math.max(1, Math.ceil(minHours - elapsed));
  return {
    last: ago(at, now),
    wait,
    tooSoon: `Settings allow one bump every ${minHours}h. Try again in about ${wait}h.`,
  };
}

/**
 * The panel's Slack action (§7.10): one big "Request review on Slack" button until a
 * request exists, then the latest request with its newest replies and a Bump button.
 * Both open the preview, where another channel can still get a new request.
 */
export function SlackLine({ api, prId, minBumpHours = 0, onOpen, onRequest, workspaces = [] }: Props) {
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
          <Tooltip
            label={blocked ?? "Request review on Slack"}
            shortcut="R"
            detail={blocked ? "Reconnect it in Settings › Accounts" : "Shows the exact message first; nothing is sent until you confirm"}
            side="bottom"
            className="w-full"
          >
            <button
              type="button"
              onClick={onRequest}
              disabled={blocked !== null}
              className="flex w-full items-center justify-center gap-2 rounded-lg bg-violet-600 px-4 py-2.5 text-sm font-semibold text-white shadow-sm hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-300"
            >
              <SlackLogo size={16} />
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
  const bump = bumpView(thread, minBumpHours, new Date());
  const channel = channelLabel(thread);
  const team = workspaces.find((w) => w.domain === thread.workspace || w.team_name === thread.workspace)?.team_name ?? thread.workspace;
  const meta = [
    thread.bump_count > 0
      ? `${thread.last_bumped_at ? `Bumped ${shortTime(thread.last_bumped_at)}` : `Requested ${shortTime(thread.requested_at)}`} · ${thread.bump_count} bump${thread.bump_count === 1 ? "" : "s"}`
      : `Requested ${shortTime(thread.requested_at)}`,
    team,
    thread.status === "closed" ? "closed" : null,
    threads && threads.length > 1 ? `${threads.length} threads` : null,
  ]
    .filter(Boolean)
    .join(" · ");
  const shown = replies.slice(-MAX_REPLIES);
  const more = replies.length - shown.length;
  const disabledReason =
    blocked ??
    (bump.wait !== null ? `Bump available in ${bump.wait}h${bump.last ? ` (${thread.last_bumped_at ? "last bump" : "requested"} ${bump.last})` : ""}` : null);
  return (
    <section aria-label="Slack request" className="flex flex-col gap-2 rounded-lg border border-slate-800 bg-slate-900/70 p-3 text-xs">
      <header className="flex min-w-0 items-start gap-2">
        <span className="mt-0.5 shrink-0">
          <SlackLogo size={16} />
        </span>
        <div className="min-w-0 flex-1">
          <Tooltip label={channel} detail={team || undefined} align="start" className="max-w-full">
            <p tabIndex={0} data-testid="slack-channel" className="truncate text-sm font-medium text-slate-100 outline-none">
              {channel}
            </p>
          </Tooltip>
          <p data-testid="slack-meta" className="truncate text-slate-500">
            {meta}
          </p>
        </div>
        {thread.permalink && (
          <Tooltip label="Open in Slack" align="end">
            <button
              type="button"
              aria-label="Open in Slack"
              className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-slate-400 hover:bg-slate-800 hover:text-slate-100"
              onClick={() => onOpen(thread.permalink ?? "")}
            >
              <ExternalIcon size={14} />
            </button>
          </Tooltip>
        )}
      </header>
      {thread.error && <p className="text-amber-200">{thread.error}</p>}
      {root && <Quote text={slackText(root.text)} />}
      {shown.length > 0 ? (
        <ol aria-label="Replies" className="space-y-1.5">
          {more > 0 && (
            <li className="text-slate-500">
              +{more} earlier repl{more === 1 ? "y" : "ies"}
            </li>
          )}
          {shown.map((message) => (
            <li key={message.ts}>
              <span className="text-slate-200">{message.author}</span>
              <span className="text-slate-500"> · {shortTime(message.at)}</span>
              <p className="line-clamp-2 text-slate-400">
                <InlineCode text={slackText(message.text)} />
              </p>
            </li>
          ))}
        </ol>
      ) : (
        root && <p className="text-slate-500">No replies yet.</p>
      )}
      {onRequest && (
        <footer className="flex flex-col gap-1 pt-1">
          <Tooltip
            label={blocked ?? (disabledReason ? "Too soon to bump" : "Bump on Slack")}
            shortcut="R"
            detail={
              blocked
                ? "Reconnect it in Settings › Accounts"
                : (bump.tooSoon ?? "Preview the bump first; pick another channel there to send a new request instead")
            }
            side="top"
            className="w-full"
          >
            <button
              type="button"
              data-testid={disabledReason ? "bump-disabled" : "bump"}
              disabled={disabledReason !== null}
              onClick={onRequest}
              className="flex w-full items-center justify-center gap-2 rounded-lg bg-violet-600 px-4 py-2 text-sm font-semibold text-white enabled:hover:bg-violet-500 disabled:cursor-not-allowed disabled:bg-slate-800 disabled:text-slate-500"
            >
              <SlackLogo size={14} variant={disabledReason ? "mono" : "color"} />
              {!disabledReason && thread.last_bumped_at && bump.last ? `Bump · last ${bump.last}` : "Bump"}
            </button>
          </Tooltip>
          {disabledReason && <p className="text-center text-slate-500">{disabledReason}</p>}
        </footer>
      )}
    </section>
  );
}

/** Lines of the sent message shown before "Show more". */
const QUOTE_LINES = 3;

/** The sent message as a quote, clamped with a toggle when it runs long. */
function Quote({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  const long = text.split("\n").length > QUOTE_LINES || text.length > 180;
  return (
    <blockquote className="rounded-r-md border-l-2 border-slate-600 bg-slate-800/40 px-2.5 py-1.5 text-slate-300">
      <p className={`whitespace-pre-line break-words ${open ? "" : "line-clamp-3"}`}>
        <InlineCode text={text} />
      </p>
      {long && (
        <button type="button" className="mt-0.5 text-violet-300 hover:text-violet-200" aria-expanded={open} onClick={() => setOpen(!open)}>
          {open ? "Show less" : "Show more"}
        </button>
      )}
    </blockquote>
  );
}

/** `backticked` spans (PR titles are often wrapped that way) render as inline code. */
export function InlineCode({ text }: { text: string }) {
  const parts = text.split(/(`[^`\n]+`)/);
  return (
    <>
      {parts.map((part, index) =>
        part.length > 2 && part.startsWith("`") && part.endsWith("`") ? (
          <code key={index} className="rounded bg-slate-800 px-1 font-mono text-[11px] text-slate-200">
            {part.slice(1, -1)}
          </code>
        ) : (
          part
        ),
      )}
    </>
  );
}
