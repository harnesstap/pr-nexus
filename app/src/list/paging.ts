import type { ListRow, ProjectDto } from "../api";
import {
  AUTHOR,
  REVIEW_REQUESTED,
  filtersActive,
  normalizeSearch,
  passesMenu,
  quickMatches,
  QUICK_FILTERS,
  projectIdOf,
  scopeRow,
  type Direction,
  type Filters,
  type Grouping,
  type Ordering,
  type QuickFilter,
} from "./model";

/** Rows per `list_page` call. Bigger pages mean fewer round trips while scrolling. */
export const PAGE_SIZE = 50;

/**
 * One request for a slice of the list. The backend filters, searches, groups and
 * sorts every matching PR; only the slice after `cursor` comes back.
 */
export interface PageRequest {
  view_key: string;
  grouping: Grouping;
  ordering: Ordering;
  direction: Direction;
  filters: Filters;
  query: string;
  /** Section keys whose rows are skipped (the header still comes back). */
  collapsed: string[];
  cursor: string | null;
  limit: number;
  now: string;
  /** Local midnight and six days before it, for the Updated grouping. */
  day_start: string;
  week_start: string;
}

/** A section header (`row` null) or a row inside `section`. */
export interface PageEntry {
  section: string;
  row: ListRow | null;
}

export interface PageSection {
  key: string;
  title: string;
  project_id: number | null;
  /** Rows matching the filters and search in this section, loaded or not. */
  count: number;
  /** Every matching id, for select-all and the section checkbox. */
  ids: number[];
}

export interface PageSummary {
  open: number;
  finished: number;
  unassigned: number;
  live_projects: number;
}

export interface PageDto {
  entries: PageEntry[];
  /** Every section in order, including ones whose entries are not loaded yet. */
  sections: PageSection[];
  next_cursor: string | null;
  summary: PageSummary;
  /** Rows in the view before menu filters and search, for the empty state. */
  in_view: number;
  /** Org filter options with counts over every synced row. */
  orgs: { name: string; count: number }[];
  /** Rows per quick filter, over the menu filters and search but ignoring the quick filter. */
  quick: Record<QuickFilter, number>;
  /** Where else the search matches, so an empty result can point there. Null without a query. */
  elsewhere: Elsewhere | null;
}

export interface Elsewhere {
  /** Matches in the other nav views, same open/archived scope, before menu filters. */
  views: { view_key: string; count: number }[];
  /** Matches in this view hidden by the menu or quick filters. */
  filtered: number;
  /** Matches in this view that pass the filters but are merged, closed, or only in archived projects. */
  finished: number;
}

/** Nav views that list rows by involvement, in sidebar order. */
const NAV_VIEWS = ["mine", "to_review"] as const;

/** Row plus the fields only search needs. The backend keeps them in SQLite. */
export type SearchRow = ListRow & { body?: string };

interface Cursor {
  o: number;
  s: string;
  r: number | null;
}

export function encodeCursor(cursor: Cursor): string {
  return JSON.stringify(cursor);
}

function decodeCursor(value: string | null): Cursor | null {
  if (!value) {
    return null;
  }
  try {
    const raw = JSON.parse(value) as Partial<Cursor>;
    return typeof raw.o === "number" && typeof raw.s === "string" ? { o: raw.o, s: raw.s, r: typeof raw.r === "number" ? raw.r : null } : null;
  } catch {
    return null;
  }
}

/** Local day boundaries the backend uses for Today / This week / Older. */
export function dayBounds(now: Date): { day_start: string; week_start: string } {
  const start = new Date(now);
  start.setHours(0, 0, 0, 0);
  const week = new Date(start);
  week.setDate(week.getDate() - 6);
  return { day_start: start.toISOString(), week_start: week.toISOString() };
}

/** Header subtitle (§8.2) from backend counts: "5 open PRs across 2 projects · 1 unassigned". */
export function summaryText(summary: PageSummary, viewKey: string, syncProgress: string | null = null): string {
  const text = countsText(summary, viewKey);
  return syncProgress ? `${text} · ${syncProgress}` : text;
}

function countsText(summary: PageSummary, viewKey: string): string {
  const { open, finished, unassigned, live_projects: live } = summary;
  const extra = finished > 0 ? ` · ${finished} merged or closed` : "";
  const prs = `${open} open ${open === 1 ? "PR" : "PRs"}`;
  if (projectIdOf(viewKey) !== null) {
    return `${prs} in this project${extra}`;
  }
  const base = `${prs} across ${live} ${live === 1 ? "project" : "projects"}${extra}`;
  return unassigned > 0 && live > 0 ? `${base} · ${unassigned} unassigned` : base;
}

/** Every id across sections, once. */
export function pageIds(sections: readonly PageSection[]): number[] {
  return [...new Set(sections.flatMap((section) => section.ids))];
}

/**
 * Reference implementation of the backend `list_page` command, used by the
 * browser preview and tests. `app/src-tauri/src/listing.rs` must agree.
 */
export function pageFromRows(rows: readonly SearchRow[], projects: readonly ProjectDto[], request: PageRequest): PageDto {
  const filters = request.filters;
  const now = new Date(request.now);
  const nav = rowsForView(rows, request.view_key);
  const scoped = nav.filter((row) => scopeRow(row, projects, filters));
  const terms = normalizeSearch(request.query).split(/\s+/).filter((term) => term.length > 0);
  const matched = scoped.filter((row) => passesMenu(row, filters, now) && matchesTerms(row, projects, terms));
  const quick = Object.fromEntries(QUICK_FILTERS.map((key) => [key, matched.filter((row) => quickMatches(row, key)).length])) as Record<QuickFilter, number>;
  const shown = filters.quick ? matched.filter((row) => quickMatches(row, filters.quick as QuickFilter)) : matched;

  const live = displayed(projects, filters);
  const liveIds = new Set(live.map((project) => project.id));
  const open = shown.filter((row) => row.state === "open").length;
  const summary: PageSummary = {
    open,
    finished: shown.length - open,
    unassigned: shown.filter((row) => !row.project_ids.some((id) => liveIds.has(id))).length,
    live_projects: live.length,
  };

  const ordered = [...shown].sort((left, right) => compare(left, right, request.ordering, request.direction));
  const keepEmpty = keptEmpty(live, request.view_key, filters, terms.length > 0);
  const groups = group(ordered, live, request, keepEmpty);
  const collapsed = new Set(request.collapsed);
  const stream: PageEntry[] = [];
  for (const section of groups) {
    stream.push({ section: section.key, row: null });
    if (!collapsed.has(section.key)) {
      for (const row of section.rows) {
        stream.push({ section: section.key, row: listRow(row) });
      }
    }
  }
  const start = resume(stream, decodeCursor(request.cursor));
  const limit = Math.max(1, request.limit);
  const entries = stream.slice(start, start + limit);
  const end = start + entries.length;
  const last = entries[entries.length - 1];
  const orgCounts = new Map<string, number>();
  for (const row of rows) {
    orgCounts.set(row.owner, (orgCounts.get(row.owner) ?? 0) + 1);
  }
  return {
    entries,
    sections: groups.map((section) => ({
      key: section.key,
      title: section.title,
      project_id: section.projectId,
      count: section.rows.length,
      ids: section.rows.map((row) => row.id),
    })),
    next_cursor: end < stream.length && last ? encodeCursor({ o: end, s: last.section, r: last.row?.id ?? null }) : null,
    summary,
    in_view: scoped.length,
    orgs: [...orgCounts.entries()].sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0)).map(([name, count]) => ({ name, count })),
    quick,
    elsewhere: terms.length > 0 ? elsewhereFor(rows, projects, request, terms, now) : null,
  };
}

function elsewhereFor(rows: readonly SearchRow[], projects: readonly ProjectDto[], request: PageRequest, terms: readonly string[], now: Date): Elsewhere {
  const filters = request.filters;
  const found = rows.filter((row) => !row.hidden && matchesTerms(row, projects, terms));
  const passes = (row: SearchRow) => passesMenu(row, filters, now) && (!filters.quick || quickMatches(row, filters.quick));
  const here = rowsForView(found, request.view_key);
  return {
    views: NAV_VIEWS.filter((key) => key !== request.view_key).map((key) => ({
      view_key: key,
      count: rowsForView(found, key).filter((row) => scopeRow(row, projects, filters)).length,
    })),
    filtered: here.filter((row) => scopeRow(row, projects, filters) && !passes(row)).length,
    finished: here.filter((row) => !scopeRow(row, projects, filters) && passes(row)).length,
  };
}

/** Linked-PR candidates for the project dialog: open, visible, not linked, matching. */
export function candidatesFromRows(rows: readonly SearchRow[], projects: readonly ProjectDto[], linked: ReadonlySet<number>, query: string, limit: number): ListRow[] {
  const terms = normalizeSearch(query).split(/\s+/).filter((term) => term.length > 0);
  if (terms.length === 0) {
    return [];
  }
  const exact = query.trim().replace(/^#/, "");
  return rows
    .filter((row) => !linked.has(row.id) && !row.hidden && row.state === "open" && matchesTerms(row, projects, terms))
    .sort((left, right) => Number(String(right.number) === exact) - Number(String(left.number) === exact) || compare(left, right, "updated", "desc"))
    .slice(0, limit)
    .map(listRow);
}

function listRow(row: SearchRow): ListRow {
  const { body: _body, ...rest } = row;
  void _body;
  return rest;
}

function resume(stream: readonly PageEntry[], cursor: Cursor | null): number {
  if (!cursor) {
    return 0;
  }
  const at = stream.findIndex((entry) => entry.section === cursor.s && (entry.row?.id ?? null) === cursor.r);
  return at >= 0 ? at + 1 : Math.min(cursor.o, stream.length);
}

function rowsForView(rows: readonly SearchRow[], viewKey: string): SearchRow[] {
  if (viewKey === "mine") {
    return rows.filter((row) => (row.involvement & AUTHOR) !== 0);
  }
  if (viewKey === "to_review") {
    return rows.filter((row) => (row.involvement & REVIEW_REQUESTED) !== 0);
  }
  const project = projectIdOf(viewKey);
  if (project !== null) {
    return rows.filter((row) => row.project_ids.includes(project));
  }
  return [...rows];
}

/** Title, description, repo, #number, author, branch, and linked project names. */
export function matchesTerms(row: SearchRow, projects: readonly ProjectDto[], terms: readonly string[]): boolean {
  if (terms.length === 0) {
    return true;
  }
  const text = normalizeSearch([row.title, `${row.repo}`, `#${row.number}`, row.author_login, row.head_ref, row.body ?? ""].join("\n"));
  const names = projects.filter((project) => row.project_ids.includes(project.id)).map((project) => normalizeSearch(project.name));
  return terms.every((term) => text.includes(term) || names.some((name) => name.includes(term)));
}

function displayed(projects: readonly ProjectDto[], filters: Filters): ProjectDto[] {
  return projects
    .filter((project) => filters.archived || !project.archived)
    .slice()
    .sort((left, right) => left.sort_order - right.sort_order || (left.name < right.name ? -1 : left.name > right.name ? 1 : 0));
}

function keptEmpty(live: readonly ProjectDto[], viewKey: string, filters: Filters, searching: boolean): Set<number> {
  if (searching || filtersActive(filters)) {
    return new Set();
  }
  const current = projectIdOf(viewKey);
  return current !== null ? new Set([current]) : new Set(live.map((project) => project.id));
}

interface Group {
  key: string;
  title: string;
  projectId: number | null;
  rows: SearchRow[];
}

function group(rows: readonly SearchRow[], live: readonly ProjectDto[], request: PageRequest, keepEmpty: ReadonlySet<number>): Group[] {
  switch (request.grouping) {
    case "project": {
      const ids = new Set(live.map((project) => project.id));
      const sections: Group[] = live
        .map((project) => ({ key: `project:${project.id}`, title: project.name, projectId: project.id, rows: rows.filter((row) => row.project_ids.includes(project.id)) }))
        .filter((section) => section.rows.length > 0 || keepEmpty.has(section.projectId));
      const unassigned = rows.filter((row) => !row.project_ids.some((id) => ids.has(id)));
      if (unassigned.length > 0) {
        sections.push({ key: "unassigned", title: "Unassigned", projectId: null, rows: unassigned });
      }
      return sections;
    }
    case "repository":
      return groupBy(rows, (row) => row.repo);
    case "org":
      return groupBy(rows, (row) => row.owner);
    case "status":
      return groupBy(rows, (row) => statusTitle(row.readiness));
    case "updated": {
      const day = new Date(request.day_start).getTime();
      const week = new Date(request.week_start).getTime();
      const bucket = (row: SearchRow) => {
        const at = new Date(row.updated_at).getTime();
        return at >= day ? "Today" : at >= week ? "This week" : "Older";
      };
      return ["Today", "This week", "Older"]
        .map((title) => ({ key: title, title, projectId: null, rows: rows.filter((row) => bucket(row) === title) }))
        .filter((section) => section.rows.length > 0);
    }
    default: {
      const unreachable: never = request.grouping;
      return unreachable;
    }
  }
}

function groupBy(rows: readonly SearchRow[], keyOf: (row: SearchRow) => string): Group[] {
  const order: string[] = [];
  const buckets = new Map<string, SearchRow[]>();
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
  return order.map((key) => ({ key, title: key, projectId: null, rows: buckets.get(key) ?? [] }));
}

function statusTitle(readiness: ListRow["readiness"]): string {
  return readiness === "ready" ? "Ready" : readiness === "blocked" ? "Blocked" : "Unknown";
}

function compare(left: ListRow, right: ListRow, ordering: Ordering, direction: Direction): number {
  const sign = direction === "asc" ? 1 : -1;
  const delta = rank(left, ordering) - rank(right, ordering);
  return delta !== 0 ? delta * sign : left.id - right.id;
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
      return row.readiness === "ready" ? 0 : row.readiness === "unknown" ? 1 : 2;
    case "number":
      return row.number;
    default: {
      const unreachable: never = ordering;
      return unreachable;
    }
  }
}
