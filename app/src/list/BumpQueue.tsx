import { useEffect, useState } from "react";
import type { Api, BulkCard, BulkMessage, BulkPreview, BulkRequest, BulkResult, DueBump } from "../api";
import { channelLabel, errorMessage } from "../api";
import { CheckNotes } from "../slack/OwnerGroups";
import { CopyMessageButton } from "../slack/CopyMessageButton";
import { Mrkdwn } from "../slack/mrkdwn";
import { plainText } from "../textInput";
import type { Toast } from "./model";
import { quietLabel, sameIds, samePlan } from "./bumpPlan";

interface Props {
  api: Api;
  onOpen: (url: string) => void;
  onNotice?: (toast: Toast) => void;
  /** The sidebar count should be read again. */
  onChanged?: () => void;
}

function requestBody(ids: number[], notes: Record<number, string>): BulkRequest {
  return {
    pr_ids: ids,
    target: null,
    single_top_level: [],
    notes: ids.filter((id) => (notes[id] ?? "").trim() !== "").map((id) => ({ pr_id: id, text: notes[id] ?? "" })),
  };
}

function actionLabel(action: BulkMessage["action"]): string {
  switch (action) {
    case "request":
      return "New request";
    case "bump":
      return "Bump in thread";
    case "remind":
      return "New reminder";
    default: {
      const unexpected: never = action;
      return unexpected;
    }
  }
}

function summary(result: BulkResult): string {
  const sent = result.results.filter((item) => item.outcome === "sent").length;
  const failed = result.results.filter((item) => item.outcome === "failed").length;
  if (failed === 0) {
    return sent === 1 ? "Bump sent" : `${sent} bumps sent`;
  }
  return `Slack: ${sent} sent · ${failed} failed`;
}

/**
 * Bumps that are due (§7.14). The channel and the message are the bulk
 * preview's. Send posts only after a click, and only when a fresh preview
 * still matches what is on screen.
 */
const EMPTY: BulkPreview = { cards: [], skipped: [], pr_warnings: [] };
const NO_DUE: DueBump[] = [];

export function BumpQueue({ api, onOpen, onNotice, onChanged }: Props) {
  const [items, setItems] = useState<DueBump[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  /** Null means every due pull request. A selection narrows the preview before it is sent. */
  const [scope, setScope] = useState<number[] | null>(null);
  const [notes, setNotes] = useState<Record<number, string>>({});
  const [checked, setChecked] = useState<number[]>([]);
  /** Last preview that arrived. Send is allowed only when `key` is the current request. */
  const [shownPlan, setShownPlan] = useState<{ key: string; body: BulkPreview } | null>(null);
  const [planError, setPlanError] = useState<{ key: string; message: string } | null>(null);
  const [sending, setSending] = useState(false);
  const [hint, setHint] = useState<string | null>(null);

  useEffect(() => {
    const load = api.dueBumps;
    if (!load) {
      return;
    }
    let cancel = false;
    void load().then(
      (next) => {
        if (!cancel) {
          setItems(next);
          setLoadError(null);
        }
      },
      (err: unknown) => {
        if (!cancel) {
          setItems([]);
          setLoadError(errorMessage(err));
        }
      },
    );
    return () => {
      cancel = true;
    };
  }, [api, reload]);

  const queue = api.dueBumps ? items : NO_DUE;
  const allIds = queue?.map((item) => item.pr_id) ?? [];
  const scopeIds = (scope ?? allIds).filter((id) => allIds.includes(id));
  const scopeKey = scopeIds.join(",");
  const notesKey = scopeIds.map((id) => `${id}:${notes[id] ?? ""}`).join("\n");
  const requestKey = `${scopeKey}\n${notesKey}`;

  useEffect(() => {
    if (queue === null || scopeIds.length === 0) {
      return;
    }
    let cancel = false;
    const timer = window.setTimeout(() => {
      void api.bulkPreview({ ...requestBody(scopeIds, notes), cached_names: true }).then(
        (next) => {
          if (!cancel) {
            setShownPlan({ key: requestKey, body: next });
            setPlanError(null);
          }
        },
        (err: unknown) => {
          if (!cancel) {
            setPlanError({ key: requestKey, message: errorMessage(err) });
          }
        },
      );
    }, 250);
    return () => {
      cancel = true;
      window.clearTimeout(timer);
    };
    // requestKey is the input. scopeIds and notes are the values it was built from.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api, queue, requestKey]);

  const matches = shownPlan?.key === requestKey;
  const failed = planError?.key === requestKey ? planError.message : null;
  const planning = queue !== null && scopeIds.length > 0 && !matches && failed === null;
  const preview = queue !== null && scopeIds.length === 0 ? EMPTY : shownPlan?.body ?? null;
  const quiet = new Map(queue?.map((item) => [item.pr_id, item.quiet_hours]) ?? []);
  const canSend = matches && !sending && (shownPlan?.body.cards.length ?? 0) > 0;

  async function post(ids: number[]) {
    if (!matches || !shownPlan || ids.length === 0) {
      return;
    }
    setSending(true);
    try {
      const body = requestBody(ids, notes);
      const fresh = await api.bulkPreview({ ...body, cached_names: false });
      if (!samePlan(shownPlan.body.cards, fresh.cards)) {
        setShownPlan({ key: requestKey, body: fresh });
        setHint("The destination or the message changed. Check it, then send.");
        return;
      }
      const result = await api.sendBulk(body, fresh.cards.map((card) => card.fingerprint));
      setHint(null);
      onNotice?.({ text: summary(result) });
      setScope(null);
      setChecked([]);
      setReload((value) => value + 1);
      onChanged?.();
    } catch (err) {
      setHint(errorMessage(err));
    } finally {
      setSending(false);
    }
  }

  function sendAll() {
    if (!queue) {
      return;
    }
    const all = queue.map((item) => item.pr_id);
    if (!sameIds(all, scopeIds)) {
      setScope(null);
      setHint("Showing every bump that is due. Send again to post them.");
      return;
    }
    void post(all);
  }

  function sendSelected() {
    if (checked.length === 0) {
      return;
    }
    if (!sameIds(checked, scopeIds)) {
      setScope(checked);
      setHint("This is what will be sent. Send again to post it.");
      return;
    }
    void post(checked);
  }

  async function hold(kind: "snooze" | "skip", prId: number) {
    const run = kind === "snooze" ? api.snoozeBump : api.skipBump;
    if (!run) {
      return;
    }
    try {
      const next = await run(prId);
      setItems(next);
      setScope(null);
      setChecked((current) => current.filter((id) => id !== prId));
      setHint(null);
      onChanged?.();
    } catch (err) {
      setHint(errorMessage(err));
    }
  }

  function toggle(prId: number) {
    setChecked((current) => (current.includes(prId) ? current.filter((id) => id !== prId) : [...current, prId]));
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-12 shrink-0 items-center gap-2 overflow-x-auto border-b border-slate-800 px-4">
        <span className="shrink-0 text-sm text-slate-200">{queue === null ? "Bumps due" : `${queue.length} due`}</span>
        <button
          type="button"
          className="shrink-0 whitespace-nowrap rounded bg-violet-500 px-2.5 py-1 text-sm font-medium text-white disabled:opacity-40"
          disabled={!canSend || checked.length === 0}
          onClick={sendSelected}
        >
          Send selected
        </button>
        <button
          type="button"
          className="shrink-0 whitespace-nowrap rounded border border-slate-700 px-2.5 py-1 text-sm text-slate-100 disabled:opacity-40"
          disabled={!canSend}
          onClick={sendAll}
        >
          Send all
        </button>
        {planning && <span className="truncate text-xs text-slate-500">Updating the preview…</span>}
        {hint && !planning && (
          <span role="status" className="truncate text-xs text-slate-400">
            {hint}
          </span>
        )}
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-4 py-3">
        {loadError && (
          <p className="text-sm text-slate-400">
            Couldn't load bumps that are due.{" "}
            <button type="button" className="underline" onClick={() => setReload((value) => value + 1)}>
              Retry
            </button>
          </p>
        )}
        {queue !== null && queue.length === 0 && !loadError && (
          <div className="max-w-lg text-sm text-slate-400">
            <p>No bumps are due.</p>
            <p className="mt-1 text-xs text-slate-500">
              A bump becomes due after the last Slack request or bump, once the wait in Settings has passed and nobody has reviewed. Nothing is sent on its own.
            </p>
          </div>
        )}
        {failed && (
          <p className="text-sm text-slate-400">
            Couldn't load the message preview.{" "}
            <button type="button" className="underline" onClick={() => setReload((value) => value + 1)}>
              Retry
            </button>
          </p>
        )}
        <div className="space-y-4">
          {preview?.cards.map((card) => (
            <Card
              key={`${card.target.workspace}:${card.target.channel_id}:${card.messages.flatMap((message) => message.prs.map((pr) => pr.pr_id)).join(",")}`}
              card={card}
              quiet={quiet}
              notes={notes}
              checked={checked}
              warnings={preview.pr_warnings}
              onToggle={toggle}
              onNote={(id, text) => setNotes((current) => ({ ...current, [id]: text }))}
              onSnooze={(id) => void hold("snooze", id)}
              onSkip={(id) => void hold("skip", id)}
              onOpen={onOpen}
              onNotice={onNotice}
            />
          ))}
        </div>
        {preview && preview.skipped.length > 0 && (
          <section className="mt-4" aria-label="Not included">
            <h2 className="text-xs font-medium uppercase tracking-wide text-slate-500">Not included</h2>
            <ul className="mt-2 space-y-2">
              {preview.skipped.map((skip) => (
                <li key={skip.pr.pr_id} className="text-sm text-slate-400">
                  <span>
                    {skip.pr.repo}#{skip.pr.number} {skip.pr.title}
                  </span>
                  <span className="mt-0.5 block text-xs text-slate-500">{skip.detail}</span>
                  <HoldButtons onSnooze={() => void hold("snooze", skip.pr.pr_id)} onSkip={() => void hold("skip", skip.pr.pr_id)} />
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>
    </div>
  );
}

function Card({
  card,
  quiet,
  notes,
  checked,
  warnings,
  onToggle,
  onNote,
  onSnooze,
  onSkip,
  onOpen,
  onNotice,
}: {
  card: BulkCard;
  quiet: Map<number, number>;
  notes: Record<number, string>;
  checked: number[];
  warnings: BulkPreview["pr_warnings"];
  onToggle: (id: number) => void;
  onNote: (id: number, text: string) => void;
  onSnooze: (id: number) => void;
  onSkip: (id: number) => void;
  onOpen: (url: string) => void;
  onNotice?: (toast: Toast) => void;
}) {
  const label = channelLabel(card.target);
  const where = [label, card.target.workspace_name || card.target.workspace].filter(Boolean).join(" · ");
  const prIds = card.messages.flatMap((message) => message.prs.map((pr) => pr.pr_id));
  return (
            <section aria-label={label} data-channel-id={card.target.channel_id} className="rounded-md border border-slate-800 p-3">
      <header className="flex flex-wrap items-baseline gap-x-2 text-sm">
        <span className="font-medium text-slate-100">{where}</span>
        {card.messages.map((message, index) => (
          <span key={`${message.action}:${index}`} className="text-slate-400">
            {actionLabel(message.action)}
          </span>
        ))}
        {card.messages.map((message) =>
          message.permalink ? (
            <button key={message.permalink} type="button" className="text-slate-400 underline" onClick={() => onOpen(message.permalink!)}>
              Open thread
            </button>
          ) : null,
        )}
      </header>
      {(card.notes ?? []).map((note) => (
        <p key={note} className="mt-1 text-xs text-sky-300">
          {note}
        </p>
      ))}
      {card.warnings.map((warning) => (
        <p key={warning} className="mt-1 text-xs text-slate-400">
          {warning}
        </p>
      ))}
      <CheckNotes warnings={warnings.filter((warning) => prIds.includes(warning.pr_id))} />
      <div className="mt-2 space-y-2">
        {card.messages.map((message, index) => (
          <div key={`${message.action}:${index}`} className="relative rounded border border-slate-800 bg-slate-950 p-3 pr-10">
            <Mrkdwn label="Message preview" text={message.text} onOpen={onOpen} />
            <CopyMessageButton text={message.text} onNotice={onNotice} />
          </div>
        ))}
      </div>
      <ul className="mt-2 space-y-2">
        {card.messages.flatMap((message) =>
          message.prs.map((pr) => (
            <li key={`${card.target.channel_id}:${pr.pr_id}`} className="flex items-start gap-2">
              <input
                type="checkbox"
                className="mt-1"
                checked={checked.includes(pr.pr_id)}
                aria-label={`Select ${pr.repo}#${pr.number}`}
                onChange={() => onToggle(pr.pr_id)}
              />
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm text-slate-100">
                  {pr.repo}#{pr.number} {pr.title}
                  {quiet.has(pr.pr_id) && <span className="ml-2 text-xs text-slate-500">Quiet {quietLabel(quiet.get(pr.pr_id) ?? 0)}</span>}
                </p>
                <label className="mt-1 block text-xs text-slate-500">
                  Note
                  <textarea
                    {...plainText}
                    aria-label={`Note for ${pr.repo}#${pr.number}`}
                    rows={2}
                    value={notes[pr.pr_id] ?? ""}
                    onChange={(event) => onNote(pr.pr_id, event.target.value)}
                    className="mt-1 block w-full resize-y rounded border border-slate-700 bg-slate-950 px-2 py-1 text-sm text-slate-100"
                  />
                </label>
                <HoldButtons onSnooze={() => onSnooze(pr.pr_id)} onSkip={() => onSkip(pr.pr_id)} />
              </div>
            </li>
          )),
        )}
      </ul>
    </section>
  );
}

function HoldButtons({ onSnooze, onSkip }: { onSnooze: () => void; onSkip: () => void }) {
  return (
    <div className="mt-1 flex gap-2 text-xs">
      <button type="button" className="text-slate-400 underline" onClick={onSnooze}>
        Snooze
      </button>
      <button type="button" className="text-slate-400 underline" onClick={onSkip}>
        Skip
      </button>
    </div>
  );
}
