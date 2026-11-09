import { targetWorkspaceName } from "./workspaces";
import { useCallback, useEffect, useRef, useState } from "react";
import type { Api, BulkCard, BulkGroup, BulkPreview, BulkRequest, BulkResult, ChannelKey, ChannelMove, GroupChannelChoice, GroupChoice, GroupRef, ReviewTarget, SlackWorkspace } from "../api";
import { channelLabel, errorMessage } from "../api";
import { ChannelHeading, CheckNotes, GroupSwitcher, PendingGroups, RemoveCardButton, SaveChannelBox, groupKey, groupOnCard, saveChecked, saveChosenChannels } from "./OwnerGroups";
import { DraftWarning } from "./DraftWarning";
import { TargetCombobox } from "./TargetCombobox";
import { Tooltip } from "../Tooltip";
import { BULK_CONCURRENCY, mapLimit } from "../mapLimit";
import type { Toast } from "../list/model";
import { CopyMessageButton } from "./CopyMessageButton";

/** A typed id has no workspace until Slack names it. Match the card it became. */
function sameTarget(left: { workspace: string; channel_id: string }, right: { workspace: string; channel_id: string }): boolean {
  if (left.channel_id.trim().toUpperCase() !== right.channel_id.trim().toUpperCase()) {
    return false;
  }
  const a = left.workspace.trim().toLowerCase();
  const b = right.workspace.trim().toLowerCase();
  return a === "" || b === "" || a === b;
}

interface Props {
  api: Api;
  prIds: number[];
  onClose: () => void;
  /** Fresh indicators for the PRs that were posted. */
  /** After each Send (and Retry failed): updates indicators and the closing toast. */
  onSent: (done: BulkResult) => void;
  onOpen: (url: string) => void;
  /** Called for each PR "Mark all ready" took out of draft. */
  onReady?: (id: number) => void;
  /** Shows a toast, e.g. after "Copy message". */
  onNotice?: (toast: Toast) => void;
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
export function BulkDialog({ api, prIds, onClose, onSent, onOpen, onReady, onNotice }: Props) {
  const [ids, setIds] = useState(prIds);
  const [oneChannel, setOneChannel] = useState(false);
  const [target, setTarget] = useState<ReviewTarget | null>(null);
  const [workspaces, setWorkspaces] = useState<SlackWorkspace[]>([]);
  const [single, setSingle] = useState<ChannelKey[]>([]);
  const [preview, setPreview] = useState<BulkPreview | null>(null);
  const [loading, setLoading] = useState(true);
  /** Cards are drawn; channel names are still being read from Slack. */
  const [naming, setNaming] = useState(false);
  /** Bumped by every new plan, so a slow answer for an older one is dropped. */
  const generation = useRef(0);
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [result, setResult] = useState<BulkResult | null>(null);
  /** PRs taken out of draft from this dialog. */
  const [readied, setReadied] = useState<number[]>([]);
  const [marking, setMarking] = useState(false);
  const [groupChoices, setGroupChoices] = useState<GroupChoice[]>([]);
  const [groupChannels, setGroupChannels] = useState<GroupChannelChoice[]>([]);
  const [skippedGroups, setSkippedGroups] = useState<GroupRef[]>([]);
  const [channelMoves, setChannelMoves] = useState<ChannelMove[]>([]);
  const [skippedChannels, setSkippedChannels] = useState<ChannelKey[]>([]);
  const [saveFor, setSaveFor] = useState<Record<string, boolean>>({});

  const request = useCallback(
    (
      next: Partial<{
        ids: number[];
        oneChannel: boolean;
        single: ChannelKey[];
        target: ReviewTarget;
        groupChoices: GroupChoice[];
        groupChannels: GroupChannelChoice[];
        skippedGroups: GroupRef[];
        channelMoves: ChannelMove[];
        skippedChannels: ChannelKey[];
      }> = {},
    ): BulkRequest => ({
      pr_ids: next.ids ?? ids,
      target: (next.oneChannel ?? oneChannel) ? (next.target ?? target) : null,
      single_top_level: next.single ?? single,
      group_choices: next.groupChoices ?? groupChoices,
      group_channels: next.groupChannels ?? groupChannels,
      skipped_groups: next.skippedGroups ?? skippedGroups,
      channel_moves: next.channelMoves ?? channelMoves,
      skipped_channels: next.skippedChannels ?? skippedChannels,
    }),
    [ids, oneChannel, target, single, groupChoices, groupChannels, skippedGroups, channelMoves, skippedChannels],
  );

  /**
   * Plans from local data first (cached channel names) and draws the cards,
   * then, only when a name is still unknown, plans again with Slack and
   * swaps in the named cards. Send waits for the second answer.
   */
  const plan = useCallback(
    async (body: BulkRequest) => {
      const run = ++generation.current;
      const current = () => run === generation.current;
      let first: BulkPreview;
      try {
        first = await api.bulkPreview({ ...body, cached_names: true });
      } catch (err) {
        if (current()) {
          setError(errorMessage(err));
          setLoading(false);
        }
        return;
      }
      if (!current()) {
        return;
      }
      setPreview(first);
      setLoading(false);
      if (!first.names_pending) {
        return;
      }
      setNaming(true);
      try {
        const named = await api.bulkPreview({ ...body, cached_names: false });
        if (current()) {
          setPreview(named);
        }
      } catch {
        // Keep the stored names: Send plans again and posts to the ids.
      } finally {
        if (current()) {
          setNaming(false);
        }
      }
    },
    [api],
  );

  /** A new plan after a toggle or a retry: clears the old cards first. */
  const replan = useCallback(
    (body: BulkRequest) => {
      setLoading(true);
      setNaming(false);
      setError(null);
      setPreview(null);
      void plan(body);
    },
    [plan],
  );

  useEffect(() => {
    let cancel = false;
    void (async () => {
      const signedIn = await api.slackWorkspaces().catch(() => [] as SlackWorkspace[]);
      if (cancel) {
        return;
      }
      setWorkspaces(signedIn);
    })();
    void (async () => {
      await plan({ pr_ids: prIds, target: null, single_top_level: [], group_choices: [] });
    })();
    return () => {
      cancel = true;
      generation.current += 1;
    };
  }, [api, prIds, plan]);

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
    replan(request({ single: next }));
  }

  function pickTeam(group: BulkGroup, team: string) {
    const next = groupChoices.filter((choice) => !(choice.pr_id === group.pr_id && choice.group_id === group.group_id));
    next.push({ pr_id: group.pr_id, group_id: group.group_id, team });
    setGroupChoices(next);
    replan(request({ groupChoices: next }));
  }

  function pickChannel(groups: BulkGroup[], target: ReviewTarget) {
    const next = groupChannels.filter((choice) => !groups.some((group) => group.pr_id === choice.pr_id && group.group_id === choice.group_id));
    for (const group of groups) {
      next.push({ pr_id: group.pr_id, group_id: group.group_id, target });
    }
    setGroupChannels(next);
    replan(request({ groupChannels: next }));
  }

  function retargetCard(card: BulkCard, nextTarget: ReviewTarget) {
    const mine = (preview?.groups ?? []).filter((group) => groupOnCard(group, card.target));
    if (mine.length > 0) {
      pickChannel(mine, nextTarget);
      return;
    }
    const existing = channelMoves.find((move) => sameTarget(move.target, card.target));
    const next = existing
      ? channelMoves.map((move) => (move === existing ? { ...move, target: nextTarget } : move))
      : [...channelMoves, { workspace: card.target.workspace, channel_id: card.target.channel_id, target: nextTarget }];
    setChannelMoves(next);
    replan(request({ channelMoves: next }));
  }

  function removeCard(card: BulkCard) {
    const mine = (preview?.groups ?? []).filter((group) => groupOnCard(group, card.target));
    if (mine.length > 0) {
      const next = [...skippedGroups, ...mine.map((group) => ({ pr_id: group.pr_id, group_id: group.group_id }))];
      setSkippedGroups(next);
      replan(request({ skippedGroups: next }));
      return;
    }
    const move = channelMoves.find((item) => sameTarget(item.target, card.target));
    const key = move ? { workspace: move.workspace, channel_id: move.channel_id } : { workspace: card.target.workspace, channel_id: card.target.channel_id };
    const nextMoves = move ? channelMoves.filter((item) => item !== move) : channelMoves;
    const nextSkipped = [...skippedChannels, key];
    setChannelMoves(nextMoves);
    setSkippedChannels(nextSkipped);
    replan(request({ channelMoves: nextMoves, skippedChannels: nextSkipped }));
  }

  function routeMode(next: boolean) {
    setOneChannel(next);
    if (!next) {
      replan(request({ oneChannel: false }));
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
      await saveChosenChannels(api, preview, groupChannels, saveFor);
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

  /** Same call as the merge card's "Mark ready for review", once per draft, a few at a time. */
  async function markAllReady() {
    setMarking(true);
    setError(null);
    const pending = drafts;
    const results = await mapLimit(pending, BULK_CONCURRENCY, async (warning) => {
      await api.setPrDraft(warning.pr_id, false);
      setReadied((current) => [...current, warning.pr_id]);
      onReady?.(warning.pr_id);
    });
    const failed = results.flatMap((result, index) =>
      result.status === "rejected" ? [`#${pending[index].number}: ${errorMessage(result.reason)}`] : [],
    );
    const message = failed.length > 0 ? `Couldn't mark ready for review: ${failed.join("; ")}` : null;
    if (message) {
      setError(message);
    }
    setMarking(false);
    // The open preview was planned before the draft flips. Plan again so a PR
    // that is no longer only a draft joins its message. A failing check stays
    // a note; this does not treat the draft flip as a passing check.
    if (results.some((result) => result.status === "fulfilled")) {
      setLoading(true);
      await plan(request());
      if (message) {
        setError(message);
      }
    }
  }

  function retryFailed() {
    if (!result) {
      return;
    }
    const failed = [...new Set(result.results.filter((item) => item.outcome === "failed").map((item) => item.pr.pr_id))];
    setIds(failed);
    setResult(null);
    replan(request({ ids: failed }));
  }

  const drafts = (preview?.pr_warnings ?? []).filter((warning) => warning.kind === "draft" && !readied.includes(warning.pr_id));
  const isDraft = (id: number) => drafts.some((warning) => warning.pr_id === id);
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
              <div className="mt-2">
                <TargetCombobox
                  api={api}
                  workspaces={workspaces}
                  suggestions={[]}
                  value={target}
                  onSelect={(next) => {
                    setTarget(next);
                    replan(request({ oneChannel: true, target: next }));
                  }}
                />
              </div>
            )}
            {preview && <CheckNotes warnings={preview.pr_warnings} />}
            {drafts.length > 0 && (
              <DraftWarning busy={marking} onMarkReady={() => void markAllReady()} action={drafts.length === 1 ? "Mark ready for review" : "Mark all ready"}>
                {drafts.length === 1 ? "1" : drafts.length} of {ids.length} PRs {drafts.length === 1 ? "is" : "are"} still {drafts.length === 1 ? "a draft" : "drafts"}:{" "}
                {drafts.map((warning) => `#${warning.number}`).join(", ")}. Reviewers may skip {drafts.length === 1 ? "it" : "them"}.
              </DraftWarning>
            )}
            {loading && <p className="mt-4 text-slate-400">Planning messages…</p>}
            {naming && (
              <p role="status" className="mt-2 text-xs text-slate-400">
                Checking channel names on Slack…
              </p>
            )}
            {preview && (
              <div className="mt-4 space-y-3">
                {preview.cards.length === 0 && <p className="text-slate-400">Nothing to send.</p>}
                {preview.cards.map((card) => (
                  <section
                    key={card.fingerprint}
                    aria-label={[targetWorkspaceName(card.target), channelLabel(card.target)].filter(Boolean).join(" ")}
                    className="rounded border border-slate-700 p-3"
                  >
                    <header className="flex items-start justify-between gap-2">
                      <h3 className="flex min-w-0 flex-1 flex-wrap items-center gap-x-1 font-medium">
                        {targetWorkspaceName(card.target) && <span>{targetWorkspaceName(card.target)} ·</span>}
                        {oneChannel ? (
                          <Tooltip label={`Channel ID ${card.target.channel_id}`} align="start">
                            <span tabIndex={0}>{channelLabel(card.target)}</span>
                          </Tooltip>
                        ) : (
                          <ChannelHeading
                            api={api}
                            workspaces={workspaces}
                            prIds={[...new Set(card.messages.flatMap((message) => message.prs.map((pr) => pr.pr_id)))]}
                            teams={[...new Set((preview.groups ?? []).filter((group) => groupOnCard(group, card.target)).flatMap((group) => group.teams))]}
                            target={card.target}
                            onSelect={(next) => retargetCard(card, next)}
                          />
                        )}
                      </h3>
                      <div className="flex shrink-0 items-center gap-3">
                        {(card.can_switch || card.single_top_level) && (
                          <label className="flex items-center gap-1 text-xs text-slate-300">
                            <input type="checkbox" checked={card.single_top_level} onChange={() => toggleSingle(card)} />
                            One top-level request for all
                          </label>
                        )}
                        {!oneChannel && <RemoveCardButton name={channelLabel(card.target)} onRemove={() => removeCard(card)} />}
                      </div>
                    </header>
                    {!oneChannel &&
                      (preview.groups ?? [])
                        .filter((group) => groupOnCard(group, card.target))
                        .map((group) => (
                          <GroupSwitcher key={`${group.pr_id}:${group.group_id}`} group={group} disabled={loading || naming || sending} onPick={(team) => pickTeam(group, team)} />
                        ))}
                    {card.messages.map((message, index) => (
                      <div key={index} className="mt-2 space-y-1">
                        <p className="text-xs text-slate-400">
                          {ACTION_LABEL[message.action]} ·{" "}
                          {message.prs.map((pr, index) => (
                            <span key={pr.pr_id}>
                              {index > 0 && ", "}#{pr.number}
                              {isDraft(pr.pr_id) && <DraftBadge />}
                            </span>
                          ))}
                          {message.permalink && (
                            <>
                              {" "}
                              <button type="button" className="underline" onClick={() => onOpen(message.permalink!)}>
                                Open thread ↗
                              </button>
                            </>
                          )}
                        </p>
                        <div className="relative">
                          <pre aria-label="Message preview" className="whitespace-pre-wrap rounded bg-slate-950 p-3 pr-10 text-xs">
                            {message.text}
                          </pre>
                          <CopyMessageButton text={message.text} onNotice={onNotice} />
                        </div>
                      </div>
                    ))}
                    {(card.notes ?? []).map((note) => (
                      <p key={note} className="mt-1 text-xs text-sky-300">
                        {note}
                      </p>
                    ))}
                    {card.warnings.map((warning) => (
                      <p key={warning} className="mt-1 text-xs text-amber-300">
                        {warning}
                      </p>
                    ))}
                    {(preview.groups ?? [])
                      .filter((group) => groupOnCard(group, card.target) && groupChannels.some((choice) => choice.pr_id === group.pr_id && choice.group_id === group.group_id) && group.selected)
                      .map((group) => (
                        <SaveChannelBox
                          key={groupKey(group)}
                          team={group.selected as string}
                          checked={saveChecked(saveFor, group)}
                          onChange={(checked) => setSaveFor((current) => ({ ...current, [groupKey(group)]: checked }))}
                        />
                      ))}
                    {!card.signed_in && (
                      <p className="mt-1 text-xs text-amber-300">Sign in to {card.target.workspace} in Settings → Slack before sending.</p>
                    )}
                  </section>
                ))}
                {!oneChannel && (
                  <PendingGroups
                    api={api}
                    workspaces={workspaces}
                    groups={preview.groups ?? []}
                    busy={loading || naming || sending}
                    onPickTeam={pickTeam}
                    onPickChannel={(group, next) => pickChannel([group], next)}
                    onRemove={(group) => {
                      const next = [...skippedGroups, { pr_id: group.pr_id, group_id: group.group_id }];
                      setSkippedGroups(next);
                      replan(request({ skippedGroups: next }));
                    }}
                  />
                )}
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
                disabled={!preview || naming || messageCount === 0 || unsigned.length > 0 || sending || marking}
                onClick={() => void send()}
              >
                {sending ? "Sending…" : `Send ${messageCount} message${messageCount === 1 ? "" : "s"}${drafts.length > 0 ? " anyway" : ""}`}
              </button>
            </>
          )}
        </footer>
      </div>
    </div>
  );
}

/** The detail panel's "Draft" pill, small, beside a PR number. */
function DraftBadge() {
  return (
    <span data-testid="draft-badge" className="ml-1 rounded-full bg-slate-700/60 px-1.5 py-px text-[10px] font-medium text-slate-300 ring-1 ring-slate-500/40">
      Draft
    </span>
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
