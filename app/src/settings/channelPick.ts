import type { ChannelRef, MappingRow, ReviewTarget } from "../api";

const bare = (name: string) => name.trim().replace(/^#/, "").toLowerCase();

/**
 * The mapping row after picking `target` in the channel picker: the id and
 * the name move together, so the row never names one channel and sends to
 * another (§7.8). The workspace follows the pick when the pick knows it.
 */
export function applyPick(row: MappingRow, target: ReviewTarget): MappingRow {
  const next: MappingRow = {
    ...row,
    slack_channel_id: target.channel_id.trim().toUpperCase(),
    slack_channel_name: target.channel_name.trim().replace(/^#/, ""),
  };
  if (target.workspace) {
    next.slack_workspace = target.workspace;
    next.slack_team_id = target.team_id ?? undefined;
  }
  return next;
}

/** The row as a picker value; null before a channel is set. */
export function rowTarget(row: MappingRow): ReviewTarget | null {
  if (!row.slack_channel_id.trim()) {
    return null;
  }
  return {
    workspace: row.slack_workspace,
    team_id: row.slack_team_id ?? null,
    channel_id: row.slack_channel_id,
    channel_name: row.slack_channel_name,
  };
}

/** What to ask Slack to name a row's channel. */
export function channelRef(row: MappingRow): ChannelRef {
  return { workspace: row.slack_workspace, team_id: row.slack_team_id ?? null, channel_id: row.slack_channel_id.trim() };
}

/**
 * Slack's name for the row's channel id when the saved name differs from it;
 * null when they agree or Slack could not say. Nothing is rewritten: the row
 * shows a warning and the user picks the channel again (§7.8).
 */
export function nameMismatch(row: MappingRow, resolved: string | null | undefined): string | null {
  if (!resolved || !row.slack_channel_id.trim() || !row.slack_channel_name.trim()) {
    return null;
  }
  return bare(resolved) === bare(row.slack_channel_name) ? null : resolved.trim().replace(/^#/, "");
}
