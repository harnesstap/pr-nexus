import { useCallback, useEffect, useRef, useState } from "react";
import type { Api, ListRow, OwnerGroup, PrDetail, ReviewPreview, ReviewTarget, SlackIndicator, SlackWorkspace, TargetChoice } from "../api";
import { channelLabel, errorMessage, isSendable } from "../api";
import { beginDraftFlip, draftPhase, offerPreviewDraft, settleDraftCall, settleDraftFlip, takeGeneration, useDraftRecord, type DraftPhase } from "../detail/draftRecord";
import { Tooltip } from "../Tooltip";
import { plainText } from "../textInput";
import type { Toast } from "../list/model";
import { CopyMessageButton } from "./CopyMessageButton";
import { DraftWarning } from "./DraftWarning";
import { Mrkdwn } from "./mrkdwn";
import { CheckNotes, GroupedRequest, needsGroupSends, withNote } from "./OwnerGroups";
import { peekReviewPrefetch } from "./reviewPrefetch";
import { TargetCombobox } from "./TargetCombobox";
import { expiredSignIn } from "./workspaces";

interface Props {
  api: Api;
  row: ListRow;
  onClose: () => void;
  /** Called with the stored request after a post or bump succeeded. */
  onSent: (indicator: SlackIndicator) => void;
  onOpen: (url: string) => void;
  /** Called after "Mark ready for review" took the PR out of draft. */
  onReady?: (id: number) => void;
  /** Shows a toast, e.g. after "Copy message". */
  onNotice?: (toast: Toast) => void;
  /** Owners the detail panel already loaded. Skips a second `prDetail`. */
  detail?: PrDetail | null;
}

function mark(name: string): void {
  if (import.meta.env.DEV && typeof performance !== "undefined") {
    performance.mark(name);
  }
}

function sameDestination(a: ReviewTarget, b: ReviewTarget): boolean {
  return a.channel_id === b.channel_id && a.workspace === b.workspace && (a.team_id ?? "") === (b.team_id ?? "");
}

function suggestionTeams(detail: { mandatory_teams: string[]; candidate_teams: string[] }): string[] {
  return detail.mandatory_teams.length > 0 ? detail.mandatory_teams : detail.candidate_teams;
}

function sameTeams(left: string[], right: string[]): boolean {
  if (left.length !== right.length) {
    return false;
  }
  const a = [...left].sort();
  const b = [...right].sort();
  return a.every((team, index) => team === b[index]);
}

interface Painted {
  teams: string[];
  found: TargetChoice[];
  first: TargetChoice;
  mine: number;
}

function hasDraft(preview: ReviewPreview): boolean {
  return preview.pr_warnings.some((warning) => warning.kind === "draft");
}

function sendLabel(phase: DraftPhase, bump: boolean): string {
  switch (phase) {
    case "draft":
      return bump ? "Bump anyway" : "Send anyway";
    case "marking":
    case "ready":
      return bump ? "Bump in thread" : "Send request";
    default: {
      const unexpected: never = phase;
      return unexpected;
    }
  }
}

/**
 * Request or bump review on Slack for one PR (§7.11). Shows the exact text
 * before anything is posted; Send is the only call that reaches Slack.
 */
export function ReviewDialog({ api, row, onClose, onSent, onOpen, onReady, onNotice, detail }: Props) {
  const [seed] = useState(() => peekReviewPrefetch(row.id));
  const record = useDraftRecord(row.id);
  const [teams, setTeams] = useState<string[]>(seed?.teams ?? []);
  const [choices, setChoices] = useState<TargetChoice[] | null>(seed?.choices ?? null);
  const [workspaces, setWorkspaces] = useState<SlackWorkspace[]>([]);
  const [target, setTarget] = useState<ReviewTarget | null>(seed?.target ?? null);
  /** The suggestion picked, whose teams go in the request; null for a searched target. */
  const [chosen, setChosen] = useState<TargetChoice | null>(seed?.choice ?? null);
  const [preview, setPreview] = useState<ReviewPreview | null>(seed?.preview ?? null);
  /** Live channel names are still in flight. Send stays off so the label matches the id. */
  const [naming, setNaming] = useState(seed?.preview != null);
  const [error, setError] = useState<string | null>(null);
  /** Why no channel is suggested, in plain words (owners or channels failed to load). */
  const [hint, setHint] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [note, setNote] = useState("");
  /** The send went through but its link is unknown: never offer a resend. */
  const [unconfirmed, setUnconfirmed] = useState<string | null>(null);
  /** `true` when several CODEOWNERS lines each need their own send. */
  const [groupsMode, setGroupsMode] = useState(false);
  const picked = useRef(false);
  /** Newest preview applied. An older cached read cannot replace it. */
  const previewGen = useRef(seed?.generation ?? 0);

  const applyPreview = useCallback((next: ReviewPreview, observed: number) => {
    if (hasDraft(next)) offerPreviewDraft(row.id, observed);
    if (observed < previewGen.current) return;
    previewGen.current = observed;
    setPreview(next);
  }, [row.id]);

  async function loadPreview(nextTarget: ReviewTarget) {
    const observed = takeGeneration();
    setError(null);
    try {
      applyPreview(await api.reviewPreview(row.id, nextTarget), observed);
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  useEffect(() => {
    if (seed?.preview && hasDraft(seed.preview)) offerPreviewDraft(row.id, seed.generation);
  }, [row.id, seed]);

  useEffect(() => {
    // ListScreen keys this dialog by PR id, so state starts empty for each PR
    // unless a prefetch already filled it. The first paint uses only stored
    // data. Slack token checks and a live owner lookup start after that.
    let cancel = false;
    let flight = 0;
    const sawValidated = { current: false };
    void (async () => {
      try {
        mark("review-dialog:start");
        const [owners, signedIn] = await Promise.all([
          loadOwners(),
          api.slackWorkspaces(false).catch(() => [] as SlackWorkspace[]),
        ]);
        mark("review-dialog:owners");
        if (cancel) {
          return;
        }
        if (!sawValidated.current) {
          setWorkspaces(signedIn);
        }
        const painted = await paintTeams(owners.teams, owners.groups, true);
        if (cancel) {
          return;
        }
        void api
          .slackWorkspaces(true)
          .then((fresh) => {
            if (cancel) {
              return;
            }
            sawValidated.current = true;
            setWorkspaces(fresh);
          })
          .catch(() => undefined);
        if (picked.current) {
          return;
        }
        if (api.reviewOwners) {
          void api
            .reviewOwners(row.id)
            .then((live) => {
              if (cancel || picked.current) {
                return;
              }
              const liveTeams = suggestionTeams(live);
              const changed = !sameTeams(liveTeams, owners.teams) || needsGroupSends(live.owner_groups) !== needsGroupSends(owners.groups);
              if (!changed) {
                return;
              }
              void paintTeams(liveTeams, live.owner_groups, false).then((next) => {
                if (next && !cancel && !picked.current) {
                  void applyLiveNames(next, false);
                }
              });
            })
            .catch(() => undefined);
        }
        if (painted) {
          await applyLiveNames(painted, true);
        }
      } catch (err) {
        if (!cancel) {
          setChoices((current) => current ?? []);
          setNaming(false);
          setError(errorMessage(err));
        }
      }
    })();

    async function loadOwners(): Promise<{ teams: string[]; groups: OwnerGroup[] | undefined }> {
      if (detail) {
        return { teams: suggestionTeams(detail), groups: detail.owner_groups };
      }
      if (api.reviewTeams) {
        try {
          const stored = await api.reviewTeams(row.id);
          return { teams: stored.teams, groups: stored.owner_groups };
        } catch (err) {
          if (!cancel) {
            setHint(`Couldn't read this PR's code owners, so no channel is suggested (${errorMessage(err)}). Search for one above.`);
          }
          return { teams: [], groups: [] };
        }
      }
      try {
        const loaded = await api.prDetail(row.id);
        return { teams: suggestionTeams(loaded), groups: loaded.owner_groups };
      } catch (err) {
        if (!cancel) {
          setHint(`Couldn't read this PR's code owners, so no channel is suggested (${errorMessage(err)}). Search for one above.`);
        }
        return { teams: [], groups: [] };
      }
    }

    async function paintTeams(wanted: string[], groups: OwnerGroup[] | undefined, first: boolean): Promise<Painted | null> {
      if (needsGroupSends(groups)) {
        if (!cancel && !picked.current) {
          setGroupsMode(true);
          setChoices([]);
          setNaming(false);
        }
        return null;
      }
      const mine = ++flight;
      const found = await api.reviewTargets(row.id, wanted, true).catch((err: unknown) => {
        if (!cancel && first) {
          setHint(`Couldn't load the code-owner channels (${errorMessage(err)}). Search for one above.`);
        }
        return [] as TargetChoice[];
      });
      if (first) {
        mark("review-dialog:targets");
      }
      if (cancel || picked.current || mine !== flight) {
        return null;
      }
      setGroupsMode(false);
      setTeams(wanted);
      setChoices(found);
      // Preselect only a channel with a real ID: a name-only mapping is a
      // suggestion to search for, never a target to preview (§7.11).
      const firstChoice = found.find((choice) => choice.source !== "unresolved" && isSendable(choice.target));
      if (!firstChoice) {
        setNaming(false);
        return null;
      }
      setTarget(firstChoice.target);
      setChosen(firstChoice);
      const observed = takeGeneration();
      const next = await api.reviewPreview(row.id, firstChoice.target, true);
      if (first) {
        mark("review-dialog:preview");
      }
      if (cancel || picked.current || mine !== flight) {
        return null;
      }
      applyPreview(next, observed);
      setNaming(true);
      return { teams: wanted, found, first: firstChoice, mine };
    }

    async function applyLiveNames(painted: Painted, first: boolean): Promise<void> {
      const { found, first: suggestion, mine } = painted;
      const live = await api.reviewTargets(row.id, painted.teams, false).catch(() => found);
      if (first && mine === flight) {
        mark("review-dialog:names");
      }
      if (cancel || picked.current || mine !== flight) {
        if (!cancel && picked.current && mine === flight) {
          setNaming(false);
        }
        return;
      }
      const index = found.indexOf(suggestion);
      const liveChoice = live[index] ?? live.find((choice) => choice.target.channel_id === suggestion.target.channel_id);
      setChoices(live);
      if (!liveChoice || sameDestination(liveChoice.target, suggestion.target)) {
        if (liveChoice && liveChoice.target.channel_name !== suggestion.target.channel_name) {
          setTarget(liveChoice.target);
          setChosen(liveChoice);
          setPreview((current) => (current ? { ...current, target: liveChoice.target } : current));
        }
        setNaming(false);
        return;
      }
      setTarget(liveChoice.target);
      setChosen(liveChoice);
      setPreview(null);
      const refreshedGen = takeGeneration();
      const refreshed = await api.reviewPreview(row.id, liveChoice.target);
      if (!cancel && !picked.current && mine === flight) {
        applyPreview(refreshed, refreshedGen);
      }
      if (!cancel && mine === flight) {
        setNaming(false);
      }
    }

    return () => {
      cancel = true;
    };
  }, [api, applyPreview, detail, row.id]);

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.stopPropagation();
        onClose();
      }
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  function choose(next: ReviewTarget, choice: TargetChoice | null) {
    if (!isSendable(next)) {
      return;
    }
    picked.current = true;
    setNaming(false);
    setTarget(next);
    setChosen(choice);
    void loadPreview(next);
  }

  /** The same call as the merge card's "Mark ready for review". */
  async function markReady() {
    setError(null);
    const observed = beginDraftFlip(row.id, false);
    try {
      const fresh = await api.setPrDraft(row.id, false);
      settleDraftCall(row.id, observed, fresh, false);
    } catch (err) {
      settleDraftFlip(row.id, observed, { ok: false });
      setError(`Couldn't mark it ready for review: ${errorMessage(err)}`);
      return;
    }
    onReady?.(row.id);
    if (target) await loadPreview(target);
  }

  async function send() {
    if (!preview) {
      return;
    }
    setSending(true);
    setError(null);
    try {
      onSent(await api.sendReview(row.id, preview.target, chosen?.teams.length ? chosen.teams : teams, note));
    } catch (err) {
      if (err && typeof err === "object" && (err as { kind?: unknown }).kind === "sent_unconfirmed") {
        setUnconfirmed(errorMessage(err));
      } else {
        setError(errorMessage(err));
      }
      setSending(false);
    }
  }

  const bump = preview?.action === "bump";
  const expired = preview ? expiredSignIn(preview.target, workspaces) : null;
  /** Old threads in channels the teams were remapped away from (§7.11). */
  const previous = (choices ?? []).filter((choice) => choice.source === "previous");
  const phase = draftPhase(record, row.state === "open" && row.is_draft);
  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center overflow-auto bg-black/50 p-8"
      onMouseDown={(event) => event.target === event.currentTarget && onClose()}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="review-dialog-title"
        className="w-full max-w-2xl rounded-lg border border-slate-700 bg-slate-900 p-5 text-sm text-slate-200 shadow-xl"
      >
        <h2 id="review-dialog-title" className="text-base font-semibold">
          {bump ? "Bump review request on Slack" : "Request review on Slack"}
        </h2>
        <Tooltip label={`#${row.number} ${row.title} · ${row.repo}`} wide align="start" className="max-w-full">
          <p className="truncate text-xs text-slate-500">
            #{row.number} {row.title} · {row.repo}
          </p>
        </Tooltip>
        {groupsMode ? (
          <GroupedRequest api={api} row={row} onClose={onClose} onSent={onSent} onOpen={onOpen} onReady={onReady} onNotice={onNotice} />
        ) : (
          <>
        {(phase === "draft" || phase === "marking") && (
          <DraftWarning busy={phase === "marking"} busyLabel="Marking ready…" onMarkReady={() => void markReady()}>
            {phase === "marking" ? "Marking this PR ready for review…" : "This PR is still a draft. Reviewers may skip it."}
          </DraftWarning>
        )}
        {preview && <CheckNotes warnings={preview.pr_warnings} />}
        {choices === null ? (
          <p className="mt-4 text-slate-400">Finding channels…</p>
        ) : (
          <div className="mt-4 space-y-3">
            <TargetCombobox
              api={api}
              workspaces={workspaces}
              suggestions={choices}
              value={target}
              onSelect={choose}
            />
            {naming && (
              <p className="flex items-center gap-1.5 text-xs text-slate-400" role="status">
                <span aria-hidden="true" className="inline-block h-3 w-3 animate-spin rounded-full border-2 border-slate-600 border-t-slate-300" />
                Checking channel…
              </p>
            )}
            {hint ? (
              <p className="text-xs text-slate-400">{hint}</p>
            ) : choices.length === 0 ? (
              <p className="text-xs text-slate-400">No mapped channel for this PR's teams. Search for one above.</p>
            ) : (
              !target &&
              choices.some((choice) => choice.source === "unresolved") && (
                <p className="text-xs text-slate-400">
                  The code-owner channel has no channel ID yet. Pick it above to search Slack for it, or add the ID in Settings → Mappings.
                </p>
              )
            )}
            {preview && (
              <div className="space-y-2">
                <p className="text-xs text-slate-400">
                  {bump
                    ? `Bump in the existing thread${preview.broadcast ? ", also sent to the channel" : " only"} (${preview.bump_count} earlier bump${preview.bump_count === 1 ? "" : "s"}) · ${[channelLabel(preview.target), preview.target.workspace_name].filter(Boolean).join(" · ")}.`
                    : `New message in ${[channelLabel(preview.target), preview.target.workspace_name].filter(Boolean).join(" · ")}.`}
                  {bump && preview.permalink && (
                    <>
                      {" "}
                      <button type="button" className="underline" onClick={() => onOpen(preview.permalink!)}>
                        Open thread ↗
                      </button>
                    </>
                  )}
                </p>
                <div className="relative rounded border border-slate-800 bg-slate-950 p-3 pr-10">
                  <Mrkdwn label="Message preview" text={withNote(preview.text, note)} onOpen={onOpen} />
                  <CopyMessageButton text={withNote(preview.text, note)} onNotice={onNotice} />
                </div>
                <label className="block text-xs text-slate-400">
                  Note (optional, added after a blank line)
                  <textarea
                    {...plainText}
                    aria-label="Note"
                    rows={2}
                    value={note}
                    onChange={(event) => setNote(event.target.value)}
                    placeholder="Context for reviewers, e.g. the risky part to look at first"
                    className="mt-1 block w-full resize-y rounded-md border border-slate-700 bg-slate-950 px-2 py-1.5 text-sm text-slate-100 placeholder:text-slate-600 focus:border-violet-500 focus:outline-none"
                  />
                </label>
                {previous.length > 0 && chosen?.source !== "previous" && (
                  <p className="text-xs text-sky-300">
                    Channel changed since the last request: new request in {channelLabel(preview.target)}. To bump the old thread in{" "}
                    {previous.map((choice) => channelLabel(choice.target)).join(", ")} instead, pick it above.
                  </p>
                )}
                {chosen?.source === "previous" && (
                  <p className="text-xs text-sky-300">This channel is no longer mapped to the PR's teams; the bump goes to the old thread here.</p>
                )}
                {preview.warnings.map((warning) => (
                  <p key={warning} className="text-xs text-amber-300">
                    {warning}
                  </p>
                ))}
                {expired ? (
                  <p className="text-xs text-amber-300">
                    Slack sign-in expired. Reconnect the {expired.team_name} workspace in Settings.
                  </p>
                ) : (
                  !preview.signed_in && (
                    <p className="text-xs text-amber-300">
                      Sign in to {preview.target.workspace} in Settings → Slack before sending.
                    </p>
                  )
                )}
              </div>
            )}
          </div>
        )}
        {error && <p className="mt-3 text-rose-300">{error}</p>}
        {unconfirmed && (
          <p className="mt-3 text-amber-300" role="status">
            {unconfirmed}
          </p>
        )}
        <footer className="mt-5 flex justify-end gap-2">
          <button type="button" className="rounded px-3 py-1.5 text-slate-300 hover:bg-slate-800" onClick={onClose}>
            {unconfirmed ? "Close" : "Cancel"}
          </button>
          <button
            type="button"
            className="rounded bg-violet-500 px-3 py-1.5 font-medium text-white disabled:opacity-40"
            disabled={!preview || !preview.signed_in || expired !== null || sending || phase === "marking" || naming || unconfirmed !== null}
            onClick={() => void send()}
          >
            {sending ? "Sending…" : sendLabel(phase, Boolean(bump))}
          </button>
        </footer>
          </>
        )}
      </div>
    </div>
  );
}
