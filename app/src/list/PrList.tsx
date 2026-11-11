import { useEffect, useMemo, useRef, useState, type ReactElement } from "react";
import type { ListRow, ProjectDto } from "../api";
import type { StackPlace } from "./stack";
import { AlertIcon, ChevronDownIcon, ChevronRightIcon, ClockIcon, CommentIcon, DraftIcon, FunnelIcon, MoreIcon, SlackLogo, StackIcon } from "../icons";
import { SearchField } from "./SearchField";
import { Tooltip } from "../Tooltip";
import { ReviewerAvatars } from "./ReviewerAvatars";
import { NextActionBadge } from "./NextActionBadge";
import { QuickFilters } from "./QuickFilters";
import { RepoPicker } from "./RepoPicker";
import { AddToProjectMenu } from "./AddToProjectMenu";
import { SelectionBar } from "./SelectionBar";
import { GitHubRowButton, StarButton } from "./StarButton";
import { ProjectInfo } from "./JiraChips";
import { isTypingTarget, wantsRowEnter } from "./keys";
import { copyText, prsToMarkdown } from "./markdown";
import { DEFAULT_REVIEW_SLA, wipHint, type ReviewSla } from "./sla";
import type { SnoozeChoice, Toast } from "./model";
import { displayKey, hint, matchShortcut, shortcut, type ShortcutId } from "./shortcuts";
import {
  ago,
  bulkIds,
  checkState,
  ciLabel,
  DRAG_MIME,
  dragIds,
  parseDragIds,
  projectIdOf,
  ciTone,
  filtersActive,
  hiddenSelectedCount,
  withSearch,
  toggleGlobal,
  visibleSelection,
  renameError,
  resetFilters,
  threadsLabel,
  toggleParent,
  type CheckState,
  type CiFilter,
  type Direction,
  type Grouping,
  type Ordering,
  type ReviewFilter,
  type SavedView,
  type Section,
  type ShowFlags,
  type SlackFilter,
  type StatusFilter,
  type Tone,
} from "./model";
import { setDragBadge } from "./dragBadge";
import { pageIds, selectionCountText, summaryText, type Elsewhere, type PageDto, type PageEntry, type PageRequest, type PageSection } from "./paging";
import { usePagedList, type PageParams } from "./usePagedList";
import { plainText } from "../textInput";
import { githubQueryOf, searchFieldValue, sortLocksOrder, splitSearch } from "./searchText";
import { queueChip } from "../detail/mergeQueue";
import { commentsText, compactSlack, rowPills, staleCue, viewEmptyState, type RowPill, type StaleCue } from "./rowCues";

/** A section as drawn: its loaded rows, plus counts and ids over every match. */
type ShownSection = Section & { count: number; ids: number[] };

/** How close to the bottom (px) the next page starts loading. */
const LOAD_AHEAD_PX = 600;
/** Search waits this long after the last keystroke (§8.2). */
export const SEARCH_DEBOUNCE_MS = 200;

interface Props {
  /** `list_page`: the backend filters, searches, groups and pages. */
  loadPage: (request: PageRequest) => Promise<PageDto>;
  /** Bumped when stored rows change (sync, links, Slack); reloads in place. */
  version: number;
  /** "Syncing my PRs… 42" while a sync runs, shown after the summary. */
  syncProgress?: string | null;
  /** Reads more rows from GitHub once the stored ones are all shown (To review). */
  remoteMore?: (() => void) | null;
  /** The debounced search, also run against GitHub (To review). */
  onRemoteSearch?: (terms: string) => void;
  projects: ProjectDto[];
  viewKey: string;
  view: SavedView;
  onViewChange: (view: SavedView) => void;
  /** Switch the nav view, keeping the search; for "matches in To review". */
  onShowView?: (viewKey: string) => void;
  now: Date;
  onOpen: (url: string) => void;
  onRename: (id: number, name: string) => Promise<void>;
  onArchive: (id: number) => void;
  onDelete: (id: number) => void;
  /** Opens the project dialog (rules, linked PRs) from the section ⋯ menu. */
  onEditProject: (id: number) => void;
  /** Manual links from dropping rows on a project section, or "Add to project". */
  onLinkPrs: (projectId: number, prIds: number[]) => void;
  /** "Add to project → New project from selection…"; rejects with the reason. */
  onCreateProjectWithPrs: (name: string, prIds: number[]) => Promise<void>;
  /** The current view's selection, for the sidebar's "Add N selected PRs". */
  onSelectionChange?: (prIds: number[]) => void;
  /** Opens the detail panel for one row. */
  onDetails: (row: ListRow) => void;
  /** The PR the detail panel shows; moving the focus then switches it. */
  detailId?: number | null;
  /** Opens the Slack request/bump preview for one row. */
  onRequestReview: (row: ListRow) => void;
  /** Stars or unstars a PR (the row's star button and Shift+S). */
  onToggleStar?: (row: ListRow) => void;
  /** Two or more PRs from the bottom bar (§7.9). */
  onBulkReview: (ids: number[]) => void;
  /** Opens the close confirmation for the active selection. */
  onClosePullRequests?: (rows: { id: number; repo: string; number: number; title: string }[]) => void;
  /** Marks these pull requests read. */
  onMarkRead?: (ids: number[]) => void;
  /** Marks every id in the current filtered view read. */
  onMarkAllRead?: (ids: number[]) => void;
  /** Snoozes or unsnoozes these pull requests. */
  onSnooze?: (ids: number[], choice: SnoozeChoice) => void;
  /** Mutes or unmutes these pull requests. */
  onMute?: (ids: number[], muted: boolean) => void;
  /** Opens the merge-queue confirmation for the active selection. */
  onEnqueuePullRequests?: (rows: ListRow[]) => void;
  /** Shows a toast, e.g. after "Copy as Markdown". */
  onNotice?: (toast: Toast) => void;
  /** Draft flags changed in the app ahead of the reload, by PR id. */
  draftPatches?: Record<number, boolean>;
  /** PRs just merged or closed in the app: gone from the list before the reload lands. */
  removedIds?: ReadonlySet<number>;
  /**
   * Ids from the view's GitHub query, in search order. Null when the view has
   * no query. An empty list (still searching, or nothing matched) shows no rows.
   */
  githubIds?: number[] | null;
  /** The query asked for merged or closed pull requests, so they stay on screen. */
  githubAnyState?: boolean;
  /** A plain sentence when GitHub rejected the query. */
  githubError?: string;
  /** The query is in flight. */
  githubSearching?: boolean;
  /** Fetches the next page of the GitHub query. */
  githubMore?: (() => void) | null;
  /** Opens Settings, from the "no open PRs" empty state. */
  onOpenSettings?: () => void;
  /** Review wait from Settings. Defaults to 48 hours, every day. */
  sla?: ReviewSla;
  /** Optional open-PR cap for My PRs. Null means no hint. */
  wipLimit?: number | null;
  /** Copies the My PRs stand-up recap. */
  onCopyStandup?: () => void;
  /** Opens the toolbar More menu on first paint (preview). */
  standupMenuInitiallyOpen?: boolean;
  /** Filled with the list's actions so the command palette reuses them. */
  actionsRef?: { current: ListActions | null };
}

/** What the command palette can ask of the list. */
export interface ListActions {
  run: (id: ShortcutId) => boolean;
  focused: () => ListRow | null;
  selectedCount: number;
}

/** Controls inside a row that keep their own click behaviour. */
const ROW_CONTROLS = "a, button, input, select, textarea, label, summary, [role='button'], [role='menu'], [role='menuitem'], [role='checkbox'], [tabindex]";

/** Whether a click on a row should open its detail panel: not on a control, not ending a text selection. */
export function rowClickOpens(target: EventTarget, row: Element): boolean {
  const control = target instanceof Element ? target.closest(ROW_CONTROLS) : null;
  if (control && control !== row && row.contains(control)) {
    return false;
  }
  const selection = window.getSelection();
  return !selection || selection.isCollapsed;
}

/** Below this list width rows switch to the compact two-line layout. */
const COMPACT_LIST_PX = 820;

const GROUPINGS: { id: Grouping; label: string }[] = [
  { id: "project", label: "Project" },
  { id: "repository", label: "Repository" },
  { id: "org", label: "Org" },
  { id: "status", label: "Status" },
  { id: "updated", label: "Updated" },
  { id: "stack", label: "Stack" },
];

const ORDERINGS: { id: Ordering; label: string }[] = [
  { id: "attention", label: "Needs attention" },
  { id: "updated", label: "Updated" },
  { id: "created", label: "Created" },
  { id: "slack", label: "Last Slack request" },
  { id: "readiness", label: "Readiness" },
  { id: "number", label: "Number" },
  { id: "waiting", label: "Waiting longest" },
  { id: "bumped", label: "Last bumped" },
];

export function PrList({
  loadPage,
  version,
  syncProgress = null,
  remoteMore = null,
  onRemoteSearch,
  projects,
  viewKey,
  view,
  onViewChange,
  onShowView,
  now,
  onOpen,
  onRename,
  onArchive,
  onDelete,
  onEditProject,
  onLinkPrs,
  onCreateProjectWithPrs,
  onSelectionChange,
  onDetails,
  detailId = null,
  onRequestReview,
  onToggleStar,
  onBulkReview,
  onClosePullRequests,
  onMarkRead,
  onMarkAllRead,
  onSnooze,
  onMute,
  onEnqueuePullRequests,
  onNotice,
  draftPatches,
  removedIds,
  githubIds = null,
  githubAnyState = false,
  githubError = "",
  githubSearching = false,
  githubMore = null,
  actionsRef,
  onOpenSettings,
  sla = DEFAULT_REVIEW_SLA,
  wipLimit = null,
  onCopyStandup,
  standupMenuInitiallyOpen = false,
}: Props) {
  const field = searchFieldValue(view.filters);
  const localQuery = splitSearch(field).text;
  const [debounced, setDebounced] = useState(() => localQuery.trim());
  const [selectedByView, setSelectedByView] = useState<Record<string, number[]>>({});
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [menu, setMenu] = useState<string | null>(null);
  const [standupMenu, setStandupMenu] = useState(standupMenuInitiallyOpen);
  const standupMenuRef = useRef<HTMLDivElement>(null);
  const [sectionMenu, setSectionMenu] = useState<string | null>(null);
  const [editing, setEditing] = useState<Editing | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const [deleteFor, setDeleteFor] = useState<Section | null>(null);
  const [focusIndex, setFocusIndex] = useState(0);
  /** The view whose "Add to project" menu is open, so switching views closes it. */
  const [addMenuFor, setAddMenuFor] = useState<string | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const commitRename = useRef(false);
  // A PR merged in the app drops out of the selection too.
  const selected = useMemo(
    () => new Set((selectedByView[viewKey] ?? []).filter((id) => !removedIds?.has(id))),
    [selectedByView, viewKey, removedIds],
  );

  const addMenu = addMenuFor === viewKey && selected.size > 0;
  const setAddMenu = (open: boolean | ((open: boolean) => boolean)) =>
    setAddMenuFor((current) => {
      const next = typeof open === "function" ? open(current === viewKey) : open;
      return next ? viewKey : null;
    });

  useEffect(() => {
    const timer = window.setTimeout(() => setDebounced(localQuery.trim()), SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [localQuery]);

  function setSearch(value: string) {
    onViewChange(withSearch(view, value));
  }

  const collapsedKeys = useMemo(
    () =>
      Object.keys(collapsed)
        .filter((key) => collapsed[key])
        .sort(),
    [collapsed],
  );
  const params = useMemo<PageParams>(
    () => ({
      view_key: viewKey,
      grouping: view.grouping,
      ordering: view.ordering,
      direction: view.direction,
      filters: { ...view.filters, search: "", github: githubQueryOf(view.filters) },
      query: debounced,
      collapsed: collapsedKeys,
      github_ids: githubIds,
      github_any_state: githubAnyState,
      review_sla_hours: sla.hours,
      review_sla_working_days: sla.workingDays,
    }),
    [viewKey, view.grouping, view.ordering, view.direction, view.filters, debounced, collapsedKeys, githubIds, githubAnyState, sla.hours, sla.workingDays],
  );
  const list = usePagedList(loadPage, params, version, now);
  // To review also asks GitHub for matches past the loaded window.
  useEffect(() => {
    if (debounced !== "") {
      onRemoteSearch?.(debounced);
    }
  }, [debounced, onRemoteSearch]);
  const page = list.page;
  const sections = useMemo(
    () =>
      withoutRemoved(shownSections(list.entries, page?.sections ?? []), removedIds).map((section) =>
        draftPatches && Object.keys(draftPatches).length > 0
          ? { ...section, rows: section.rows.map((row) => (row.id in draftPatches ? { ...row, is_draft: draftPatches[row.id] } : row)) }
          : section,
      ),
    [list.entries, page, draftPatches, removedIds],
  );
  const currentProject = projectIdOf(viewKey);
  const flat = useMemo(() => sections.flatMap((section) => section.rows.map((row) => ({ section, row }))), [sections]);
  // Every matching id, loaded or not: select-all, counts and drag cover them all.
  const ids = useMemo(() => pageIds(page?.sections ?? []).filter((id) => !removedIds?.has(id)), [page, removedIds]);
  const hidden = hiddenSelectedCount(selected, ids);
  const global = checkState(ids, selected);
  // One source of truth for counts and bulk actions: the selected rows that match now.
  const active = useMemo(() => visibleSelection(selected, ids), [selected, ids]);
  const activeIds = bulkIds(active);
  const activeKey = activeIds.join(",");
  useEffect(() => {
    onSelectionChange?.(activeKey === "" ? [] : activeKey.split(",").map(Number));
  }, [activeKey, onSelectionChange]);
  const singleSelected = active.size === 1 ? (flat.find((item) => active.has(item.row.id))?.row ?? null) : null;
  // Opening a pull request from the stack moves the highlight with the panel.
  const [seenDetail, setSeenDetail] = useState(detailId);
  if (seenDetail !== detailId) {
    setSeenDetail(detailId);
    if (detailId !== null) {
      const index = flat.findIndex((item) => item.row.id === detailId);
      if (index >= 0) {
        setFocusIndex(index);
      }
    }
  }
  // The focused row stays in range when filters or a sync shrink the list.
  const focus = flat.length === 0 ? -1 : Math.min(focusIndex, flat.length - 1);
  const listRef = useRef<HTMLDivElement>(null);
  const sentinelRef = useRef<HTMLDivElement>(null);
  const sentinelVisible = useRef(false);
  const inView = page?.in_view ?? 0;
  const searching = localQuery.trim() !== "" && (localQuery.trim() !== debounced || list.pending);
  const { loadMore, hasMore, loadingMore } = list;
  const [narrow, setNarrow] = useState(false);
  // The panel takes most of the width; a narrow window does the same.
  const compact = detailId !== null || narrow;
  const viewEmpty = page ? viewEmptyState(viewKey, inView, localQuery.trim() !== "", filtersActive(view.filters)) : { kind: "rows" as const };
  const showSections = viewEmpty.kind === "rows" || viewEmpty.kind === "empty_project";

  useEffect(() => {
    const root = listRef.current;
    if (!root || typeof ResizeObserver === "undefined") {
      return;
    }
    const observer = new ResizeObserver(() => setNarrow(root.clientWidth < COMPACT_LIST_PX));
    observer.observe(root);
    return () => observer.disconnect();
  }, []);

  // Infinite scroll: load the next page when the sentinel nears the viewport.
  useEffect(() => {
    const root = listRef.current;
    const sentinel = sentinelRef.current;
    if (!root || !sentinel || typeof IntersectionObserver === "undefined") {
      return;
    }
    const observer = new IntersectionObserver(
      (items) => {
        sentinelVisible.current = items.some((item) => item.isIntersecting);
        if (sentinelVisible.current) {
          loadMore();
        }
      },
      { root, rootMargin: `0px 0px ${LOAD_AHEAD_PX}px 0px` },
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [loadMore]);

  // A page that does not fill the viewport leaves the sentinel in view, and
  // the observer does not fire again; keep going until it scrolls away.
  useEffect(() => {
    if (sentinelVisible.current && hasMore && !loadingMore) {
      loadMore();
    }
  }, [list.entries, hasMore, loadingMore, loadMore]);

  // Moving the focus with j/k near the last loaded row reads ahead too.
  useEffect(() => {
    if (focus >= 0 && focus >= flat.length - 5 && hasMore) {
      loadMore();
    }
  }, [focus, flat.length, hasMore, loadMore]);

  function onListScroll() {
    const root = listRef.current;
    if (root && root.scrollTop + root.clientHeight >= root.scrollHeight - LOAD_AHEAD_PX) {
      if (hasMore) {
        loadMore();
      } else if (!loadingMore && remoteMore && root.scrollTop > 0) {
        remoteMore();
      }
    }
  }

  useEffect(() => {
    const element = listRef.current?.querySelector<HTMLElement>('[data-focused="true"]');
    element?.scrollIntoView?.({ block: "nearest" });
  }, [focus]);

  function setSelected(next: Set<number>) {
    setSelectedByView((current) => ({ ...current, [viewKey]: [...next] }));
    if (next.size === 0) {
      setAddMenuFor(null);
    }
  }

  /** The bulk bar's action: one PR opens the single preview, several open the bulk one (§7.9). */
  function requestSelection() {
    const ids = activeIds;
    if (singleSelected && ids.length === 1) {
      onRequestReview(singleSelected);
    } else if (ids.length > 0) {
      onBulkReview(ids);
    }
  }

  /** Copies the selected loaded rows, in list order, as a Markdown list. */
  function copySelectionAsMarkdown() {
    const chosen = new Set(activeIds);
    const rows = flat.filter((item) => chosen.has(item.row.id)).map((item) => item.row);
    if (rows.length === 0) {
      return;
    }
    copyText(prsToMarkdown(rows)).then(
      () => onNotice?.({ text: `Copied ${rows.length} ${rows.length === 1 ? "PR" : "PRs"} as Markdown` }),
      (reason: unknown) => onNotice?.({ text: `Couldn't copy to the clipboard: ${reason instanceof Error ? reason.message : String(reason)}`, error: true }),
    );
  }

  function toggleRow(id: number) {
    const next = new Set(selected);
    if (next.has(id)) {
      next.delete(id);
    } else {
      next.add(id);
    }
    setSelected(next);
  }

  /** Runs one registry action on the focused row or the selection; keys and the command palette share it. */
  function runAction(id: ShortcutId, event?: KeyboardEvent): boolean {
    const current = focus >= 0 ? flat[focus] : undefined;
    switch (id) {
      case "next":
      case "prev": {
        event?.preventDefault();
        if (flat.length === 0) {
          return true;
        }
        const next = Math.min(flat.length - 1, Math.max(0, focus + (id === "next" ? 1 : -1)));
        setFocusIndex(next);
        if (detailId !== null && flat[next].row.id !== detailId) {
          onDetails(flat[next].row);
        }
        return true;
      }
      case "search":
        event?.preventDefault();
        searchRef.current?.focus();
        return true;
      case "selectAll":
        event?.preventDefault();
        setSelected(toggleGlobal(ids, selected));
        return true;
      case "clear":
        if (addMenu) {
          setAddMenu(false);
        } else if (sectionMenu !== null) {
          setSectionMenu(null);
        } else if (selected.size > 0) {
          setSelected(new Set());
        }
        return true;
      case "toggleSection":
        if (current) {
          setSelected(toggleParent(current.section.ids, selected));
        }
        return true;
      case "toggleRow":
        if (current) {
          toggleRow(current.row.id);
        }
        return true;
      case "request":
        if (selected.size > 0) {
          requestSelection();
        } else if (current) {
          onRequestReview(current.row);
        }
        return true;
      case "slack": {
        const link = current?.row.slack?.permalink;
        if (link) {
          onOpen(link);
        }
        return true;
      }
      case "details":
        if (current) {
          event?.preventDefault();
          onDetails(current.row);
        }
        return true;
      case "github": {
        const url = current?.row.url;
        if (url) {
          onOpen(url);
        }
        return true;
      }
      case "filters":
        setMenu((open) => (open ? null : "root"));
        return true;
      case "project":
        // With nothing selected, P acts on the focused row.
        if (selected.size === 0 && current) {
          setSelected(new Set([current.row.id]));
        }
        if (selected.size > 0 || current) {
          event?.preventDefault();
          setAddMenu((open) => !open);
        }
        return true;
      case "copyMarkdown":
        if (selected.size > 0) {
          event?.preventDefault();
          copySelectionAsMarkdown();
        }
        return true;
      case "star":
        if (current && onToggleStar) {
          onToggleStar(current.row);
        }
        return true;
      case "palette":
      case "help":
        // Handled by the list screen.
        return false;
      default: {
        const never: never = id;
        return never;
      }
    }
  }

  useEffect(() => {
    if (actionsRef) {
      actionsRef.current = { run: (id) => runAction(id), focused: () => (focus >= 0 ? (flat[focus]?.row ?? null) : null), selectedCount: selected.size };
    }
  });

  useEffect(() => {
    if (!standupMenu) {
      return;
    }
    function onDown(event: MouseEvent) {
      if (!standupMenuRef.current?.contains(event.target as Node)) {
        setStandupMenu(false);
      }
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setStandupMenu(false);
      }
    }
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [standupMenu]);

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      const target = event.target instanceof HTMLElement ? event.target : null;
      const typing = isTypingTarget(target);
      if (document.querySelector('[role="dialog"][aria-modal="true"]')) {
        return;
      }
      const id = matchShortcut(event);
      if (id === "search" && !typing) {
        runAction(id, event);
        return;
      }
      if (event.key === "Escape" && target === searchRef.current) {
        setSearch("");
        return;
      }
      if (typing || id === null) {
        return;
      }
      if ((id === "next" || id === "prev") && event.key.startsWith("Arrow")) {
        // ↑/↓ also move while the detail panel is open, so it can step through PRs.
        // Inside the panel the arrows still scroll it.
        if (detailId === null || target?.closest("[data-detail-panel]")) {
          return;
        }
      }
      if (id === "details" && !wantsRowEnter(target)) {
        return;
      }
      runAction(id, event);
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const header = selectionCountText(ids.length, page?.summary.finished ?? 0, activeIds.length, hidden);
  const openLimitHint = page && viewKey === "mine" ? wipHint(page.summary.open, wipLimit) : null;
  const queryOrders = sortLocksOrder(searchFieldValue(view.filters));

  const titleProject = currentProject !== null ? projects.find((project) => project.id === currentProject) : undefined;
  const sectionProject = (id: number | null) => (id === null ? undefined : projects.find((project) => project.id === id));

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="px-4 py-3">
        <div className="min-w-0">
          {currentProject !== null && editing?.id === currentProject && editing.where === "title" ? (
            <RenameField
              label={`Rename ${viewTitle(viewKey, projects)}`}
              editing={editing}
              className="text-lg font-semibold"
              onChange={setEditing}
              onCommit={() => void commit(currentProject)}
              onCancel={() => setEditing(null)}
              committing={commitRename}
            />
          ) : (
            <Tooltip label={currentProject !== null ? "Double-click to rename" : null} align="start">
              <h1
                className="text-lg font-semibold"
                onDoubleClick={() => {
                  if (currentProject !== null && projects.some((project) => project.id === currentProject)) {
                    setEditing({ id: currentProject, where: "title", name: viewTitle(viewKey, projects), error: null });
                  }
                }}
              >
                {viewTitle(viewKey, projects)}
              </h1>
            </Tooltip>
          )}
          {editing?.error && editing.where === "title" && <p className="text-xs text-rose-300">{editing.error}</p>}
          {titleProject && <ProjectInfo project={titleProject} onOpen={onOpen} className="mt-0.5 max-w-md" />}
          <p className="text-sm text-slate-400">
            {page ? summaryText(page.summary, viewKey, syncProgress, view.filters.snoozed) : (syncProgress ?? "\u00a0")}
            {openLimitHint && <span className="text-amber-200/80"> · {openLimitHint}</span>}
          </p>
        </div>
      </header>
      {/* One toolbar row for every list command, quick filter chips included: controls share one height and never
          wrap. Narrow widths (container queries) shrink the search, drop text labels, then the plain PR count; chips
          that still do not fit fold into More (N). */}
      <div className="@container border-y border-slate-800 px-4 py-2 text-sm text-slate-300">
        <div role="toolbar" aria-label="List toolbar" className="flex h-8 min-w-0 flex-nowrap items-center gap-2">
          <TriCheckbox label="Select all" state={global} onToggle={() => setSelected(toggleGlobal(ids, selected))} />
          <span className={`shrink-0 whitespace-nowrap ${checkState(ids, selected) === "none" && hidden === 0 ? "hidden @4xl:inline" : ""}`}>{header}</span>
          <QuickFilters
            active={view.filters.quick}
            counts={page?.quick ?? null}
            onChange={(quick) => onViewChange({ ...view, filters: { ...view.filters, quick } })}
            starred={view.filters.starred}
            starredCount={page ? new Set(list.entries.flatMap((entry) => (entry.row?.starred ? [entry.row.id] : []))).size : null}
            onStarredChange={(starred) => onViewChange({ ...view, filters: { ...view.filters, starred } })}
            unread={view.filters.unread}
            unreadCount={page?.attention ?? null}
            onUnreadChange={(unread) => onViewChange({ ...view, filters: { ...view.filters, unread } })}
            snoozed={view.filters.snoozed}
            snoozedCount={page?.snoozed_count ?? null}
            onSnoozedChange={(snoozed) => onViewChange({ ...view, filters: { ...view.filters, snoozed } })}
          />
          <RepoPicker
            options={page?.repos ?? []}
            selected={view.filters.repos}
            onChange={(repos) => onViewChange({ ...view, filters: { ...view.filters, repos } })}
          />
          <SearchField value={field} onChange={setSearch} searching={searching || githubSearching} inputRef={searchRef} />
          <FilterMenu view={view} orgs={page?.orgs ?? []} menu={menu} setMenu={setMenu} onViewChange={onViewChange} fromQuery={queryOrders} />
          {viewKey === "mine" && (
            <div ref={standupMenuRef} className="relative shrink-0">
              <Tooltip label="More" detail="Copy a stand-up recap of your pull requests" align="end">
                <button
                  type="button"
                  aria-label="More"
                  aria-haspopup="menu"
                  aria-expanded={standupMenu}
                  className="flex h-8 w-8 items-center justify-center rounded-md text-slate-400 hover:bg-slate-800 hover:text-slate-200"
                  onClick={() => setStandupMenu((open) => !open)}
                >
                  <MoreIcon />
                </button>
              </Tooltip>
              {standupMenu && (
                <div role="menu" aria-label="More" className="absolute right-0 top-8 z-30 w-max min-w-48 rounded-md border border-slate-700 bg-slate-900 py-1 text-sm shadow-xl">
                  <button
                    type="button"
                    role="menuitem"
                    className="block w-full px-3 py-1.5 text-left text-slate-200 hover:bg-slate-800"
                    onClick={() => {
                      setStandupMenu(false);
                      onCopyStandup?.();
                    }}
                  >
                    Copy stand-up recap
                  </button>
                </div>
              )}
            </div>
          )}
        </div>
        {githubError !== "" && (
          <p role="status" data-github-query-error className="mt-1 truncate text-xs text-slate-400">
            {githubError}
          </p>
        )}
      </div>
      <div
        ref={listRef}
        className={`@container min-h-0 flex-1 overflow-auto px-3 pt-2 ${selected.size > 0 ? "pb-32" : "pb-20"}`}
        aria-label="Pull requests"
        aria-busy={list.pending || loadingMore}
        role="region"
        onScroll={onListScroll}
      >
        {!page && list.error && (
          <div role="alert" className="px-3 py-6 text-sm">
            <p className="font-medium text-rose-200">Could not load your pull requests.</p>
            <p className="mt-1 text-slate-400">{list.error}</p>
            <button type="button" className="mt-3 rounded border border-slate-700 px-3 py-1.5 hover:bg-slate-800" onClick={list.retry}>
              Try again
            </button>
          </div>
        )}
        {!page && !list.error && (
          <p role="status" className="px-3 py-6 text-sm text-slate-400">
            Loading pull requests…
          </p>
        )}
        {showSections && sections.map((section) => {
          const open = !collapsed[section.key];
          const state = checkState(section.ids, selected);
          return (
            <section
              key={section.key}
              aria-label={section.title}
              className={`mb-2 rounded-lg border ${dropTarget === section.key ? "border-violet-400 bg-violet-500/5" : "border-slate-800"}`}
              onDragOver={(event) => {
                if (section.projectId !== null && event.dataTransfer.types.includes(DRAG_MIME)) {
                  event.preventDefault();
                  event.dataTransfer.dropEffect = "link";
                  setDropTarget(section.key);
                }
              }}
              onDragLeave={(event) => {
                if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
                  setDropTarget(null);
                }
              }}
              onDrop={(event) => {
                setDropTarget(null);
                const ids = parseDragIds(event.dataTransfer.getData(DRAG_MIME));
                if (section.projectId !== null && ids.length > 0) {
                  event.preventDefault();
                  onLinkPrs(section.projectId, ids);
                }
              }}
            >
              <header className="flex items-center gap-2 px-2 py-1.5">
                <Tooltip label={open ? "Collapse" : "Expand"} detail={open ? "Hide this group's PRs" : "Show this group's PRs"} align="start">
                  <button
                    type="button"
                    aria-expanded={open}
                    aria-label={`${open ? "Collapse" : "Expand"} ${section.title}`}
                    className="flex h-5 w-5 items-center justify-center rounded text-slate-400 hover:bg-slate-800 hover:text-slate-200"
                    onClick={() => setCollapsed((current) => ({ ...current, [section.key]: open }))}
                  >
                    {open ? <ChevronDownIcon /> : <ChevronRightIcon />}
                  </button>
                </Tooltip>
                <TriCheckbox
                  label={`Select ${section.title}`}
                  state={state}
                  onToggle={() => setSelected(toggleParent(section.ids, selected))}
                />
                {section.projectId !== null && editing?.id === section.projectId && editing.where === "section" ? (
                  <RenameField
                    label={`Rename ${section.title}`}
                    editing={editing}
                    className="text-sm"
                    onChange={setEditing}
                    onCommit={() => void commit(section.projectId!)}
                    onCancel={() => setEditing(null)}
                    committing={commitRename}
                  />
                ) : (
                  <Tooltip label={section.projectId !== null ? "Double-click to rename" : null} align="start">
                    <span
                      className="text-sm font-medium"
                      onDoubleClick={() => {
                        if (section.projectId !== null) {
                          setEditing({ id: section.projectId, where: "section", name: section.title, error: null });
                        }
                      }}
                    >
                      {section.title}
                    </span>
                  </Tooltip>
                )}
                <span className="text-xs text-slate-500">({section.count})</span>
                {sectionProject(section.projectId) && <ProjectInfo project={sectionProject(section.projectId)!} onOpen={onOpen} className="max-w-lg" />}
                {editing?.error && editing.where === "section" && editing.id === section.projectId && (
                  <span className="text-xs text-rose-300">{editing.error}</span>
                )}
                <span className="flex-1" />
                {section.projectId !== null && (
                  <div className="relative">
                    <Tooltip label="Project actions" detail="Edit auto-link rules, archive or delete" align="end">
                      <button
                        type="button"
                        aria-label={`Actions for ${section.title}`}
                        aria-haspopup="menu"
                        aria-expanded={sectionMenu === section.key}
                        className="flex h-6 w-6 items-center justify-center rounded text-slate-400 hover:bg-slate-800 hover:text-slate-200"
                        onClick={() => setSectionMenu((current) => (current === section.key ? null : section.key))}
                      >
                        <MoreIcon />
                      </button>
                    </Tooltip>
                    {sectionMenu === section.key && (
                      <div role="menu" className="absolute right-0 z-10 w-48 rounded border border-slate-700 bg-slate-900 py-1 text-sm">
                        <button type="button" className="block w-full px-3 py-1.5 text-left" onClick={() => { onEditProject(section.projectId!); setSectionMenu(null); }}>
                          Edit auto-link rules
                        </button>
                        <button type="button" className="block w-full px-3 py-1.5 text-left" onClick={() => { onArchive(section.projectId!); setSectionMenu(null); }}>
                          Archive project
                        </button>
                        <button type="button" className="block w-full px-3 py-1.5 text-left" onClick={() => { setDeleteFor(section); setSectionMenu(null); }}>
                          Delete project
                        </button>
                      </div>
                    )}
                  </div>
                )}
                {/* Same height as a project header, whose ⋯ button sets it. */}
                {section.projectId === null && <span aria-hidden="true" className="h-6 w-6" />}
              </header>
              {open && section.count === 0 && (
                <p className="border-t border-slate-800 px-3 py-3 text-xs text-slate-500">
                  No PRs here yet. Drag PRs onto this project, use Add to project (p), or add an auto-link rule from the actions menu.
                </p>
              )}
              {open &&
                section.rows.map((row) => {
                  const index = flat.findIndex((item) => item.row.id === row.id && item.section.key === section.key);
                  return (
                    <article
                      key={`${section.key}-${row.id}`}
                      draggable
                      onDragStart={(event) => {
                        const ids = dragIds(row.id, active);
                        event.dataTransfer.setData(DRAG_MIME, JSON.stringify(ids));
                        event.dataTransfer.setData("text/plain", ids.length === 1 ? row.url : `${ids.length} pull requests`);
                        event.dataTransfer.effectAllowed = "link";
                        setDragBadge(event.dataTransfer, ids.length);
                      }}
                      onDragEnd={() => setDropTarget(null)}
                      onMouseDown={() => setFocusIndex(index)}
                      onClick={(event) => {
                        // The whole row opens (or switches) the panel; its own controls keep their behaviour.
                        if (rowClickOpens(event.target, event.currentTarget) && detailId !== row.id) {
                          onDetails(row);
                        }
                      }}
                      aria-label={`#${row.number} ${row.title}`}
                      aria-current={index === focus ? "true" : undefined}
                      data-focused={index === focus ? "true" : undefined}
                      className={`group flex ${compact ? "items-start gap-2 py-1" : "items-center gap-3 py-1.5"} cursor-pointer border-t border-l-2 border-t-slate-800 px-2 ${index === focus ? "border-l-violet-400 bg-slate-900" : "border-l-transparent hover:bg-slate-900/60"}`}
                    >
                      <TriCheckbox
                        label={`Select ${row.title}`}
                        state={selected.has(row.id) ? "all" : "none"}
                        onToggle={() => toggleRow(row.id)}
                      />
                      {onToggleStar && <StarButton starred={row.starred} title={row.title} onToggle={() => onToggleStar(row)} />}
                      <GitHubRowButton title={row.title} onOpen={() => onOpen(row.url)} />
                      {compact ? (
                        <CompactRow row={row} view={view} viewKey={viewKey} now={now} sla={sla} onDetails={onDetails} onOpen={onOpen} onRequestReview={onRequestReview} />
                      ) : (
                        <>
                          <div className="min-w-64 flex-1 overflow-hidden">
                            <TitleLine row={row} view={view} now={now} sla={sla} onDetails={onDetails} />
                            <p className="flex items-center gap-1 truncate text-xs text-slate-500">
                              {view.show.repo && <span className="truncate">{row.repo}</span>}
                              {view.show.repo && view.show.ci && <span aria-hidden="true">·</span>}
                              {view.show.ci && <CiStatus checks={row.checks} />}
                              {view.show.activity && <span className="whitespace-nowrap"> · updated {ago(row.updated_at, now)}</span>}
                            </p>
                          </div>
                          {view.show.review && (
                            <div className="flex w-40 shrink-0 items-center">
                              <NextActionBadge row={row} viewKey={viewKey} now={now} sla={sla} onOpen={onOpen} onRequestReview={onRequestReview} onDetails={onDetails} />
                            </div>
                          )}
                          {view.show.review && (
                            <div className="flex w-20 shrink-0 items-center justify-end">
                              <ReviewerAvatars reviewers={row.pending_reviewers} />
                            </div>
                          )}
                          {/* Lowest priority first: the comment count, then the Slack text, give way on narrow lists. */}
                          {view.show.threads && (
                            <div className="hidden w-14 shrink-0 @min-[64rem]:flex">
                              <Threads row={row} />
                            </div>
                          )}
                          <div className="hidden w-20 shrink-0 items-center justify-end @min-[56rem]:flex">
                            <SlackCell row={row} now={now} showMissing={view.show.notRequested} onOpen={onOpen} />
                          </div>
                        </>
                      )}
                    </article>
                  );
                })}
            </section>
          );
        })}
        {(hasMore || loadingMore) && (
          <p role="status" className="px-3 py-3 text-center text-xs text-slate-500">
            Loading more…
          </p>
        )}
        {!hasMore && !loadingMore && remoteMore && (
          <div className="px-3 py-3 text-center">
            <button type="button" className="rounded border border-slate-700 px-3 py-1 text-xs text-slate-300 hover:bg-slate-800" onClick={remoteMore}>
              Load more review requests
            </button>
          </div>
        )}
        {!hasMore && !loadingMore && githubMore && (
          <div className="px-3 py-3 text-center">
            <button type="button" className="rounded border border-slate-700 px-3 py-1 text-xs text-slate-300 hover:bg-slate-800" onClick={githubMore}>
              Load more matching pull requests
            </button>
          </div>
        )}
        <div ref={sentinelRef} data-testid="list-sentinel" aria-hidden="true" className="h-px" />
        {page && !list.pending && !showSections && (
          <ViewEmptyState kind={viewEmpty.kind} onOpenSettings={onOpenSettings} />
        )}
        {githubSearching && sections.length === 0 && (
          <p role="status" className="px-3 py-6 text-sm text-slate-400">
            Searching GitHub…
          </p>
        )}
        {page && !list.pending && showSections && sections.length === 0 && !githubSearching && githubError === "" && (
          <EmptyList
            title={viewTitle(viewKey, projects)}
            inView={inView}
            query={localQuery}
            filtered={filtersActive(view.filters)}
            starredOnly={view.filters.starred}
            snoozedOnly={view.filters.snoozed}
            snoozedCount={page.snoozed_count}
            elsewhere={page.elsewhere}
            viewTitle={(key) => viewTitle(key, projects)}
            onClearSearch={() => setSearch("")}
            onResetFilters={() => onViewChange(resetFilters(view))}
            onShowView={onShowView}
            onShowFinished={() => onViewChange({ ...view, filters: { ...view.filters, archived: true } })}
            onShowSnoozed={() => onViewChange({ ...view, filters: { ...view.filters, snoozed: true } })}
          />
        )}
      </div>
      {selected.size > 0 && (
        <SelectionBar
          count={activeIds.length}
          hidden={hidden}
          addMenuOpen={addMenu}
          onToggleAddMenu={() => setAddMenu((open) => !open)}
          addMenu={
            <AddToProjectMenu
              projects={projects}
              count={activeIds.length}
              onClose={() => setAddMenu(false)}
              onPick={(projectId) => {
                setAddMenu(false);
                onLinkPrs(projectId, activeIds);
              }}
              onCreate={async (name) => {
                await onCreateProjectWithPrs(name, activeIds);
                setAddMenu(false);
              }}
            />
          }
          onRequest={requestSelection}
          onCopy={copySelectionAsMarkdown}
          onClose={() => {
            const byId = new Map(flat.map((item) => [item.row.id, item.row]));
            onClosePullRequests?.(
              activeIds.map((id) => {
                const row = byId.get(id);
                return row
                  ? { id: row.id, repo: row.repo, number: row.number, title: row.title }
                  : { id, repo: "", number: 0, title: "" };
              }),
            );
          }}
          onEnqueue={
            onEnqueuePullRequests &&
            activeIds.some((id) => flat.some((item) => item.row.id === id && item.row.merge_queue?.required))
              ? () => {
                  const byId = new Map(flat.map((item) => [item.row.id, item.row]));
                  onEnqueuePullRequests(activeIds.flatMap((id) => {
                    const row = byId.get(id);
                    return row ? [row] : [];
                  }));
                }
              : undefined
          }
          onClear={() => setSelected(new Set())}
          onMarkRead={() => onMarkRead?.(activeIds)}
          onMarkAllRead={() => onMarkAllRead?.(page ? pageIds(page.sections) : [])}
          onSnooze={(choice) => onSnooze?.(activeIds, choice)}
          onMute={(muted) => onMute?.(activeIds, muted)}
        />
      )}
      {deleteFor?.projectId !== null && deleteFor && (
        <dialog open className="rounded border border-slate-700 bg-slate-900 p-4 text-sm">
          <p>Delete {deleteFor.title}? This cannot be undone.</p>
          <div className="mt-3 flex gap-2">
            <button
              type="button"
              className="rounded bg-rose-500 px-3 py-1"
              onClick={() => {
                if (deleteFor.projectId !== null) {
                  onDelete(deleteFor.projectId);
                }
                setDeleteFor(null);
              }}
            >
              Delete
            </button>
            <button type="button" onClick={() => setDeleteFor(null)}>
              Cancel
            </button>
          </div>
        </dialog>
      )}
    </div>
  );

  async function commit(projectId: number) {
    if (!editing) {
      return;
    }
    const error = renameError(editing.name, projects, projectId);
    if (error) {
      setEditing({ ...editing, error });
      return;
    }
    commitRename.current = true;
    try {
      await onRename(projectId, editing.name.trim());
      setEditing(null);
    } catch (err) {
      setEditing({ ...editing, error: err instanceof Error ? err.message : "Could not rename" });
    } finally {
      commitRename.current = false;
    }
  }
}

/** Why the list is empty, and the way out (§8.2). */
function EmptyList({
  title,
  inView,
  query,
  filtered,
  starredOnly,
  snoozedOnly,
  snoozedCount,
  elsewhere,
  viewTitle,
  onClearSearch,
  onResetFilters,
  onShowView,
  onShowFinished,
  onShowSnoozed,
}: {
  title: string;
  inView: number;
  query: string;
  filtered: boolean;
  starredOnly: boolean;
  snoozedOnly: boolean;
  snoozedCount: number;
  elsewhere: Elsewhere | null;
  viewTitle: (viewKey: string) => string;
  onClearSearch: () => void;
  onResetFilters: () => void;
  onShowView?: (viewKey: string) => void;
  onShowFinished: () => void;
  onShowSnoozed: () => void;
}) {
  const searching = query.trim() !== "";
  if (inView === 0 && !searching && snoozedCount > 0 && !snoozedOnly) {
    return (
      <div className="px-2 py-10 text-center text-sm text-slate-400">
        <p className="font-medium text-slate-300">All pull requests in {title} are snoozed.</p>
        <div className="mt-3 flex justify-center">
          <button type="button" className="rounded border border-slate-700 px-3 py-1 hover:bg-slate-800" onClick={onShowSnoozed}>
            Show snoozed
          </button>
        </div>
      </div>
    );
  }
  if (snoozedOnly && inView === 0 && !searching) {
    return (
      <div className="px-2 py-10 text-center text-sm text-slate-400">
        <p className="font-medium text-slate-300">No snoozed pull requests in {title}.</p>
      </div>
    );
  }
  if (inView === 0 && !searching) {
    return (
      <div className="px-2 py-10 text-center text-sm text-slate-400">
        <p className="font-medium text-slate-300">No open pull requests in {title}.</p>
        <p className="mt-1 text-xs text-slate-500">New ones appear after the next sync. Refresh from the sidebar to sync now.</p>
      </div>
    );
  }
  if (starredOnly && !searching) {
    return (
      <div className="px-2 py-10 text-center text-sm text-slate-400">
        <p className="font-medium text-slate-300">No starred pull requests in {title}{filtered ? " match your filters" : ""}.</p>
        <p className="mt-1 text-xs text-slate-500">
          Star a PR with the ☆ on its row or {displayKey(shortcut("star").keys[0])}; starred PRs stay at the top of the list.
        </p>
        <div className="mt-3 flex justify-center">
          <button type="button" className="rounded border border-slate-700 px-3 py-1 hover:bg-slate-800" onClick={onResetFilters}>
            Reset filters
          </button>
        </div>
      </div>
    );
  }
  const views = onShowView ? (elsewhere?.views ?? []).filter((item) => item.count > 0) : [];
  const finished = elsewhere?.finished ?? 0;
  const hiddenByFilters = elsewhere?.filtered ?? 0;
  const button = "rounded border border-slate-700 px-3 py-1 hover:bg-slate-800";
  return (
    <div className="px-2 py-10 text-center text-sm text-slate-400">
      <p className="font-medium text-slate-300">
        No pull requests in {title} match {searching && filtered ? "your search and filters" : searching ? "your search" : "your filters"}.
      </p>
      {(views.length > 0 || finished > 0 || hiddenByFilters > 0) && (
        <p className="mt-1 text-xs text-slate-500">{elsewhereText(views, finished, hiddenByFilters, viewTitle)}</p>
      )}
      <div className="mt-3 flex flex-wrap justify-center gap-2">
        {views.map((item) => (
          <button key={item.view_key} type="button" className={button} onClick={() => onShowView?.(item.view_key)}>
            Show in {viewTitle(item.view_key)} ({item.count})
          </button>
        ))}
        {finished > 0 && (
          <button type="button" className={button} onClick={onShowFinished}>
            Include archived and closed ({finished})
          </button>
        )}
        {searching && (
          <button type="button" className={button} onClick={onClearSearch}>
            Clear search
          </button>
        )}
        {filtered && (
          <button type="button" className={button} onClick={onResetFilters}>
            Reset filters{hiddenByFilters > 0 ? ` (${hiddenByFilters})` : ""}
          </button>
        )}
      </div>
    </div>
  );
}

function elsewhereText(views: readonly { view_key: string; count: number }[], finished: number, filtered: number, viewTitle: (viewKey: string) => string): string {
  const parts = [
    ...views.map((item) => `${item.count} in ${viewTitle(item.view_key)}`),
    ...(finished > 0 ? [`${finished} merged, closed or archived`] : []),
    ...(filtered > 0 ? [`${filtered} hidden by filters`] : []),
  ];
  return `Matches elsewhere: ${parts.join(", ")}.`;
}

interface Editing {
  id: number;
  where: "section" | "title";
  name: string;
  error: string | null;
}

/** Inline rename (§8.2): Enter saves, Esc or blur cancels. */
function RenameField({
  label,
  editing,
  className,
  onChange,
  onCommit,
  onCancel,
  committing,
}: {
  label: string;
  editing: Editing;
  className: string;
  onChange: (editing: Editing) => void;
  onCommit: () => void;
  onCancel: () => void;
  committing: { current: boolean };
}) {
  return (
    <input {...plainText}
      aria-label={label}
      className={`rounded border border-slate-600 bg-slate-900 px-2 py-0.5 ${className}`}
      value={editing.name}
      autoFocus
      onChange={(event) => onChange({ ...editing, name: event.target.value, error: null })}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.stopPropagation();
          onCancel();
        }
        if (event.key === "Enter") {
          event.preventDefault();
          onCommit();
        }
      }}
      onBlur={() => {
        if (!committing.current) {
          onCancel();
        }
      }}
    />
  );
}

/**
 * The Slack cell of a list row: the coloured logo plus "6h ago" once a request
 * was sent (a link to the message when there is one), a grey logo otherwise.
 * Workspace, channel and action live in the tooltip and the aria-label.
 */
function SlackCell({ row, now, showMissing, onOpen }: { row: ListRow; now: Date; showMissing: boolean; onOpen: (url: string) => void }) {
  const slack = compactSlack(row.slack, now);
  if (!slack.sent && !showMissing) {
    return null;
  }
  const permalink = row.slack?.permalink;
  const content = (
    <>
      <SlackLogo variant={slack.sent ? "color" : "mono"} size={14} />
      {slack.time}
    </>
  );
  const look = "flex items-center gap-1 whitespace-nowrap text-xs tabular-nums";
  if (slack.sent && permalink) {
    return (
      <Tooltip label={slack.title} align="end">
        <button
          type="button"
          aria-label={`Open Slack request for ${row.title}: ${slack.title}`}
          className={`${look} rounded text-slate-300 hover:text-slate-100`}
          onClick={(event) => {
            event.stopPropagation();
            onOpen(permalink);
          }}
        >
          {content}
        </button>
      </Tooltip>
    );
  }
  return (
    <Tooltip label={slack.title} align="end">
      <span aria-label={slack.title} role="img" className={`${look} ${slack.sent ? "text-slate-300" : "text-slate-600"}`}>
        {content}
      </span>
    </Tooltip>
  );
}

const TONE_TEXT: Record<Tone, string> = {
  good: "text-emerald-400",
  bad: "text-rose-400",
  warn: "text-amber-400",
  pending: "text-violet-400",
  muted: "text-slate-500",
};

function Dot({ tone }: { tone: Tone }) {
  return <span aria-hidden="true" className={`inline-block h-1.5 w-1.5 rounded-full bg-current ${TONE_TEXT[tone]}`} />;
}

function Threads({ row }: { row: ListRow }) {
  const text = commentsText(row.unresolved_threads, row.total_threads);
  if (text === null) {
    return null;
  }
  const { title } = threadsLabel(row.unresolved_threads, row.total_threads);
  return (
    <Tooltip label={title} className="shrink-0">
      <span
        className={`flex shrink-0 items-center gap-1 text-xs tabular-nums ${row.unresolved_threads > 0 ? "text-slate-300" : "text-slate-500"}`}
        aria-label={title}
      >
        <CommentIcon size={14} />
        {text}
      </span>
    </Tooltip>
  );
}

const PILL_SPECS: Record<RowPill, { label: string; title: string; className: string; icon: ReactElement }> = {
  draft: { label: "Draft", title: "Draft PR: not ready for review", className: "border-slate-600 text-slate-300", icon: <DraftIcon size={12} /> },
  conflicts: { label: "Conflicts", title: "GitHub reports merge conflicts", className: "border-amber-500/50 text-amber-300", icon: <AlertIcon size={12} /> },
};

function RowPillTag({ pill }: { pill: RowPill }) {
  const spec = PILL_SPECS[pill];
  return (
    <Tooltip label={spec.title} className="shrink-0">
      <span className={`inline-flex shrink-0 items-center gap-1 rounded-full border px-1.5 text-[11px] leading-4 ${spec.className}`}>
        {spec.icon}
        {spec.label}
      </span>
    </Tooltip>
  );
}

function StaleChip({ cue }: { cue: StaleCue }) {
  return (
    <Tooltip label={cue.title} className="shrink-0">
      <span
        className={`inline-flex shrink-0 items-center gap-1 rounded-full px-1.5 text-[11px] leading-4 ${cue.kind === "waiting" ? "bg-amber-500/15 text-amber-300" : "bg-slate-800 text-slate-400"}`}
      >
        <ClockIcon size={12} />
        {cue.label}
      </span>
    </Tooltip>
  );
}

/** Row cues after the title: draft, conflict, queue, and bot pills, then the age chip. */
function RowCues({ row, now, sla }: { row: ListRow; now: Date; sla: ReviewSla }) {
  const stale = staleCue(row, now, sla);
  const bots = row.bot_comments ?? 0;
  const botLabel = bots === 1 ? "1 bot comment" : `${bots} bot comments`;
  const queue = queueChip(row.merge_queue);
  return (
    <>
      {rowPills(row).map((pill) => (
        <RowPillTag key={pill} pill={pill} />
      ))}
      {bots > 0 && (
        <Tooltip label={botLabel} className="shrink-0">
          <span aria-label={botLabel} className="inline-flex shrink-0 items-center rounded-full bg-slate-800 px-1.5 text-[11px] leading-4 text-slate-400">
            bot
          </span>
        </Tooltip>
      )}
      {queue && <QueuePill chip={queue} />}
      {stale && <StaleChip cue={stale} />}
    </>
  );
}

const QUEUE_PILL: Record<"queue" | "merging" | "failed" | "removed", string> = {
  queue: "bg-teal-500/15 text-teal-200",
  merging: "bg-teal-500/15 text-teal-200",
  failed: "bg-amber-500/15 text-amber-200",
  removed: "bg-slate-800 text-slate-300",
};

function QueuePill({ chip }: { chip: { label: string; detail: string | null; tone: keyof typeof QUEUE_PILL } }) {
  const pill = (
    <span className={`inline-flex shrink-0 items-center rounded-full px-1.5 text-[11px] leading-4 ${QUEUE_PILL[chip.tone]}`}>
      {chip.label}
    </span>
  );
  if (!chip.detail) {
    return pill;
  }
  return (
    <Tooltip label={chip.detail} className="shrink-0">
      {pill}
    </Tooltip>
  );
}

function UnreadDot({ unread, className = "shrink-0" }: { unread: boolean; className?: string }) {
  if (!unread) {
    return null;
  }
  return (
    <Tooltip label="Unread" detail="New commits, comments, reviews, or CI since you last opened this pull request" className={className}>
      <span aria-label="Unread" className="inline-block h-1.5 w-1.5 rounded-full bg-violet-400" />
    </Tooltip>
  );
}

function TitleLine({ row, view, now, sla, onDetails }: { row: ListRow; view: SavedView; now: Date; sla: ReviewSla; onDetails: (row: ListRow) => void }) {
  return (
    <div className="flex min-w-0 items-center gap-2 text-sm">
      <UnreadDot unread={row.unread} />
      {view.show.numberTitle && (
        <Tooltip label="Readiness and code owners" shortcut="Enter" align="start" className="min-w-0">
          <button type="button" className="min-w-0 truncate text-left hover:underline" onClick={() => onDetails(row)}>
            <span className="text-slate-500">#{row.number}</span> {row.title}
          </button>
        </Tooltip>
      )}
      {row.state !== "open" && (
        <span className="shrink-0 rounded bg-slate-800 px-1 text-xs text-slate-300">{row.state === "merged" ? "Merged" : "Closed"}</span>
      )}
      <RowCues row={row} now={now} sla={sla} />
      {view.show.badges && row.project_ids.length > 1 && (
        <Tooltip label={`Also in ${row.project_ids.length - 1} other project${row.project_ids.length > 2 ? "s" : ""}`} className="shrink-0">
          <span className="shrink-0 rounded bg-slate-800 px-1 text-xs text-slate-400">+{row.project_ids.length - 1}</span>
        </Tooltip>
      )}
      {row.stack && <StackBadge stack={row.stack} />}
    </div>
  );
}

/** Compact place in a stack, for example 2/4. Hidden when the pull request is not stacked. */
function StackBadge({ stack }: { stack: StackPlace }) {
  const detail = stack.blocking ? "Merge this one first" : `Merge #${stack.root_number} first`;
  return (
    <Tooltip label={`Stack ${stack.position} of ${stack.size}`} detail={detail} className="shrink-0">
      <span className="inline-flex items-center gap-0.5 text-xs text-slate-400" aria-label={`Stack ${stack.position} of ${stack.size}. ${detail}`}>
        <StackIcon size={12} />
        {stack.position}/{stack.size}
      </span>
    </Tooltip>
  );
}

function CiStatus({ checks }: { checks: string }) {
  return (
    <span className="flex shrink-0 items-center gap-1">
      <Dot tone={ciTone(checks)} />
      {ciLabel(checks)}
    </span>
  );
}

/**
 * Two-line row for a narrow list (detail panel open, small window): the title
 * gets the full first line, everything else shares the second.
 */
/**
 * The row while the detail panel is open or the list is narrow: the title wraps
 * over two lines, repo and CI under it, and review, comments and Slack shrink
 * to icons with tooltips. Avatars are left to the detail panel.
 */
function CompactRow({
  row,
  view,
  viewKey,
  now,
  sla,
  onDetails,
  onOpen,
  onRequestReview,
}: {
  row: ListRow;
  view: SavedView;
  viewKey: string;
  now: Date;
  sla: ReviewSla;
  onDetails: (row: ListRow) => void;
  onOpen: (url: string) => void;
  onRequestReview: (row: ListRow) => void;
}) {
  const fullTitle = `#${row.number} ${row.title}`;
  return (
    <div className="flex min-w-0 flex-1 items-start gap-2">
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-start gap-1.5 text-sm leading-snug">
          <UnreadDot unread={row.unread} className="mt-1.5 shrink-0" />
          {view.show.numberTitle && (
            <Tooltip label={fullTitle} align="start" className="min-w-0">
              <button type="button" className="line-clamp-2 min-w-0 text-left break-words hover:underline" onClick={() => onDetails(row)}>
                <span className="text-slate-500">#{row.number}</span> {row.title}
              </button>
            </Tooltip>
          )}
          {row.state !== "open" && (
            <span className="shrink-0 rounded bg-slate-800 px-1 text-xs text-slate-300">{row.state === "merged" ? "Merged" : "Closed"}</span>
          )}
        </div>
        <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs text-slate-500">
          {view.show.repo && (
            <Tooltip label={row.repo} align="start" className="min-w-0 max-w-[16rem]">
              <span className="truncate">{row.repo}</span>
            </Tooltip>
          )}
          {view.show.ci && (
            <>
              <span aria-hidden="true">·</span>
              <CiStatus checks={row.checks} />
            </>
          )}
          {row.stack && <StackBadge stack={row.stack} />}
          <RowCues row={row} now={now} sla={sla} />
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-1.5 pt-0.5 text-xs text-slate-400">
        {view.show.review && <NextActionBadge row={row} viewKey={viewKey} now={now} sla={sla} compact onOpen={onOpen} onRequestReview={onRequestReview} onDetails={onDetails} />}
        {view.show.threads && <Threads row={row} />}
        <SlackCell row={row} now={now} showMissing={view.show.notRequested} onOpen={onOpen} />
      </div>
    </div>
  );
}

/** One message for a view that has no PRs at all, instead of empty project groups. */
function ViewEmptyState({ kind, onOpenSettings }: { kind: "no_reviews" | "no_prs"; onOpenSettings?: () => void }) {
  return (
    <div className="px-2 py-16 text-center text-sm text-slate-400">
      <p className="font-medium text-slate-300">{kind === "no_reviews" ? "No reviews requested from you" : "No open PRs in your tracked orgs"}</p>
      <p className="mt-1 text-xs text-slate-500">
        {kind === "no_reviews"
          ? "PRs where someone asks for your review show up here after the next sync."
          : "New ones appear after the next sync. Check which orgs are tracked if you expected some here."}
      </p>
      {kind === "no_prs" && onOpenSettings && (
        <button type="button" className="mt-3 rounded border border-slate-700 px-3 py-1 text-slate-200 hover:bg-slate-800" onClick={onOpenSettings}>
          Open Settings
        </button>
      )}
    </div>
  );
}

function TriCheckbox({ label, state, onToggle }: { label: string; state: CheckState; onToggle: () => void }) {
  return (
    <input
      type="checkbox"
      className="h-4 w-4 shrink-0 accent-violet-500"
      aria-label={label}
      aria-checked={state === "all" ? "true" : state === "partial" ? "mixed" : "false"}
      checked={state === "all"}
      ref={(element) => {
        if (element) {
          element.indeterminate = state === "partial";
        }
      }}
      onChange={onToggle}
    />
  );
}

function FilterMenu({
  view,
  orgs,
  menu,
  setMenu,
  onViewChange,
  fromQuery,
}: {
  view: SavedView;
  orgs: OrgCount[];
  menu: string | null;
  setMenu: (menu: string | null) => void;
  onViewChange: (view: SavedView) => void;
  fromQuery: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const open = menu !== null;
  useEffect(() => {
    if (!open) {
      return;
    }
    function onPointer(event: MouseEvent) {
      if (ref.current && event.target instanceof Node && !ref.current.contains(event.target)) {
        setMenu(null);
      }
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.stopPropagation();
        setMenu(null);
      }
    }
    document.addEventListener("mousedown", onPointer);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onPointer);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, setMenu]);
  return (
    <div ref={ref} className="relative shrink-0">
      <Tooltip label="View options" shortcut={hint("filters")} detail="Grouping, sort, visible columns and filters" align="end">
        <button
          type="button"
          aria-label="Filter menu"
          aria-haspopup="menu"
          aria-expanded={open}
          className={`relative flex h-8 w-8 items-center justify-center rounded-md border text-slate-300 hover:text-white ${open ? "border-violet-400 bg-slate-800" : "border-slate-700 bg-slate-900"}`}
          onClick={() => setMenu(menu ? null : "root")}
        >
          <FunnelIcon />
          {filtersActive(view.filters) && <span aria-label="Filters active" className="absolute -right-1 -top-1 h-2 w-2 rounded-full bg-violet-400" />}
        </button>
      </Tooltip>
      {menu && (
        <div role="menu" className="absolute right-0 z-20 mt-1 w-64 rounded border border-slate-700 bg-slate-900 p-2 text-sm">
          <MenuRow label="Grouping" value={labelOf(GROUPINGS, view.grouping)} onClick={() => setMenu(menu === "grouping" ? "root" : "grouping")} />
          {menu === "grouping" && (
            <ChoiceList
              options={GROUPINGS}
              current={view.grouping}
              onPick={(grouping) => onViewChange({ ...view, grouping })}
            />
          )}
          <MenuRow
            label="Sort"
            value={fromQuery ? "From query" : labelOf(ORDERINGS, view.ordering)}
            onClick={() => setMenu(menu === "ordering" ? "root" : "ordering")}
          />
          {menu === "ordering" && fromQuery && (
            <p className="px-2 py-1 text-xs text-slate-400">sort: in the search sets this order. Remove it to choose another.</p>
          )}
          {menu === "ordering" && !fromQuery && (
            <div>
              <ChoiceList options={ORDERINGS} current={view.ordering} onPick={(ordering) => onViewChange({ ...view, ordering })} />
              <div className="mt-1 flex gap-2 px-2">
                {(["asc", "desc"] as Direction[]).map((direction) => (
                  <button key={direction} type="button" className={view.direction === direction ? "text-violet-300" : ""} onClick={() => onViewChange({ ...view, direction })}>
                    {direction === "asc" ? "Ascending" : "Descending"}
                  </button>
                ))}
              </div>
            </div>
          )}
          <MenuRow label="Show" value="" onClick={() => setMenu(menu === "show" ? "root" : "show")} />
          {menu === "show" && <ShowList view={view} onViewChange={onViewChange} />}
          <div className="mt-2 flex items-center justify-between border-t border-slate-800 pt-2">
            <span>Filters</span>
            <button type="button" onClick={() => onViewChange(resetFilters(view))}>
              Reset
            </button>
          </div>
          <FilterChoices view={view} orgs={orgs} menu={menu} setMenu={setMenu} onViewChange={onViewChange} />
        </div>
      )}
    </div>
  );
}

function MenuRow({ label, value, onClick }: { label: string; value: string; onClick: () => void }) {
  return (
    <button type="button" className="flex w-full items-center justify-between px-1 py-1" onClick={onClick}>
      <span>{label}</span>
      <span className="text-slate-400">{value} ›</span>
    </button>
  );
}

function ChoiceList<T extends string>({
  options,
  current,
  onPick,
}: {
  options: { id: T; label: string }[];
  current: T;
  onPick: (id: T) => void;
}) {
  return (
    <div className="ml-2 border-l border-slate-800 pl-2">
      {options.map((option) => (
        <button key={option.id} type="button" className="block w-full px-1 py-0.5 text-left" onClick={() => onPick(option.id)}>
          {option.id === current ? "✓ " : ""}
          {option.label}
        </button>
      ))}
    </div>
  );
}

function ShowList({ view, onViewChange }: { view: SavedView; onViewChange: (view: SavedView) => void }) {
  const items: { key: keyof ShowFlags; label: string }[] = [
    { key: "numberTitle", label: "Number and title" },
    { key: "repo", label: "Repository" },
    { key: "ci", label: "CI" },
    { key: "review", label: "Review decision" },
    { key: "threads", label: "Threads" },
    { key: "teams", label: "Owning teams" },
    { key: "activity", label: "Last activity" },
    { key: "badges", label: "Project badges" },
    { key: "notRequested", label: "Not requested label" },
  ];
  return (
    <div className="ml-2 border-l border-slate-800 pl-2">
      {items.map((item) => (
        <label key={item.key} className="flex items-center gap-2 py-0.5">
          <input
            type="checkbox"
            checked={view.show[item.key]}
            onChange={() => onViewChange({ ...view, show: { ...view.show, [item.key]: !view.show[item.key] } })}
          />
          {item.label}
        </label>
      ))}
    </div>
  );
}

function FilterChoices({
  view,
  orgs,
  menu,
  setMenu,
  onViewChange,
}: {
  view: SavedView;
  orgs: OrgCount[];
  menu: string;
  setMenu: (menu: string | null) => void;
  onViewChange: (view: SavedView) => void;
}) {
  return (
    <div>
      <MenuRow label="Org" value={String(view.filters.orgs.length || "")} onClick={() => setMenu(menu === "org" ? "root" : "org")} />
      {menu === "org" && (
        <ToggleList
          options={orgs.map((org) => ({ id: org.name, label: `${org.name} (${org.count})` }))}
          selected={view.filters.orgs}
          onToggle={(org) => onViewChange({ ...view, filters: { ...view.filters, orgs: toggleValue(view.filters.orgs, org) } })}
        />
      )}
      <MenuRow label="Status / draft" value="" onClick={() => setMenu(menu === "status" ? "root" : "status")} />
      {menu === "status" && (
        <ToggleList
          options={(["ready", "unknown", "blocked", "draft"] as StatusFilter[]).map((id) => ({ id, label: id }))}
          selected={view.filters.statuses}
          onToggle={(id) => onViewChange({ ...view, filters: { ...view.filters, statuses: toggleValue(view.filters.statuses, id) } })}
        />
      )}
      <MenuRow label="Review decision" value="" onClick={() => setMenu(menu === "review" ? "root" : "review")} />
      {menu === "review" && (
        <ToggleList
          options={(["approved", "changes_requested", "review_required", "none"] as ReviewFilter[]).map((id) => ({ id, label: id }))}
          selected={view.filters.reviews}
          onToggle={(id) => onViewChange({ ...view, filters: { ...view.filters, reviews: toggleValue(view.filters.reviews, id) } })}
        />
      )}
      <MenuRow label="CI" value="" onClick={() => setMenu(menu === "ci" ? "root" : "ci")} />
      {menu === "ci" && (
        <ToggleList
          options={(["passing", "failing", "pending", "none"] as CiFilter[]).map((id) => ({ id, label: id }))}
          selected={view.filters.ci}
          onToggle={(id) => onViewChange({ ...view, filters: { ...view.filters, ci: toggleValue(view.filters.ci, id) } })}
        />
      )}
      <MenuRow label="Slack request" value="" onClick={() => setMenu(menu === "slack" ? "root" : "slack")} />
      {menu === "slack" && (
        <ToggleList
          options={(["none", "requested", "bumped", "bump_due"] as SlackFilter[]).map((id) => ({ id, label: id }))}
          selected={view.filters.slack}
          onToggle={(id) => onViewChange({ ...view, filters: { ...view.filters, slack: toggleValue(view.filters.slack, id) } })}
        />
      )}
      <label className="mt-1 flex items-center gap-2 px-1">
        <input
          type="checkbox"
          checked={view.filters.starred}
          onChange={() => onViewChange({ ...view, filters: { ...view.filters, starred: !view.filters.starred } })}
        />
        Starred only
      </label>
      <label className="mt-1 flex items-center gap-2 px-1">
        <input
          type="checkbox"
          checked={view.filters.archived}
          onChange={() => onViewChange({ ...view, filters: { ...view.filters, archived: !view.filters.archived } })}
        />
        Archived and closed
      </label>
    </div>
  );
}

function ToggleList<T extends string>({
  options,
  selected,
  onToggle,
}: {
  options: { id: T; label: string }[];
  selected: readonly T[];
  onToggle: (id: T) => void;
}) {
  return (
    <div className="ml-2 border-l border-slate-800 pl-2">
      {options.map((option) => (
        <label key={option.id} className="flex items-center gap-2 py-0.5">
          <input type="checkbox" checked={selected.includes(option.id)} onChange={() => onToggle(option.id)} />
          {option.label}
        </label>
      ))}
    </div>
  );
}

function toggleValue<T extends string>(values: readonly T[], value: T): T[] {
  return values.includes(value) ? values.filter((item) => item !== value) : [...values, value];
}

function labelOf<T extends string>(options: { id: T; label: string }[], id: T): string {
  return options.find((option) => option.id === id)?.label ?? id;
}

type OrgCount = PageDto["orgs"][number];

/** Loaded entries grouped under their section headers, with backend counts. */
/** Drops PRs merged in the app from the shown sections, ahead of the reload. */
export function withoutRemoved(sections: ShownSection[], removed: ReadonlySet<number> | undefined): ShownSection[] {
  if (!removed || removed.size === 0) {
    return sections;
  }
  return sections
    .map((section) => {
      const gone = section.ids.filter((id) => removed.has(id)).length;
      if (gone === 0) {
        return section;
      }
      return {
        ...section,
        rows: section.rows.filter((row) => !removed.has(row.id)),
        ids: section.ids.filter((id) => !removed.has(id)),
        count: Math.max(0, section.count - gone),
      };
    })
    .filter((section) => section.ids.length > 0 || section.rows.length > 0);
}

function shownSections(entries: readonly PageEntry[], all: readonly PageSection[]): ShownSection[] {
  const meta = new Map(all.map((section) => [section.key, section]));
  const shown: ShownSection[] = [];
  let current: ShownSection | null = null;
  for (const entry of entries) {
    if (!current || current.key !== entry.section) {
      const info = meta.get(entry.section);
      if (!info) {
        continue;
      }
      current = { key: info.key, title: info.title, projectId: info.project_id, rows: [], count: info.count, ids: info.ids };
      shown.push(current);
    }
    if (entry.row) {
      current.rows.push(entry.row);
    }
  }
  return shown;
}

function viewTitle(viewKey: string, projects: readonly ProjectDto[]): string {
  if (viewKey === "mine") {
    return "My PRs";
  }
  if (viewKey === "to_review") {
    return "To review";
  }
  if (viewKey.startsWith("project:")) {
    const id = Number(viewKey.slice("project:".length));
    return projects.find((project) => project.id === id)?.name ?? "Project";
  }
  return "Pull requests";
}

