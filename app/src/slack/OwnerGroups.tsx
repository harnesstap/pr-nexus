import { useCallback, useEffect, useRef, useState } from "react";
import type { Api, BulkCard, BulkGroup, BulkPreview, GroupChoice, ListRow, OwnerGroup, PrWarning, ReviewTarget, SlackIndicator } from "../api";
import { channelLabel, errorMessage } from "../api";
import type { Toast } from "../list/model";
import { plainText } from "../textInput";
import { CopyMessageButton } from "./CopyMessageButton";
import { DraftWarning } from "./DraftWarning";
import { Mrkdwn } from "./mrkdwn";

function withNote(text: string, note: string): string {
  const trimmed = note.trim();
  return trimmed ? `${text.trimEnd()}\n\n${trimmed}\n` : text;
}

/** More than one pending line, or a line that is not exactly one team. */
export function needsGroupSends(groups: OwnerGroup[] | undefined): boolean {
  const pending = (groups ?? []).filter((group) => !group.satisfied);
  return pending.length > 1 || pending.some((group) => group.teams.length !== 1);
}

export function teamLabel(team: string): string {
  return team.startsWith("@") ? team : `@${team}`;
}

function sameWorkspace(left: string | null | undefined, right: string): boolean {
  return (left ?? "").trim().toLowerCase() === right.trim().toLowerCase();
}

/** The group is posted on this card. The card header is the channel that is sent. */
export function groupOnCard(group: BulkGroup, target: ReviewTarget): boolean {
  return group.channel_id === target.channel_id && sameWorkspace(group.workspace, target.workspace);
}

export function isFailingCheck(warning: PrWarning): boolean {
  switch (warning.kind) {
    case "draft":
      return false;
    case "checks":
      return true;
    default: {
      const unexpected: never = warning.kind;
      return unexpected;
    }
  }
}

/** Compact team choice. The button names the team, never a channel. */
export function GroupSwitcher({
  group,
  disabled,
  onPick,
}: {
  group: BulkGroup;
  disabled: boolean;
  onPick: (team: string) => void;
}) {
  if (group.hint || group.teams.length < 2) {
    return null;
  }
  return (
    <div className="mt-2 flex flex-wrap items-center gap-1" role="group" aria-label={`Team for ${group.pattern}`}>
      {group.teams.map((team) => {
        const pressed = group.selected?.toLowerCase() === team.toLowerCase();
        return (
          <button
            key={team}
            type="button"
            aria-pressed={pressed}
            disabled={disabled}
            className={`rounded px-2 py-0.5 text-xs ${pressed ? "bg-violet-500 text-white" : "bg-slate-800 text-slate-200 hover:bg-slate-700"}`}
            onClick={() => onPick(team)}
          >
            {teamLabel(team)}
          </button>
        );
      })}
    </div>
  );
}

export function UnmappedHints({ groups }: { groups: BulkGroup[] }) {
  return (
    <>
      {groups
        .filter((group) => group.hint)
        .map((group) => (
          <p key={`${group.pr_id}:${group.group_id}`} role="note" aria-label="No Slack channel" className="text-xs text-slate-500">
            {group.hint}
          </p>
        ))}
    </>
  );
}

export function CheckNotes({ warnings }: { warnings: PrWarning[] }) {
  return (
    <>
      {warnings.filter(isFailingCheck).map((warning) => (
        <p key={`${warning.pr_id}:${warning.kind}`} role="note" aria-label="Failing checks" className="text-xs text-slate-500">
          {warning.message}
        </p>
      ))}
    </>
  );
}

interface GroupedProps {
  api: Api;
  row: ListRow;
  onClose: () => void;
  onSent: (indicator: SlackIndicator) => void;
  onOpen: (url: string) => void;
  onReady?: (id: number) => void;
  onNotice?: (toast: Toast) => void;
}

/**
 * One send per CODEOWNERS line that still needs an approval. Lines that share
 * a channel are already one card. Send posts each card to the channel it shows.
 */
export function GroupedRequest({ api, row, onClose, onSent, onOpen, onReady, onNotice }: GroupedProps) {
  const [preview, setPreview] = useState<BulkPreview | null>(null);
  const [choices, setChoices] = useState<GroupChoice[]>([]);
  const [loading, setLoading] = useState(true);
  const [naming, setNaming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [note, setNote] = useState("");
  const [markedReady, setMarkedReady] = useState(false);
  const [marking, setMarking] = useState(false);
  const [unconfirmed, setUnconfirmed] = useState<string | null>(null);
  const generation = useRef(0);

  const plan = useCallback(
    async (groupChoices: GroupChoice[]) => {
      const run = ++generation.current;
      const current = () => run === generation.current;
      const body = { pr_ids: [row.id], target: null, single_top_level: [], group_choices: groupChoices };
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
        // Keep the stored names. Send posts the channel id the card shows.
      } finally {
        if (current()) {
          setNaming(false);
        }
      }
    },
    [api, row.id],
  );

  useEffect(() => {
    void (async () => {
      await plan([]);
    })();
    return () => {
      generation.current += 1;
    };
  }, [plan]);

  function pick(group: BulkGroup, team: string) {
    const next = choices.filter((choice) => !(choice.pr_id === group.pr_id && choice.group_id === group.group_id));
    next.push({ pr_id: group.pr_id, group_id: group.group_id, team });
    setChoices(next);
    setLoading(true);
    setPreview(null);
    setError(null);
    void plan(next);
  }

  async function markReady() {
    setMarking(true);
    setError(null);
    try {
      await api.setPrDraft(row.id, false);
      setMarkedReady(true);
      onReady?.(row.id);
      setLoading(true);
      await plan(choices);
    } catch (err) {
      setError(`Couldn't mark it ready for review: ${errorMessage(err)}`);
    } finally {
      setMarking(false);
    }
  }

  async function send() {
    if (!preview) {
      return;
    }
    setSending(true);
    setError(null);
    try {
      let last: SlackIndicator | null = null;
      for (const card of preview.cards) {
        const teams = (preview.groups ?? []).filter((group) => groupOnCard(group, card.target) && group.selected).map((group) => group.selected as string);
        last = await api.sendReview(row.id, card.target, teams, note);
      }
      if (last) {
        onSent(last);
      }
    } catch (err) {
      if (err && typeof err === "object" && (err as { kind?: unknown }).kind === "sent_unconfirmed") {
        setUnconfirmed(errorMessage(err));
      } else {
        setError(errorMessage(err));
      }
      setSending(false);
    }
  }

  const groups = preview?.groups ?? [];
  const drafts = !markedReady && (preview?.pr_warnings ?? []).some((warning) => warning.kind === "draft");
  const messageCount = preview?.cards.reduce((sum, card) => sum + card.messages.length, 0) ?? 0;
  const unsigned = preview?.cards.some((card) => !card.signed_in) ?? false;
  const label = messageCount === 1 ? "Send request" : `Send ${messageCount} messages`;

  return (
    <>
      {(drafts || (row.state === "open" && row.is_draft && !markedReady)) && (
        <DraftWarning busy={marking} onMarkReady={() => void markReady()}>
          This PR is still a draft. Reviewers may skip it.
        </DraftWarning>
      )}
      {preview && <CheckNotes warnings={preview.pr_warnings} />}
      {loading && <p className="mt-4 text-slate-400">Finding channels…</p>}
      {naming && (
        <p role="status" className="mt-2 text-xs text-slate-400">
          Checking channel names on Slack…
        </p>
      )}
      {preview && (
        <div className="mt-4 space-y-3">
          {preview.cards.map((card) => (
            <GroupCard key={card.fingerprint} card={card} groups={groups} busy={loading || naming || sending} note={note} onPick={pick} onOpen={onOpen} onNotice={onNotice} />
          ))}
          <UnmappedHints groups={groups} />
          {preview.cards.length > 0 && (
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
          disabled={!preview || loading || naming || messageCount === 0 || unsigned || sending || marking || unconfirmed !== null}
          onClick={() => void send()}
        >
          {sending ? "Sending…" : drafts ? `${label} anyway` : label}
        </button>
      </footer>
    </>
  );
}

function GroupCard({
  card,
  groups,
  busy,
  note,
  onPick,
  onOpen,
  onNotice,
}: {
  card: BulkCard;
  groups: BulkGroup[];
  busy: boolean;
  note: string;
  onPick: (group: BulkGroup, team: string) => void;
  onOpen: (url: string) => void;
  onNotice?: (toast: Toast) => void;
}) {
  const mine = groups.filter((group) => groupOnCard(group, card.target));
  return (
    <section aria-label={channelLabel(card.target)} className="rounded border border-slate-700 p-3">
      <h3 className="font-medium">{channelLabel(card.target)}</h3>
      {mine.map((group) => (
        <GroupSwitcher key={group.group_id} group={group} disabled={busy} onPick={(team) => onPick(group, team)} />
      ))}
      {card.messages.map((message, index) => {
        const text = withNote(message.text, note);
        return (
          <div key={index} className="relative mt-2 rounded border border-slate-800 bg-slate-950 p-3 pr-10">
            <Mrkdwn label="Message preview" text={text} onOpen={onOpen} />
            <CopyMessageButton text={text} onNotice={onNotice} />
          </div>
        );
      })}
      {!card.signed_in && <p className="mt-1 text-xs text-amber-300">Sign in to {card.target.workspace} in Settings → Slack before sending.</p>}
    </section>
  );
}
