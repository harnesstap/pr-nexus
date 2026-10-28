import { channelLabel, type ListRow } from "../api";

const HOUR_MS = 60 * 60 * 1000;
const DAY_MS = 24 * HOUR_MS;

/** A PR waiting on review longer than this gets a "Waiting" chip. */
export const STALE_REVIEW_HOURS = 48;
/** An open PR with no update and no Slack bump for this many days gets an "Idle" chip. */
export const IDLE_DAYS = 7;

/** Small status pills on a row, next to the title. */
export type RowPill = "draft" | "conflicts";

/** Draft first, then conflicts; closed and merged PRs carry neither. */
export function rowPills(row: Pick<ListRow, "is_draft" | "conflicts" | "state">): RowPill[] {
  if (row.state !== "open") {
    return [];
  }
  const pills: RowPill[] = [];
  if (row.is_draft) {
    pills.push("draft");
  }
  if (row.conflicts) {
    pills.push("conflicts");
  }
  return pills;
}

/**
 * When an open PR started waiting on review: its Slack request, else its
 * creation (GitHub does not give the request time). Null when nobody is asked
 * to review it, or it is a draft or approved. Mirrors `waiting_since` in
 * `app/src-tauri/src/listing.rs`.
 */
export function waitingSince(row: ListRow): number | null {
  const asked = row.reviewers_requested || row.slack !== null;
  if (row.state !== "open" || row.is_draft || !asked || row.review_decision === "approved") {
    return null;
  }
  return new Date(row.slack ? row.slack.requested_at : row.created_at).getTime();
}

/** The last time anything happened on a PR: a GitHub update or a Slack bump. */
export function lastActivity(row: ListRow): number {
  const updated = new Date(row.updated_at).getTime();
  const slack = row.slack ? new Date(row.slack.last_bumped_at ?? row.slack.requested_at).getTime() : 0;
  return Math.max(updated, slack);
}

export interface StaleCue {
  kind: "waiting" | "idle";
  /** Chip text: `Waiting 3d`, `Idle 9d`. */
  label: string;
  title: string;
}

function span(ms: number): string {
  const hours = Math.floor(ms / HOUR_MS);
  return hours < 72 ? `${hours}h` : `${Math.floor(ms / DAY_MS)}d`;
}

/** The age chip for a row, or null when it is fresh. A long review wait wins over idleness. Mirrors `stale_cue` in `app/src-tauri/src/listing.rs` (the `waiting`/`idle` MCP statuses). */
export function staleCue(row: ListRow, now: Date): StaleCue | null {
  if (row.state !== "open") {
    return null;
  }
  const since = waitingSince(row);
  if (since !== null) {
    const waited = now.getTime() - since;
    if (waited > STALE_REVIEW_HOURS * HOUR_MS) {
      return { kind: "waiting", label: `Waiting ${span(waited)}`, title: `Waiting on review for ${span(waited)} (over ${STALE_REVIEW_HOURS}h)` };
    }
  }
  const idle = now.getTime() - lastActivity(row);
  if (idle > IDLE_DAYS * DAY_MS) {
    return { kind: "idle", label: `Idle ${span(idle)}`, title: `No activity or bump for ${span(idle)} (over ${IDLE_DAYS} days)` };
  }
  return null;
}

/** Comment counter text: hidden at zero, `unresolved/total` when there are threads. */
export function commentsText(unresolved: number, total: number): string | null {
  if (total <= 0 && unresolved <= 0) {
    return null;
  }
  const cap = (value: number) => (value >= 100 ? "100+" : String(value));
  return total > 0 ? `${cap(unresolved)}/${cap(total)}` : cap(unresolved);
}

/** What the list shows when the view itself has no PRs (not a search or filter miss). */
export type ViewEmpty = { kind: "rows" } | { kind: "no_reviews" } | { kind: "no_prs"; settings: boolean } | { kind: "empty_project" };

export function viewEmptyState(viewKey: string, inView: number, searching: boolean, filtered: boolean): ViewEmpty {
  if (inView > 0 || searching || filtered) {
    return { kind: "rows" };
  }
  if (viewKey === "to_review") {
    return { kind: "no_reviews" };
  }
  if (viewKey.startsWith("project:")) {
    return { kind: "empty_project" };
  }
  return { kind: "no_prs", settings: true };
}

/** Compact relative time for the compact row: `5m`, `2h`, `3d`, `4w`. */
export function shortAgo(iso: string, now: Date): string {
  const ms = Math.max(0, now.getTime() - new Date(iso).getTime());
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 60) {
    return `${Math.max(1, minutes)}m`;
  }
  const hours = Math.floor(ms / HOUR_MS);
  if (hours < 24) {
    return `${hours}h`;
  }
  const days = Math.floor(ms / DAY_MS);
  return days < 14 ? `${days}d` : `${Math.floor(days / 7)}w`;
}

/**
 * The Slack cell of a list row (normal and compact): a grey icon alone when
 * nothing was sent, else the coloured icon plus the time of the last send or
 * bump. Workspace, channel and action go in the tooltip.
 */
export function compactSlack(slack: ListRow["slack"], now: Date): { sent: boolean; time: string | null; title: string } {
  if (!slack) {
    return { sent: false, time: null, title: "Not requested on Slack" };
  }
  const last = slack.last_bumped_at ?? slack.requested_at;
  const verb = slack.last_bumped_at ? "bumped" : "requested";
  const time = `${shortAgo(last, now)} ago`;
  const workspace = slack.workspace_name ?? slack.workspace;
  const disabled = slack.workspace_disabled ? "workspace disabled in Settings → Slack" : "";
  return { sent: true, time, title: [workspace, channelLabel(slack), `${verb} ${time}`, disabled].filter(Boolean).join(" · ") };
}
