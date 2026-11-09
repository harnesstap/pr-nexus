import { useCallback, useEffect, useRef, useState } from "react";
import type { Api, BulkCard, BulkGroup, BulkMessage, BulkPreview, GroupChannelChoice, GroupChoice, GroupRef, ListRow, MappingRow, OwnerGroup, PrWarning, ReviewTarget, SlackIndicator, SlackWorkspace, TargetChoice } from "../api";
import { channelLabel, errorMessage } from "../api";
import type { Toast } from "../list/model";
import { plainText } from "../textInput";
import { Tooltip } from "../Tooltip";
import { CopyMessageButton } from "./CopyMessageButton";
import { DraftWarning } from "./DraftWarning";
import { Mrkdwn } from "./mrkdwn";
import { TargetCombobox } from "./TargetCombobox";

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
  if (group.teams.length < 2) {
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

/** A line with no channel yet: a card, not a send, until the user picks one. */
export function needsChannel(group: BulkGroup): boolean {
  return group.needs_channel === true || (!group.channel_id && Boolean(group.hint));
}

export function groupKey(group: Pick<BulkGroup, "pr_id" | "group_id">): string {
  return `${group.pr_id}:${group.group_id}`;
}

export function actionLabel(action: BulkMessage["action"]): string {
  switch (action) {
    case "request":
      return "New request";
    case "bump":
      return "Bump in thread";
    case "remind":
      return "New reminder (replaces the old threads)";
    default: {
      const unexpected: never = action;
      return unexpected;
    }
  }
}

/** Save is on unless the user turns it off. Matches the mapping-correction default. */
export function saveChecked(saved: Record<string, boolean>, group: BulkGroup): boolean {
  return saved[groupKey(group)] ?? true;
}

export function RemoveCardButton({ name, onRemove }: { name: string; onRemove: () => void }) {
  return (
    <button type="button" className="shrink-0 text-xs text-slate-400 underline" aria-label={`Remove ${name}`} onClick={onRemove}>
      Remove
    </button>
  );
}

export function SaveChannelBox({ team, checked, onChange }: { team: string; checked: boolean; onChange: (checked: boolean) => void }) {
  return (
    <label className="mt-2 flex items-center gap-2 text-xs text-slate-400">
      <input type="checkbox" checked={checked} onChange={(event) => onChange(event.target.checked)} />
      Save channel for {teamLabel(team)}
    </label>
  );
}

function useSuggestions(api: Pick<Api, "reviewTargets">, prIds: number[], teams: string[]) {
  const [suggestions, setSuggestions] = useState<TargetChoice[]>([]);
  const ids = prIds.join(",");
  const teamKey = teams.join(",");
  useEffect(() => {
    if (!ids) {
      return;
    }
    let cancel = false;
    void Promise.all(ids.split(",").map((id) => api.reviewTargets(Number(id), teamKey ? teamKey.split(",") : []).catch(() => [] as TargetChoice[]))).then(
      (lists) => {
        if (!cancel) {
          setSuggestions(lists.flat());
        }
      },
    );
    return () => {
      cancel = true;
    };
  }, [api, ids, teamKey]);
  return suggestions;
}

/** The review dialog's channel picker, optionally tightened for a card. */
export function ChannelField({
  api,
  workspaces,
  prIds,
  teams,
  value,
  label,
  compact = false,
  onSelect,
}: {
  api: Pick<Api, "reviewTargets" | "searchSlackTargets">;
  workspaces: SlackWorkspace[];
  prIds: number[];
  teams: string[];
  value: ReviewTarget | null;
  label: string;
  compact?: boolean;
  onSelect: (target: ReviewTarget) => void;
}) {
  const suggestions = useSuggestions(api, prIds, teams);
  return (
    <TargetCombobox
      api={api}
      workspaces={workspaces}
      suggestions={suggestions}
      value={value}
      label={label}
      compact={compact}
      onSelect={(target) => onSelect(target)}
    />
  );
}

/** Click the channel that will be posted to, and pick another one. */
export function ChannelHeading({
  api,
  workspaces,
  prIds,
  teams,
  target,
  onSelect,
}: {
  api: Pick<Api, "reviewTargets" | "searchSlackTargets">;
  workspaces: SlackWorkspace[];
  prIds: number[];
  teams: string[];
  target: ReviewTarget;
  onSelect: (target: ReviewTarget) => void;
}) {
  const [editing, setEditing] = useState(false);
  const label = channelLabel(target);
  if (editing) {
    return (
      <div className="min-w-0 flex-1">
        <ChannelField
          api={api}
          workspaces={workspaces}
          prIds={prIds}
          teams={teams}
          value={target}
          label={`Channel, currently ${label}`}
          compact
          onSelect={(next) => {
            setEditing(false);
            onSelect(next);
          }}
        />
      </div>
    );
  }
  return (
    <Tooltip label={`Channel ID ${target.channel_id}`} align="start">
      <button
        type="button"
        className="font-medium underline decoration-slate-600 underline-offset-2 hover:decoration-slate-200"
        aria-label={`Change channel, currently ${label}`}
        onClick={() => setEditing(true)}
      >
        {label}
      </button>
    </Tooltip>
  );
}

export function PendingGroups({
  api,
  workspaces,
  groups,
  busy,
  onPickTeam,
  onPickChannel,
  onRemove,
}: {
  api: Pick<Api, "reviewTargets" | "searchSlackTargets">;
  workspaces: SlackWorkspace[];
  groups: BulkGroup[];
  busy: boolean;
  onPickTeam: (group: BulkGroup, team: string) => void;
  onPickChannel: (group: BulkGroup, target: ReviewTarget) => void;
  onRemove: (group: BulkGroup) => void;
}) {
  const pending = groups.filter(needsChannel);
  if (pending.length === 0) {
    return null;
  }
  return (
    <>
      {pending.map((group) => {
        const name = group.teams.map(teamLabel).join(", ") || group.pattern;
        return (
          <section key={groupKey(group)} aria-label={`Choose a channel for ${name}`} className="rounded border border-dashed border-slate-600 p-3">
            <header className="flex items-start justify-between gap-2">
              <h3 className="font-medium">{name}</h3>
              <RemoveCardButton name={name} onRemove={() => onRemove(group)} />
            </header>
            {group.hint && (
              <p role="note" aria-label="No Slack channel" className="mt-1 text-xs text-slate-500">
                {group.hint}
              </p>
            )}
            <GroupSwitcher group={group} disabled={busy} onPick={(team) => onPickTeam(group, team)} />
            <div className="mt-2">
              <ChannelField
                api={api}
                workspaces={workspaces}
                prIds={[group.pr_id]}
                teams={group.teams}
                value={null}
                label="Channel"
                onSelect={(target) => onPickChannel(group, target)}
              />
            </div>
          </section>
        );
      })}
    </>
  );
}

/** Write a user mapping for each picked channel the user left checked. */
export async function saveChosenChannels(api: Pick<Api, "saveMapping">, preview: BulkPreview, channels: GroupChannelChoice[], saved: Record<string, boolean>) {
  for (const choice of channels) {
    const group = (preview.groups ?? []).find((item) => item.pr_id === choice.pr_id && item.group_id === choice.group_id);
    if (!group?.selected || !saveChecked(saved, group)) {
      continue;
    }
    const card = preview.cards.find((item) => groupOnCard(group, item.target));
    const target = card?.target ?? choice.target;
    if (!target.channel_id.trim()) {
      continue;
    }
    const [org, slug] = group.selected.replace(/^@/, "").split("/");
    if (!org || !slug || slug.includes("/")) {
      continue;
    }
    const row: MappingRow = {
      github_org: org,
      team_slug: slug,
      team_name: "",
      slack_workspace: target.workspace,
      slack_team_id: target.team_id ?? undefined,
      slack_channel_name: target.channel_name,
      slack_channel_id: target.channel_id,
    };
    await api.saveMapping(row, false);
  }
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
  const [channels, setChannels] = useState<GroupChannelChoice[]>([]);
  const [skipped, setSkipped] = useState<GroupRef[]>([]);
  const [saveFor, setSaveFor] = useState<Record<string, boolean>>({});
  const [workspaces, setWorkspaces] = useState<SlackWorkspace[]>([]);
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
    async (groupChoices: GroupChoice[], groupChannels: GroupChannelChoice[], skippedGroups: GroupRef[]) => {
      const run = ++generation.current;
      const current = () => run === generation.current;
      const body = {
        pr_ids: [row.id],
        target: null,
        single_top_level: [],
        group_choices: groupChoices,
        group_channels: groupChannels,
        skipped_groups: skippedGroups,
      };
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
    void api.slackWorkspaces().then(setWorkspaces).catch(() => setWorkspaces([]));
  }, [api]);

  useEffect(() => {
    void (async () => {
      await plan([], [], []);
    })();
    return () => {
      generation.current += 1;
    };
  }, [plan]);

  function replan(groupChoices: GroupChoice[], groupChannels: GroupChannelChoice[], skippedGroups: GroupRef[]) {
    setLoading(true);
    setPreview(null);
    setError(null);
    void plan(groupChoices, groupChannels, skippedGroups);
  }

  function pick(group: BulkGroup, team: string) {
    const next = choices.filter((choice) => !(choice.pr_id === group.pr_id && choice.group_id === group.group_id));
    next.push({ pr_id: group.pr_id, group_id: group.group_id, team });
    setChoices(next);
    replan(next, channels, skipped);
  }

  function pickChannel(group: BulkGroup, target: ReviewTarget) {
    const next = channels.filter((choice) => !(choice.pr_id === group.pr_id && choice.group_id === group.group_id));
    next.push({ pr_id: group.pr_id, group_id: group.group_id, target });
    setChannels(next);
    replan(choices, next, skipped);
  }

  function retarget(card: BulkCard, target: ReviewTarget) {
    const mine = (preview?.groups ?? []).filter((group) => groupOnCard(group, card.target));
    pickChannelMany(mine, target);
  }

  function pickChannelMany(mine: BulkGroup[], target: ReviewTarget) {
    const next = channels.filter((choice) => !mine.some((group) => group.pr_id === choice.pr_id && group.group_id === choice.group_id));
    for (const group of mine) {
      next.push({ pr_id: group.pr_id, group_id: group.group_id, target });
    }
    setChannels(next);
    replan(choices, next, skipped);
  }

  function removeGroups(mine: BulkGroup[]) {
    const next = [...skipped, ...mine.map((group) => ({ pr_id: group.pr_id, group_id: group.group_id }))];
    setSkipped(next);
    replan(choices, channels, next);
  }

  async function markReady() {
    setMarking(true);
    setError(null);
    try {
      await api.setPrDraft(row.id, false);
      setMarkedReady(true);
      onReady?.(row.id);
      setLoading(true);
      await plan(choices, channels, skipped);
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
      await saveChosenChannels(api, preview, channels, saveFor);
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
  const label = messageCount > 1 ? `Send ${messageCount} messages` : "Send request";
  const overridden = (group: BulkGroup) => channels.some((choice) => choice.pr_id === group.pr_id && choice.group_id === group.group_id);

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
            <GroupCard
              key={card.fingerprint}
              api={api}
              workspaces={workspaces}
              card={card}
              groups={groups}
              busy={loading || naming || sending}
              note={note}
              saveFor={saveFor}
              overridden={overridden}
              onPick={pick}
              onRetarget={(target) => retarget(card, target)}
              onRemove={() => removeGroups(groups.filter((group) => groupOnCard(group, card.target)))}
              onSave={(group, checked) => setSaveFor((current) => ({ ...current, [groupKey(group)]: checked }))}
              onOpen={onOpen}
              onNotice={onNotice}
            />
          ))}
          <PendingGroups
            api={api}
            workspaces={workspaces}
            groups={groups}
            busy={loading || naming || sending}
            onPickTeam={pick}
            onPickChannel={pickChannel}
            onRemove={(group) => removeGroups([group])}
          />
          {(preview.cards.length > 0 || groups.some(needsChannel)) && (
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
  api,
  workspaces,
  card,
  groups,
  busy,
  note,
  saveFor,
  overridden,
  onPick,
  onRetarget,
  onRemove,
  onSave,
  onOpen,
  onNotice,
}: {
  api: Api;
  workspaces: SlackWorkspace[];
  card: BulkCard;
  groups: BulkGroup[];
  busy: boolean;
  note: string;
  saveFor: Record<string, boolean>;
  overridden: (group: BulkGroup) => boolean;
  onPick: (group: BulkGroup, team: string) => void;
  onRetarget: (target: ReviewTarget) => void;
  onRemove: () => void;
  onSave: (group: BulkGroup, checked: boolean) => void;
  onOpen: (url: string) => void;
  onNotice?: (toast: Toast) => void;
}) {
  const mine = groups.filter((group) => groupOnCard(group, card.target));
  const teams = [...new Set(mine.flatMap((group) => group.teams))];
  const prIds = [...new Set(mine.map((group) => group.pr_id))];
  const label = channelLabel(card.target);
  return (
    <section aria-label={label} className="rounded border border-slate-700 p-3">
      <header className="flex items-start justify-between gap-2">
        <ChannelHeading api={api} workspaces={workspaces} prIds={prIds.length > 0 ? prIds : [card.messages[0]?.prs[0]?.pr_id ?? 0]} teams={teams} target={card.target} onSelect={onRetarget} />
        <RemoveCardButton name={label} onRemove={onRemove} />
      </header>
      {mine.length > 1 && <p className="mt-1 text-xs text-slate-500">These lines share this message.</p>}
      {mine.map((group) => (
        <GroupSwitcher key={group.group_id} group={group} disabled={busy} onPick={(team) => onPick(group, team)} />
      ))}
      {card.messages.map((message, index) => {
        const text = withNote(message.text, note);
        return (
          <div key={index} className="relative mt-2">
            <p className="text-xs text-slate-400">
              {actionLabel(message.action)}
              {message.permalink && (
                <>
                  {" "}
                  <button type="button" className="underline" onClick={() => onOpen(message.permalink!)}>
                    Open thread ↗
                  </button>
                </>
              )}
            </p>
            <div className="relative mt-1 rounded border border-slate-800 bg-slate-950 p-3 pr-10">
              <Mrkdwn label="Message preview" text={text} onOpen={onOpen} />
              <CopyMessageButton text={text} onNotice={onNotice} />
            </div>
          </div>
        );
      })}
      {mine.filter((group) => overridden(group) && group.selected).map((group) => (
        <SaveChannelBox key={groupKey(group)} team={group.selected as string} checked={saveChecked(saveFor, group)} onChange={(checked) => onSave(group, checked)} />
      ))}
      {!card.signed_in && <p className="mt-1 text-xs text-amber-300">Sign in to {card.target.workspace} in Settings → Slack before sending.</p>}
    </section>
  );
}
