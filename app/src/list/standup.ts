import type { ListRow } from "../api";
import { escapeLinkText } from "./markdown";
import { ciBucket, defaultFilters } from "./model";
import { dayBounds, type PageDto, type PageRequest } from "./paging";
import { waitingSince } from "./rowCues";

const HOUR_MS = 60 * 60 * 1000;

/** A review the signed-in user submitted, when that time is known. */
export interface ReviewedPr {
  title: string;
  url: string;
  /** When the review was submitted. */
  at: string;
}

const MINE = ["merged", "ready", "waiting", "changes", "ci", "drafts"] as const;
type MineSection = (typeof MINE)[number];

const TITLES: Record<MineSection, string> = {
  merged: "Merged",
  ready: "Ready to merge",
  waiting: "Waiting for review",
  changes: "Changes requested",
  ci: "CI failing",
  drafts: "Drafts",
};

const WEEKDAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"] as const;

/**
 * Local midnight of the previous weekday. Tuesday starts at Monday 00:00;
 * Monday, Saturday and Sunday start at Friday 00:00.
 */
export function lastWorkingDayStart(now: Date): Date {
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  do {
    start.setDate(start.getDate() - 1);
  } while (start.getDay() === 0 || start.getDay() === 6);
  return start;
}

function waitLabel(ms: number): string {
  if (ms < HOUR_MS) {
    return "<1h";
  }
  const hours = Math.floor(ms / HOUR_MS);
  return hours < 72 ? `${hours}h` : `${Math.floor(hours / 24)}d`;
}

function line(title: string, url: string, suffix = ""): string {
  return `- [${escapeLinkText(title)}](${url.trim()})${suffix}`;
}

/**
 * One bucket per open or recently merged PR. First match wins, so a draft
 * with red CI stays under Drafts. Display order is separate.
 */
function classify(row: ListRow, cutoffMs: number): MineSection | null {
  if (row.state === "merged" && new Date(row.updated_at).getTime() >= cutoffMs) {
    return "merged";
  }
  if (row.state !== "open") {
    return null;
  }
  if (row.is_draft) {
    return "drafts";
  }
  if (row.review_decision === "changes_requested") {
    return "changes";
  }
  const ci = ciBucket(row.checks);
  if (row.review_decision === "approved" && ci !== "pending" && ci !== "failing" && !row.conflicts) {
    return "ready";
  }
  if (ci === "failing") {
    return "ci";
  }
  if ((row.reviewers_requested || row.slack !== null) && row.review_decision !== "approved") {
    return "waiting";
  }
  return null;
}

function byUpdatedDesc(left: ListRow, right: ListRow): number {
  return new Date(right.updated_at).getTime() - new Date(left.updated_at).getTime() || left.id - right.id;
}

/**
 * Markdown stand-up for My PRs. Time is `now` (injected). Merged rows use
 * `updated_at` as the merge time, which is what the list stores. Reviewed by
 * me is omitted when `reviewed` is empty: the list does not store that time.
 */
/** My PRs, including merged rows still kept locally, for the stand-up recap. */
export async function loadStandupRows(load: (request: PageRequest) => Promise<PageDto>, now: Date): Promise<ListRow[]> {
  const filters = { ...defaultFilters(), archived: true };
  const byId = new Map<number, ListRow>();
  let cursor: string | null = null;
  for (let page = 0; page < 4; page += 1) {
    const result = await load({
      view_key: "mine",
      grouping: "project",
      ordering: "updated",
      direction: "desc",
      filters,
      query: "",
      collapsed: [],
      cursor,
      limit: 5000,
      now: now.toISOString(),
      ...dayBounds(now),
    });
    for (const entry of result.entries) {
      if (entry.row) {
        byId.set(entry.row.id, entry.row);
      }
    }
    cursor = result.next_cursor;
    if (!cursor) {
      break;
    }
  }
  return [...byId.values()];
}

export function standupRecap(rows: readonly ListRow[], now: Date, reviewed: readonly ReviewedPr[] = []): string {
  const cutoff = lastWorkingDayStart(now);
  const cutoffMs = cutoff.getTime();
  const unique = new Map<number, ListRow>();
  for (const row of rows) {
    if (!unique.has(row.id)) {
      unique.set(row.id, row);
    }
  }
  const grouped = new Map<MineSection, ListRow[]>();
  for (const row of unique.values()) {
    const section = classify(row, cutoffMs);
    if (section === null) {
      continue;
    }
    const bucket = grouped.get(section);
    if (bucket) {
      bucket.push(row);
    } else {
      grouped.set(section, [row]);
    }
  }
  for (const section of MINE) {
    const bucket = grouped.get(section);
    if (!bucket) {
      continue;
    }
    if (section === "waiting") {
      bucket.sort((left, right) => (waitingSince(left) ?? new Date(left.created_at).getTime()) - (waitingSince(right) ?? new Date(right.created_at).getTime()) || left.id - right.id);
    } else {
      bucket.sort(byUpdatedDesc);
    }
  }

  const blocks: string[] = [`## Stand-up · since ${WEEKDAYS[cutoff.getDay()]}`, ""];
  let any = false;
  for (const section of MINE) {
    const bucket = grouped.get(section);
    if (!bucket || bucket.length === 0) {
      continue;
    }
    any = true;
    const lines = bucket.map((row) => {
      if (section !== "waiting") {
        return line(row.title, row.url);
      }
      const since = waitingSince(row) ?? new Date(row.created_at).getTime();
      return line(row.title, row.url, ` · ${waitLabel(Math.max(0, now.getTime() - since))}`);
    });
    blocks.push(`### ${TITLES[section]}`, ...lines, "");
  }

  const reviews = reviewed
    .filter((pr) => new Date(pr.at).getTime() >= cutoffMs)
    .slice()
    .sort((left, right) => new Date(right.at).getTime() - new Date(left.at).getTime());
  if (reviews.length > 0) {
    any = true;
    blocks.push("### Reviewed by me", ...reviews.map((pr) => line(pr.title, pr.url)), "");
  }

  if (!any) {
    blocks.push("Nothing to report.", "");
  }
  return `${blocks.join("\n").replace(/\n+$/, "")}\n`;
}
