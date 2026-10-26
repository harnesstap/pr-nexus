import { useCallback, useEffect, useState } from "react";
import type { Api, BulkCard, BulkPreview, BulkRequest, BulkResult, ChannelKey, ReviewTarget, SlackWorkspace } from "../api";
import { channelLabel, errorMessage } from "../api";

interface Props {
  api: Api;
  prIds: number[];
  onClose: () => void;
  /** Fresh indicators for the PRs that were posted. */
  /** After each Send (and Retry failed): updates indicators and the closing toast. */
  onSent: (done: BulkResult) => void;
  onOpen: (url: string) => void;
}

const ACTION_LABEL = {
  request: "New request",
  bump: "Bump in thread",
  remind: "New reminder (replaces the old threads)",
} as const;

function sameKey(a: ChannelKey, b: ChannelKey) {
  return a.workspace === b.workspace && a.channel_id === b.channel_id;
}

/**
 * Bulk request/bump on Slack (§7.9). Shows every message, grouped by channel,
 * before anything is posted. Send posts one message at a time and reports a
 * result per PR and channel; nothing already posted is rolled back.
 */
export function BulkDialog({ api, prIds, onClose, onSent, onOpen }: Props) {
  const [ids, setIds] = useState(prIds);
  const [oneChannel, setOneChannel] = useState(false);
  const [manual, setManual] = useState<ReviewTarget>({ workspace: "", channel_id: "", channel_name: "" });
  const [workspaces, setWorkspaces] = useState<SlackWorkspace[]>([]);
  const [single, setSingle] = useState<ChannelKey[]>([]);
  const [preview, setPreview] = useState<BulkPreview | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [result, setResult] = useState<BulkResult | null>(null);

  const request = useCallback(
    (next: Partial<{ ids: number[]; oneChannel: boolean; single: ChannelKey[] }> = {}): BulkRequest => ({
      pr_ids: next.ids ?? ids,
      target: (next.oneChannel ?? oneChannel) ? manual : null,
      single_top_level: next.single ?? single,
    }),
    [ids, oneChannel, manual, single],
  );

  const load = useCallback(
    async (body: BulkRequest) => {
      setLoading(true);
      setError(null);
      setPreview(null);
      try {
        setPreview(await api.bulkPreview(body));
      } catch (err) {
        setError(errorMessage(err));
      } finally {
        setLoading(false);
      }
    },
    [api],
  );

  useEffect(() => {
    let cancel = false;
    void (async () => {
      const signedIn = await api.slackWorkspaces().catch(() => [] as SlackWorkspace[]);
      if (cancel) {
        return;
      }
      setWorkspaces(signedIn);
      const first = signedIn.find((item) => item.signed_in) ?? signedIn[0];
      setManual((current) => ({ ...current, workspace: first?.domain ?? "" }));
    })();
    api
      .bulkPreview({ pr_ids: prIds, target: null, single_top_level: [] })
      .then((planned) => !cancel && setPreview(planned))
      .catch((err: unknown) => !cancel && setError(errorMessage(err)))
      .finally(() => !cancel && setLoading(false));
    return () => {
      cancel = true;
    };
  }, [api, prIds]);

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape" && !sending) {
        event.stopPropagation();
        onClose();
      }
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose, sending]);

  function toggleSingle(card: BulkCard) {
    const key = { workspace: card.target.workspace, channel_id: card.target.channel_id };
    const next = single.some((item) => sameKey(item, key)) ? single.filter((item) => !sameKey(item, key)) : [...single, key];
    setSingle(next);
    void load(request({ single: next }));
  }

  function routeMode(next: boolean) {
    setOneChannel(next);
    if (!next) {
      void load(request({ oneChannel: false }));
    } else {
      setPreview(null);
    }
  }

  async function send() {
    if (!preview) {
      return;
    }
    setSending(true);
    setError(null);
    try {
      const done = await api.sendBulk(
        request(),
        preview.cards.map((card) => card.fingerprint),
      );
      setResult(done);
      onSent(done);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setSending(false);
    }
  }

  function retryFailed() {
    if (!result) {
      return;
    }
    const failed = [...new Set(result.results.filter((item) => item.outcome === "failed").map((item) => item.pr.pr_id))];
    setIds(failed);
    setResult(null);
    void load(request({ ids: failed }));
  }

  const input = "mt-1 w-full rounded border border-slate-700 bg-slate-950 px-2 py-1.5";
  const messageCount = preview?.cards.reduce((sum, card) => sum + card.messages.length, 0) ?? 0;
  const unsigned = preview?.cards.filter((card) => !card.signed_in) ?? [];
  const failedCount = result?.results.filter((item) => item.outcome === "failed").length ?? 0;

  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center overflow-auto bg-black/50 p-8"
      onMouseDown={(event) => event.target === event.currentTarget && !sending && onClose()}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="bulk-dialog-title"
        className="w-full max-w-3xl rounded-lg border border-slate-700 bg-slate-900 p-5 text-sm text-slate-200 shadow-xl"
      >
        <h2 id="bulk-dialog-title" className="text-base font-semibold">
          {result ? "Slack results" : `Request / bump ${ids.length} PRs on Slack`}
        </h2>
        {result ? (
          <Results result={result} onOpen={onOpen} />
        ) : (
          <>
            <fieldset className="mt-3 flex flex-wrap gap-4 text-xs text-slate-300">
              <label className="flex items-center gap-1">
                <input type="radio" name="route" checked={!oneChannel} onChange={() => routeMode(false)} />
                Each PR's team channels
              </label>
              <label className="flex items-center gap-1">
                <input type="radio" name="route" checked={oneChannel} onChange={() => routeMode(true)} />
                All to one channel
              </label>
            </fieldset>
            {oneChannel && (
              <div className="mt-2 grid grid-cols-3 gap-2">
                <label className="block">
                  Workspace
                  <select className={input} value={manual.workspace} onChange={(event) => setManual({ ...manual, workspace: event.target.value })}>
                    {workspaces.length === 0 && <option value="">No workspace connected</option>}
                    {workspaces.map((item) => (
                      <option key={item.id} value={item.domain}>
                        {item.team_name}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="block">
                  Channel or DM ID
                  <input
                    className={input}
                    placeholder="C0123456789 or D0123456789"
                    value={manual.channel_id}
                    onChange={(event) => setManual({ ...manual, channel_id: event.target.value })}
                  />
                </label>
                <label className="block">
                  Channel name
                  <input className={input} placeholder="#reviews" value={manual.channel_name} onChange={(event) => setManual({ ...manual, channel_name: event.target.value })} />
                </label>
                <button
                  type="button"
                  className="col-span-3 justify-self-start rounded border border-slate-600 px-3 py-1.5"
                  onClick={() => void load(request({ oneChannel: true }))}
                >
                  Preview
                </button>
              </div>
            )}
            {loading && <p className="mt-4 text-slate-400">Planning messages…</p>}
            {preview && (
              <div className="mt-4 space-y-3">
                {preview.cards.length === 0 && <p className="text-slate-400">Nothing to send.</p>}
                {preview.cards.map((card) => (
                  <section
                    key={card.fingerprint}
                    aria-label={`${card.target.workspace} ${channelLabel(card.target)}`}
                    className="rounded border border-slate-700 p-3"
                  >
                    <header className="flex items-center justify-between gap-2">
                      <h3 className="font-medium">
                        {card.target.workspace} · {channelLabel(card.target)}
                      </h3>
                      {(card.can_switch || card.single_top_level) && (
                        <label className="flex items-center gap-1 text-xs text-slate-300">
                          <input type="checkbox" checked={card.single_top_level} onChange={() => toggleSingle(card)} />
                          One top-level request for all
                        </label>
                      )}
                    </header>
                    {card.messages.map((message, index) => (
                      <div key={index} className="mt-2 space-y-1">
                        <p className="text-xs text-slate-400">
                          {ACTION_LABEL[message.action]} · {message.prs.map((pr) => `#${pr.number}`).join(", ")}
                          {message.permalink && (
                            <>
                              {" "}
                              <button type="button" className="underline" onClick={() => onOpen(message.permalink!)}>
                                Open thread ↗
                              </button>
                            </>
                          )}
                        </p>
                        <pre aria-label="Message preview" className="whitespace-pre-wrap rounded bg-slate-950 p-3 text-xs">
                          {message.text}
                        </pre>
                      </div>
                    ))}
                    {card.warnings.map((warning) => (
                      <p key={warning} className="mt-1 text-xs text-amber-300">
                        {warning}
                      </p>
                    ))}
                    {!card.signed_in && (
                      <p className="mt-1 text-xs text-amber-300">Sign in to {card.target.workspace} in Settings → Slack before sending.</p>
                    )}
                  </section>
                ))}
                {preview.skipped.length > 0 && (
                  <section aria-label="Skipped" className="rounded border border-slate-800 p-3 text-xs text-slate-400">
                    <h3 className="mb-1 font-medium text-slate-300">Skipped</h3>
                    <ul>
                      {preview.skipped.map((skip) => (
                        <li key={skip.pr.pr_id}>
                          #{skip.pr.number} {skip.pr.title}: {skip.detail}
                        </li>
                      ))}
                    </ul>
                  </section>
                )}
              </div>
            )}
          </>
        )}
        {error && <p className="mt-3 text-rose-300">{error}</p>}
        <footer className="mt-5 flex justify-end gap-2">
          {result ? (
            <>
              {failedCount > 0 && (
                <button type="button" className="rounded border border-slate-600 px-3 py-1.5" onClick={retryFailed}>
                  Retry failed
                </button>
              )}
              <button type="button" className="rounded bg-violet-500 px-3 py-1.5 font-medium text-white" onClick={onClose}>
                Done
              </button>
            </>
          ) : (
            <>
              <button type="button" className="rounded px-3 py-1.5 text-slate-300 hover:bg-slate-800" disabled={sending} onClick={onClose}>
                Cancel
              </button>
              <button
                type="button"
                className="rounded bg-violet-500 px-3 py-1.5 font-medium text-white disabled:opacity-40"
                disabled={!preview || messageCount === 0 || unsigned.length > 0 || sending}
                onClick={() => void send()}
              >
                {sending ? "Sending…" : `Send ${messageCount} message${messageCount === 1 ? "" : "s"}`}
              </button>
            </>
          )}
        </footer>
      </div>
    </div>
  );
}

function Results({ result, onOpen }: { result: BulkResult; onOpen: (url: string) => void }) {
  const tone = { sent: "text-emerald-300", skipped: "text-slate-400", failed: "text-rose-300" } as const;
  return (
    <table className="mt-3 w-full text-left text-xs">
      <thead className="text-slate-400">
        <tr>
          <th className="py-1">PR</th>
          <th>Channel</th>
          <th>Result</th>
          <th>Detail</th>
        </tr>
      </thead>
      <tbody>
        {result.results.map((item, index) => (
          <tr key={index} className="border-t border-slate-800 align-top">
            <td className="py-1 pr-2">
              #{item.pr.number} {item.pr.title}
            </td>
            <td className="pr-2">{item.target ? `${item.target.workspace} · ${channelLabel(item.target)}` : "—"}</td>
            <td className={`pr-2 ${tone[item.outcome]}`}>{item.outcome}</td>
            <td>
              {item.detail}
              {item.permalink && (
                <>
                  {" "}
                  <button type="button" className="underline" onClick={() => onOpen(item.permalink!)}>
                    Open ↗
                  </button>
                </>
              )}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
