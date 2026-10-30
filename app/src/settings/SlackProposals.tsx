import { useCallback, useEffect, useState } from "react";
import type { Api, SlackProposal } from "../api";
import { errorMessage } from "../api";
import { ago } from "../list/model";

interface Props {
  api: Pick<Api, "mcpSlackProposals" | "decideMcpSlackProposal">;
  /** Slack sends is on, so approving can go out. */
  canSend: boolean;
}

/** How often the list re-reads, so a new proposal shows up. */
export const PROPOSALS_POLL_MS = 3_000;

const button = "rounded border border-slate-700 px-2.5 py-1 text-xs hover:bg-slate-800 disabled:opacity-50";

const VERB: Record<string, string> = {
  slack_send_message: "Send",
  slack_schedule_message: "Schedule",
  slack_send_message_draft: "Draft",
  slack_add_reaction: "React",
};

const STATUS: Record<SlackProposal["status"], string> = {
  pending: "border-amber-700/60 bg-amber-950/40 text-amber-200",
  sent: "border-emerald-700/60 bg-emerald-950/40 text-emerald-200",
  failed: "border-rose-700/60 bg-rose-950/40 text-rose-200",
  rejected: "border-slate-700 bg-slate-900 text-slate-300",
};

function why(p: SlackProposal): string | null {
  if (p.auto_sent) {
    return "Sent at once, without approval";
  }
  if (!p.held) {
    return null;
  }
  switch (p.held.reason) {
    case "approve_all":
      return "Every write waits for approval";
    case "not_listed":
      return "Channel not on the auto-send list";
    case "read_other_workspace":
      return `Auto-send blocked: this agent read ${p.held.workspaces.join(", ")} first`;
    default: {
      const never: never = p.held;
      return never;
    }
  }
}

function body(p: SlackProposal): string {
  const args = p.arguments;
  if (typeof args.message === "string") {
    return args.message;
  }
  if (typeof args.emoji === "string") {
    return `:${args.emoji}: on ${String(args.message_ts)}`;
  }
  return JSON.stringify(args);
}

function slackError(error: unknown): string {
  return typeof error === "string" ? error : JSON.stringify(error);
}

/** Slack writes agents proposed: approve or reject each one. */
export function SlackProposals({ api, canSend }: Props) {
  const [items, setItems] = useState<SlackProposal[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [deciding, setDeciding] = useState<number | null>(null);

  const load = useCallback(() => {
    api
      .mcpSlackProposals()
      .then((next) => {
        setItems(next);
        setError(null);
      })
      .catch((err: unknown) => setError(errorMessage(err)));
  }, [api]);

  useEffect(() => {
    load();
    const timer = setInterval(load, PROPOSALS_POLL_MS);
    return () => clearInterval(timer);
  }, [load]);

  function decide(id: number, approve: boolean) {
    setDeciding(id);
    api
      .decideMcpSlackProposal(id, approve)
      .then((next) => setItems((list) => list.map((p) => (p.id === id ? next : p))))
      .catch((err: unknown) => setError(errorMessage(err)))
      .finally(() => setDeciding(null));
  }

  const pending = items.filter((p) => p.status === "pending");
  return (
    <section aria-labelledby="agents-proposals">
      <h2 id="agents-proposals" className="text-base font-semibold">
        Slack proposals {pending.length > 0 && <span className="text-amber-300">({pending.length} waiting)</span>}
      </h2>
      <p className="mt-1 max-w-prose text-xs text-slate-400">Kept until PR Nexus quits. Nothing reaches Slack until you approve it.</p>
      {error && (
        <p role="alert" className="mt-2 text-sm text-rose-300">
          {error}
        </p>
      )}
      {items.length === 0 ? (
        <p className="mt-2 text-sm text-slate-400">No proposals yet.</p>
      ) : (
        <ul className="mt-2 space-y-2" aria-label="Slack proposals">
          {items.map((p) => (
            <li key={p.id} className="rounded border border-slate-800 bg-slate-900/40 p-3 text-sm">
              <div className="flex items-center gap-2">
                <span className={`rounded border px-1.5 py-0.5 text-[11px] leading-none ${STATUS[p.status]}`}>{p.status}</span>
                <span className="font-medium">{VERB[p.tool] ?? p.tool}</span>
                <span className="min-w-0 truncate font-mono text-xs text-slate-400">{p.channel_id}</span>
                <span className="ml-auto shrink-0 text-xs text-slate-500">
                  {p.client ?? "agent"} · {ago(p.created_at, new Date())}
                </span>
              </div>
              <p className="mt-1 whitespace-pre-wrap break-words text-slate-200">{body(p)}</p>
              {why(p) && <p className="mt-1 text-xs text-slate-400">{why(p)}</p>}
              {p.status === "failed" && p.error != null && <p className="mt-1 text-xs text-rose-300">Slack: {slackError(p.error)}</p>}
              {p.status === "pending" && (
                <div className="mt-2 flex gap-2">
                  <button
                    type="button"
                    className={button}
                    disabled={!canSend || deciding !== null}
                    aria-label={`Approve proposal ${p.id}`}
                    onClick={() => decide(p.id, true)}
                  >
                    Approve
                  </button>
                  <button
                    type="button"
                    className={button}
                    disabled={deciding !== null}
                    aria-label={`Reject proposal ${p.id}`}
                    onClick={() => decide(p.id, false)}
                  >
                    Reject
                  </button>
                  {!canSend && <span className="self-center text-xs text-slate-400">Turn on Slack sends to approve.</span>}
                </div>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
