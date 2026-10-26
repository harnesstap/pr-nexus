import type { ListRow, ProjectDto, SyncStatus } from "../api";
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

export function matchesSearch(row: ListRow, projectNames: readonly string[], query: string): boolean {
  const terms = normalizeSearch(query).split(/\s+/).filter((term) => term.length > 0);
  if (terms.length === 0) {
    return true;
  }
  const haystack = normalizeSearch(
    [...projectNames, row.title, row.body, row.repo, row.owner, `#${row.number}`, String(row.number)].join("\n"),
  );
  return terms.every((term) => haystack.includes(term));
}

export function rowsForNav(rows: readonly ListRow[], viewKey: string): ListRow[] {
  if (viewKey === "mine") {
    return rows.filter((row) => (row.involvement & AUTHOR) !== 0);
  }
  if (viewKey === "to_review") {
    return rows.filter((row) => (row.involvement & REVIEW_REQUESTED) !== 0);
  }
  if (viewKey.startsWith("project:")) {
    const id = Number(viewKey.slice("project:".length));
    return rows.filter((row) => row.project_ids.includes(id));
  }
  return [...rows];
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

export function scopeRow(row: ListRow, projects: readonly ProjectDto[], filters: Filters): boolean {
  if (row.hidden && !filters.archived) {
    return false;
  }
  const live = new Set(displayedProjects(projects, filters).map((project) => project.id));
  const linkedLive = row.project_ids.filter((id) => live.has(id));
  const onlyArchived = row.project_ids.length > 0 && linkedLive.length === 0;
  return !(onlyArchived && !filters.archived);
}

export function visibleRows(
  rows: readonly ListRow[],
  projects: readonly ProjectDto[],
  viewKey: string,
  view: SavedView,
  query: string,
  now: Date,
): ListRow[] {
  return rowsForNav(rows, viewKey).filter((row) => {
    if (!scopeRow(row, projects, view.filters)) {
      return false;
    }
    if (!passesMenu(row, view.filters, now)) {
      return false;
    }
    const names = projects.filter((project) => row.project_ids.includes(project.id)).map((project) => project.name);
    return matchesSearch(row, names, query);
  });
}

/**
 * Which empty project sections stay visible in Project grouping, so they can be
 * renamed and receive dropped PRs (§8.2). Search or menu filters hide them; a
 * project view keeps only its own section.
 */
export function keptEmptyProjects(
  projects: readonly ProjectDto[],
  viewKey: string,
  filters: Filters,
  query: string,
): Set<number> {
  if (query.trim() || filtersActive(filters)) {
    return new Set();
  }
  const current = projectIdOf(viewKey);
  if (current !== null) {
    return new Set([current]);
  }
  return new Set(displayedProjects(projects, filters).map((project) => project.id));
}

export function projectIdOf(viewKey: string): number | null {
  if (!viewKey.startsWith("project:")) {
    return null;
  }
  const id = Number(viewKey.slice("project:".length));
  return Number.isFinite(id) ? id : null;
}

/** Header subtitle (§8.2): "5 open PRs across 2 projects", or "… in this project". */
export function headerSummary(
  shown: readonly ListRow[],
  projects: readonly ProjectDto[],
  viewKey: string,
  filters: Filters,
): string {
  const prs = `${shown.length} open ${shown.length === 1 ? "PR" : "PRs"}`;
  if (projectIdOf(viewKey) !== null) {
    return `${prs} in this project`;
  }
  const live = displayedProjects(projects, filters);
  const ids = new Set(live.map((project) => project.id));
  const unassigned = shown.filter((row) => !row.project_ids.some((id) => ids.has(id))).length;
  const base = `${prs} across ${live.length} ${live.length === 1 ? "project" : "projects"}`;
  return unassigned > 0 && live.length > 0 ? `${base} · ${unassigned} unassigned` : base;
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

export function groupRows(
  rows: readonly ListRow[],
  projects: readonly ProjectDto[],
  view: SavedView,
  now: Date,
  keepEmpty: ReadonlySet<number> = new Set(),
): Section[] {
  const ordered = [...rows].sort((left, right) => compareRows(left, right, view.ordering, view.direction));
  switch (view.grouping) {
    case "project":
      return groupByProject(ordered, projects, view.filters, keepEmpty);
    case "repository":
      return groupBy(ordered, (row) => row.repo);
    case "org":
      return groupBy(ordered, (row) => row.owner);
    case "status":
      return groupBy(ordered, (row) => statusLabel(row.readiness));
    case "updated":
      return ["Today", "This week", "Older"]
        .map((title) => ({
          key: title,
          title,
          projectId: null,
          rows: ordered.filter((row) => updatedBucket(row.updated_at, now) === title),
        }))
        .filter((section) => section.rows.length > 0);
    default: {
      const unreachable: never = view.grouping;
      return unreachable;
    }
  }
}

export function sectionIds(section: Section): number[] {
  return section.rows.map((row) => row.id);
}

export function visibleIds(sections: readonly Section[]): number[] {
  return [...new Set(sections.flatMap((section) => sectionIds(section)))];
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

function groupByProject(
  rows: readonly ListRow[],
  projects: readonly ProjectDto[],
  filters: Filters,
  keepEmpty: ReadonlySet<number>,
): Section[] {
  const shown = displayedProjects(projects, filters);
  const ids = new Set(shown.map((project) => project.id));
  const sections: Section[] = shown
    .map((project) => ({
      key: `project:${project.id}`,
      title: project.name,
      projectId: project.id,
      rows: rows.filter((row) => row.project_ids.includes(project.id)),
    }))
    .filter((section) => section.rows.length > 0 || keepEmpty.has(section.projectId));
  const unassigned = rows.filter((row) => !row.project_ids.some((id) => ids.has(id)));
  if (unassigned.length > 0) {
    sections.push({ key: "unassigned", title: "Unassigned", projectId: null, rows: unassigned });
  }
  return sections;
}

function groupBy(rows: readonly ListRow[], keyOf: (row: ListRow) => string): Section[] {
  const order: string[] = [];
  const buckets = new Map<string, ListRow[]>();
  for (const row of rows) {
    const key = keyOf(row);
    const bucket = buckets.get(key);
    if (bucket) {
      bucket.push(row);
    } else {
      order.push(key);
      buckets.set(key, [row]);
    }
  }
  return order.map((key) => ({
    key,
    title: key,
    projectId: null,
    rows: buckets.get(key) ?? [],
  }));
}

function compareRows(left: ListRow, right: ListRow, ordering: Ordering, direction: Direction): number {
  const sign = direction === "asc" ? 1 : -1;
  const delta = rank(left, ordering) - rank(right, ordering);
  if (delta !== 0) {
    return delta * sign;
  }
  return left.id - right.id;
}

function rank(row: ListRow, ordering: Ordering): number {
  switch (ordering) {
    case "updated":
      return new Date(row.updated_at).getTime();
    case "created":
      return new Date(row.created_at).getTime();
    case "slack":
      return row.slack ? new Date(row.slack.requested_at).getTime() : 0;
    case "readiness":
      return readinessRank(row.readiness);
    case "number":
      return row.number;
    default: {
      const unreachable: never = ordering;
      return unreachable;
    }
  }
}

function readinessRank(readiness: ListRow["readiness"]): number {
  switch (readiness) {
    case "ready":
      return 0;
    case "unknown":
      return 1;
    case "blocked":
      return 2;
    default: {
      const unreachable: never = readiness;
      return unreachable;
    }
  }
}

function statusLabel(readiness: ListRow["readiness"]): string {
  switch (readiness) {
    case "ready":
      return "Ready";
    case "unknown":
      return "Unknown";
    case "blocked":
      return "Blocked";
    default: {
      const unreachable: never = readiness;
      return unreachable;
    }
  }
}

function updatedBucket(iso: string, now: Date): "Today" | "This week" | "Older" {
  const then = new Date(iso);
  const start = new Date(now);
  start.setHours(0, 0, 0, 0);
  if (then >= start) {
    return "Today";
  }
  const week = new Date(start);
  week.setDate(week.getDate() - 6);
  if (then >= week) {
    return "This week";
  }
  return "Older";
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
  const where = `${slack.workspace} · ${channelLabel(slack)}`;
  const requested = `requested ${ago(slack.requested_at, now)}`;
  if (slack.last_bumped_at) {
    const bumps = slack.bump_count === 1 ? "1 bump" : `${slack.bump_count} bumps`;
    return { label: `Bumped · ${ago(slack.last_bumped_at, now)}`, title: `${where} · ${requested} · ${bumps}` };
  }
  return { label: `Requested · ${ago(slack.requested_at, now)}`, title: `${where} · ${requested}` };
}
