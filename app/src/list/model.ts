import type { BulkOutcome, BulkResult, ListRow, ProjectDto, SlackIndicator, SyncStatus } from "../api";
import { channelLabel } from "../api";

export const AUTHOR = 1;
export const REVIEW_REQUESTED = 2;
const BUMP_DUE_MS = 24 * 60 * 60 * 1000;

export type Grouping = "project" | "repository" | "org" | "status" | "updated";
export type Ordering = "updated" | "created" | "slack" | "readiness" | "number";
export type Direction = "asc" | "desc";
export type CheckState = "none" | "partial" | "all";
export type StatusFilter = "ready" | "unknown" | "blocked" | "draft";
export type ReviewFilter = "approved" | "changes_requested" | "review_required" | "none";
export type CiFilter = "passing" | "failing" | "pending" | "none";
export type SlackFilter = "none" | "requested" | "bumped" | "bump_due";

export interface ShowFlags {
  numberTitle: boolean;
  repo: boolean;
  ci: boolean;
  review: boolean;
  threads: boolean;
  teams: boolean;
  activity: boolean;
  badges: boolean;
  notRequested: boolean;
}

export interface Filters {
  orgs: string[];
  statuses: StatusFilter[];
  reviews: ReviewFilter[];
  ci: CiFilter[];
  slack: SlackFilter[];
  archived: boolean;
}

export interface SavedView {
  grouping: Grouping;
  ordering: Ordering;
  direction: Direction;
  show: ShowFlags;
  filters: Filters;
}

export interface Section {
  key: string;
  title: string;
  projectId: number | null;
  rows: ListRow[];
}

export function defaultShow(): ShowFlags {
  return {
    numberTitle: true,
    repo: true,
    ci: true,
    review: true,
    threads: true,
    teams: false,
    activity: false,
    badges: true,
    notRequested: true,
  };
}

export function defaultFilters(): Filters {
  return { orgs: [], statuses: [], reviews: [], ci: [], slack: [], archived: false };
}

export function defaultView(): SavedView {
  return {
    grouping: "project",
    ordering: "updated",
    direction: "desc",
    show: defaultShow(),
    filters: defaultFilters(),
  };
}

export function parseViewState(json: string | null): SavedView {
  const base = defaultView();
  if (!json) {
    return base;
  }
  let raw: unknown;
  try {
    raw = JSON.parse(json);
  } catch {
    return base;
  }
  if (!raw || typeof raw !== "object") {
    return base;
  }
  const record = raw as Partial<SavedView>;
  return {
    grouping: pick(record.grouping, ["project", "repository", "org", "status", "updated"], base.grouping),
    ordering: pick(record.ordering, ["updated", "created", "slack", "readiness", "number"], base.ordering),
    direction: pick(record.direction, ["asc", "desc"], base.direction),
    show: { ...base.show, ...(record.show ?? {}) },
    filters: {
      ...base.filters,
      ...(record.filters ?? {}),
      orgs: stringList(record.filters?.orgs),
      statuses: pickList(record.filters?.statuses, ["ready", "unknown", "blocked", "draft"]),
      reviews: pickList(record.filters?.reviews, ["approved", "changes_requested", "review_required", "none"]),
      ci: pickList(record.filters?.ci, ["passing", "failing", "pending", "none"]),
      slack: pickList(record.filters?.slack, ["none", "requested", "bumped", "bump_due"]),
      archived: record.filters?.archived === true,
    },
  };
}

export function filtersActive(filters: Filters): boolean {
  return (
    filters.orgs.length > 0 ||
    filters.statuses.length > 0 ||
    filters.reviews.length > 0 ||
    filters.ci.length > 0 ||
    filters.slack.length > 0 ||
    filters.archived
  );
}

export function resetFilters(view: SavedView): SavedView {
  return { ...view, filters: defaultFilters() };
}

export function checkState(visibleIds: readonly number[], selected: ReadonlySet<number>): CheckState {
  const unique = [...new Set(visibleIds)];
  if (unique.length === 0) {
    return "none";
  }
  const hits = unique.filter((id) => selected.has(id)).length;
  if (hits === 0) {
    return "none";
  }
  if (hits === unique.length) {
    return "all";
  }
  return "partial";
}

/** Selects every visible id, or clears them when they are already all selected. */
export function toggleParent(visibleIds: readonly number[], selected: ReadonlySet<number>): Set<number> {
  const next = new Set(selected);
  const visible = [...new Set(visibleIds)];
  if (checkState(visible, selected) === "all") {
    for (const id of visible) {
      next.delete(id);
    }
  } else {
    for (const id of visible) {
      next.add(id);
    }
  }
  return next;
}

export function hiddenSelectedCount(selected: ReadonlySet<number>, visibleIds: readonly number[]): number {
  const visible = new Set(visibleIds);
  let hidden = 0;
  for (const id of selected) {
    if (!visible.has(id)) {
      hidden += 1;
    }
  }
  return hidden;
}

export function bulkIds(selected: ReadonlySet<number>): number[] {
  return [...selected].sort((left, right) => left - right);
}

export function normalizeSearch(value: string): string {
  return value.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
}

export function passesMenu(row: ListRow, filters: Filters, now: Date): boolean {
  if (filters.orgs.length > 0 && !filters.orgs.some((org) => org.toLowerCase() === row.owner.toLowerCase())) {
    return false;
  }
  if (filters.statuses.length > 0 && !filters.statuses.some((status) => statusMatches(row, status))) {
    return false;
  }
  if (filters.reviews.length > 0 && !filters.reviews.includes(reviewBucket(row))) {
    return false;
  }
  if (filters.ci.length > 0 && !filters.ci.includes(ciBucket(row.checks))) {
    return false;
  }
  if (filters.slack.length > 0 && !filters.slack.some((kind) => slackKinds(row, now).includes(kind))) {
    return false;
  }
  return true;
}

/**
 * Merged and closed PRs leave the open list at the sync that sees them. The
 * Archived filter brings them back until `sync.keep_closed_days` hides them.
 */
export function scopeRow(row: ListRow, projects: readonly ProjectDto[], filters: Filters): boolean {
  if (row.hidden) {
    return false;
  }
  if (!isOpen(row) && !filters.archived) {
    return false;
  }
  const live = new Set(displayedProjects(projects, filters).map((project) => project.id));
  const linkedLive = row.project_ids.filter((id) => live.has(id));
  const onlyArchived = row.project_ids.length > 0 && linkedLive.length === 0;
  return !(onlyArchived && !filters.archived);
}

export function projectIdOf(viewKey: string): number | null {
  if (!viewKey.startsWith("project:")) {
    return null;
  }
  const id = Number(viewKey.slice("project:".length));
  return Number.isFinite(id) ? id : null;
}

export function isOpen(row: ListRow): boolean {
  return row.state === "open";
}

/** MIME type carried by dragged PR rows. */
export const DRAG_MIME = "application/x-pr-nexus-prs";

/** Dragging a selected row drags the whole selection; otherwise just that row. */
export function dragIds(rowId: number, selected: ReadonlySet<number>): number[] {
  return selected.has(rowId) ? bulkIds(selected) : [rowId];
}

export function parseDragIds(data: string): number[] {
  try {
    const value: unknown = JSON.parse(data);
    return Array.isArray(value) ? value.filter((id): id is number => Number.isInteger(id)) : [];
  } catch {
    return [];
  }
}

export function renameError(name: string, projects: readonly ProjectDto[], id: number | null): string | null {
  const trimmed = name.trim();
  if (!trimmed) {
    return "Name cannot be empty";
  }
  if (projects.some((project) => project.id !== id && project.name === trimmed)) {
    return "A project with that name already exists";
  }
  return null;
}

export function ago(iso: string, now: Date): string {
  const minutes = Math.round((now.getTime() - new Date(iso).getTime()) / 60_000);
  if (minutes < 1) {
    return "just now";
  }
  if (minutes < 60) {
    return `${minutes}m ago`;
  }
  const hours = Math.round(minutes / 60);
  if (hours < 24) {
    return `${hours}h ago`;
  }
  const days = Math.round(hours / 24);
  if (days < 14) {
    return `${days}d ago`;
  }
  return `${Math.round(days / 7)}w ago`;
}

export function syncLabel(sync: SyncStatus, now: Date): string {
  if (sync.phase === "demo") {
    return "Demo data · sync off";
  }
  if (sync.phase === "syncing") {
    return "Syncing…";
  }
  if (sync.paused || sync.phase === "paused") {
    return "Rate limited";
  }
  if (sync.phase === "error") {
    const reason = sync.message.replace(/^Sync failed · /, "");
    return `Sync failed · ${reason}`;
  }
  if (sync.synced_at) {
    return `Synced · ${ago(sync.synced_at, now)}`;
  }
  return "Not synced yet";
}

export function ciLabel(checks: string): string {
  const bucket = ciBucket(checks);
  switch (bucket) {
    case "passing":
      return "Passing";
    case "failing":
      return "Failing";
    case "pending":
      return "Pending";
    case "none":
      return "No checks";
    default: {
      const unreachable: never = bucket;
      return unreachable;
    }
  }
}

export function reviewLabel(decision: string | null): string | null {
  switch (decision) {
    case "approved":
      return "Approved";
    case "changes_requested":
      return "Changes requested";
    case "review_required":
      return "Review pending";
    case null:
      return null;
    default:
      return decision;
  }
}

function displayedProjects(projects: readonly ProjectDto[], filters: Filters): ProjectDto[] {
  return projects
    .filter((project) => filters.archived || !project.archived)
    .slice()
    .sort((left, right) => left.sort_order - right.sort_order || left.name.localeCompare(right.name));
}

function statusMatches(row: ListRow, status: StatusFilter): boolean {
  switch (status) {
    case "draft":
      return row.is_draft;
    case "ready":
      return !row.is_draft && row.readiness === "ready";
    case "unknown":
      return !row.is_draft && row.readiness === "unknown";
    case "blocked":
      return !row.is_draft && row.readiness === "blocked";
    default: {
      const unreachable: never = status;
      return unreachable;
    }
  }
}

function reviewBucket(row: ListRow): ReviewFilter {
  switch (row.review_decision) {
    case "approved":
      return "approved";
    case "changes_requested":
      return "changes_requested";
    case "review_required":
      return "review_required";
    case null:
      return "none";
    default:
      return "none";
  }
}

export function ciBucket(checks: string): CiFilter {
  switch (checks) {
    case "success":
      return "passing";
    case "failure":
    case "error":
      return "failing";
    case "pending":
    case "expected":
      return "pending";
    default:
      return "none";
  }
}

function slackKinds(row: ListRow, now: Date): SlackFilter[] {
  if (!row.slack) {
    return ["none"];
  }
  const kinds: SlackFilter[] = [row.slack.last_bumped_at ? "bumped" : "requested"];
  const stamp = row.slack.last_bumped_at ?? row.slack.requested_at;
  if (now.getTime() - new Date(stamp).getTime() >= BUMP_DUE_MS) {
    kinds.push("bump_due");
  }
  return kinds;
}

function pick<T extends string>(value: unknown, allowed: readonly T[], fallback: T): T {
  return typeof value === "string" && allowed.includes(value as T) ? (value as T) : fallback;
}

function pickList<T extends string>(value: unknown, allowed: readonly T[]): T[] {
  if (!Array.isArray(value)) {
    return [];
  }
  return value.filter((item): item is T => typeof item === "string" && allowed.includes(item as T));
}

function stringList(value: unknown): string[] {
  if (!Array.isArray(value)) {
    return [];
  }
  return value.filter((item): item is string => typeof item === "string");
}

export type Tone = "good" | "bad" | "warn" | "pending" | "muted";

export function reviewTone(decision: string | null): Tone {
  switch (decision) {
    case "approved":
      return "good";
    case "changes_requested":
      return "bad";
    case "review_required":
      return "pending";
    default:
      return "muted";
  }
}

export function ciTone(checks: string): Tone {
  switch (ciBucket(checks)) {
    case "passing":
      return "good";
    case "failing":
      return "bad";
    case "pending":
      return "warn";
    default:
      return "muted";
  }
}

/** Unresolved review-thread count as shown in a row, plus its accessible description. */
export function threadsLabel(unresolved: number, total: number): { text: string; title: string } {
  const text = unresolved >= 100 ? "100+" : String(unresolved);
  const noun = total === 1 ? "review thread" : "review threads";
  return { text, title: `${text} unresolved of ${total >= 100 ? "100+" : total} ${noun}` };
}

/** Row Slack indicator: the newest event (bump wins over the request) and its age. */
export function slackStatus(slack: ListRow["slack"], now: Date): { label: string; title: string } | null {
  if (!slack) {
    return null;
  }
  const action = requestAction(slack, now);
  return { label: action.label, title: `${slack.workspace} · ${channelLabel(slack)} · ${action.title}` };
}

/**
 * The last thing done on a review request: `Bumped · 9h ago` or
 * `Requested · 2d ago`. Shared by the list row and the PR detail Slack card.
 */
export function requestAction(
  request: { requested_at: string; last_bumped_at: string | null; bump_count: number },
  now: Date,
): { label: string; title: string } {
  const requested = `requested ${ago(request.requested_at, now)}`;
  if (request.last_bumped_at) {
    const bumps = request.bump_count === 1 ? "1 bump" : `${request.bump_count} bumps`;
    return { label: `Bumped · ${ago(request.last_bumped_at, now)}`, title: `${requested} · ${bumps}` };
  }
  return { label: `Requested · ${ago(request.requested_at, now)}`, title: requested };
}

export const TOAST_MS = 6000;

export interface Toast {
  text: string;
  /** Opens the posted Slack message. */
  link?: string | null;
}

/** Success toast for a single request or bump. */
export function sentToast(slack: SlackIndicator): Toast {
  const what = slack.bump_count > 0 ? "Bumped" : "Review requested";
  return { text: `${what} in ${slack.workspace} · ${channelLabel(slack)}`, link: slack.permalink };
}

/** One-line summary of a bulk run: `Slack: 2 sent · 1 skipped · 1 failed`. */
export function bulkToast(done: BulkResult): string {
  const count = (outcome: BulkOutcome["outcome"]) => done.results.filter((item) => item.outcome === outcome).length;
  const parts = (["sent", "skipped", "failed"] as const).filter((label) => count(label) > 0).map((label) => `${count(label)} ${label}`);
  return `Slack: ${parts.length > 0 ? parts.join(" · ") : "nothing sent"}`;
}
