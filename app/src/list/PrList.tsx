import { useEffect, useMemo, useRef, useState } from "react";
import type { ListRow, ProjectDto } from "../api";
import { ChevronDownIcon, ChevronRightIcon, CloseIcon, CommentIcon, ExternalIcon, FolderPlusIcon, FunnelIcon, MoreIcon, SearchIcon, SlackIcon } from "../icons";
import { Tooltip } from "../Tooltip";
import { ReviewerAvatars } from "./ReviewerAvatars";
import { QuickFilters } from "./QuickFilters";
import { AddToProjectMenu } from "./AddToProjectMenu";
import { isPlainShortcut, isTypingTarget, wantsRowEnter } from "./keys";
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
  renameError,
  resetFilters,
  reviewLabel,
  reviewTone,
  slackStatus,
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
import { pageIds, summaryText, type Elsewhere, type PageDto, type PageEntry, type PageRequest, type PageSection } from "./paging";
import { usePagedList, type PageParams } from "./usePagedList";
import { plainText } from "../textInput";

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
  /** "Syncing… 42 so far" while a sync runs, shown after the summary. */
  syncProgress?: string | null;
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
  /** Two or more PRs from the bottom bar (§7.9). */
  onBulkReview: (ids: number[]) => void;
  /** Draft flags changed in the app ahead of the reload, by PR id. */
  draftPatches?: Record<number, boolean>;
}

const GROUPINGS: { id: Grouping; label: string }[] = [
  { id: "project", label: "Project" },
  { id: "repository", label: "Repository" },
  { id: "org", label: "Org" },
  { id: "status", label: "Status" },
  { id: "updated", label: "Updated" },
];

const ORDERINGS: { id: Ordering; label: string }[] = [
  { id: "updated", label: "Updated" },
  { id: "created", label: "Created" },
  { id: "slack", label: "Last Slack request" },
  { id: "readiness", label: "Readiness" },
  { id: "number", label: "Number" },
];

export function PrList({
  loadPage,
  version,
  syncProgress = null,
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
  onBulkReview,
  draftPatches,
}: Props) {
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  const [selectedByView, setSelectedByView] = useState<Record<string, number[]>>({});
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [menu, setMenu] = useState<string | null>(null);
  const [sectionMenu, setSectionMenu] = useState<string | null>(null);
  const [editing, setEditing] = useState<Editing | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const [deleteFor, setDeleteFor] = useState<Section | null>(null);
  const [focusIndex, setFocusIndex] = useState(0);
  /** The view whose "Add to project" menu is open, so switching views closes it. */
  const [addMenuFor, setAddMenuFor] = useState<string | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const commitRename = useRef(false);
  const selected = useMemo(() => new Set(selectedByView[viewKey] ?? []), [selectedByView, viewKey]);

  useEffect(() => {
    onSelectionChange?.(bulkIds(selected));
  }, [selected, onSelectionChange]);
  const addMenu = addMenuFor === viewKey && selected.size > 0;
  const setAddMenu = (open: boolean | ((open: boolean) => boolean)) =>
    setAddMenuFor((current) => {
      const next = typeof open === "function" ? open(current === viewKey) : open;
      return next ? viewKey : null;
    });

  useEffect(() => {
    const timer = window.setTimeout(() => setDebounced(query.trim()), SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [query]);

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
      filters: view.filters,
      query: debounced,
      collapsed: collapsedKeys,
    }),
    [viewKey, view.grouping, view.ordering, view.direction, view.filters, debounced, collapsedKeys],
  );
  const list = usePagedList(loadPage, params, version, now);
  const page = list.page;
  const sections = useMemo(
    () =>
      shownSections(list.entries, page?.sections ?? []).map((section) =>
        draftPatches && Object.keys(draftPatches).length > 0
          ? { ...section, rows: section.rows.map((row) => (row.id in draftPatches ? { ...row, is_draft: draftPatches[row.id] } : row)) }
          : section,
      ),
    [list.entries, page, draftPatches],
  );
  const currentProject = projectIdOf(viewKey);
  const flat = useMemo(() => sections.flatMap((section) => section.rows.map((row) => ({ section, row }))), [sections]);
  // Every matching id, loaded or not: select-all, counts and drag cover them all.
  const ids = useMemo(() => pageIds(page?.sections ?? []), [page]);
  const hidden = hiddenSelectedCount(selected, ids);
  const global = checkState(ids, selected);
  const singleSelected = selected.size === 1 ? (flat.find((item) => selected.has(item.row.id))?.row ?? null) : null;
  // The focused row stays in range when filters or a sync shrink the list.
  const focus = flat.length === 0 ? -1 : Math.min(focusIndex, flat.length - 1);
  const listRef = useRef<HTMLDivElement>(null);
  const sentinelRef = useRef<HTMLDivElement>(null);
  const sentinelVisible = useRef(false);
  const inView = page?.in_view ?? 0;
  const searching = query.trim() !== "" && (query.trim() !== debounced || list.pending);
  const { loadMore, hasMore, loadingMore } = list;

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
    if (root && hasMore && root.scrollTop + root.clientHeight >= root.scrollHeight - LOAD_AHEAD_PX) {
      loadMore();
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
    const ids = bulkIds(selected);
    if (singleSelected && ids.length === 1) {
      onRequestReview(singleSelected);
    } else if (ids.length > 0) {
      onBulkReview(ids);
    }
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

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      const target = event.target instanceof HTMLElement ? event.target : null;
      const typing = isTypingTarget(target);
      if (document.querySelector('[role="dialog"][aria-modal="true"]')) {
        return;
      }
      if (event.key === "/" && !typing) {
        event.preventDefault();
        searchRef.current?.focus();
        return;
      }
      if (event.key === "Escape" && target === searchRef.current) {
        setQuery("");
        return;
      }
      if (typing) {
        return;
      }
      if (event.key === "Escape") {
        if (addMenu) {
          setAddMenu(false);
        } else if (sectionMenu !== null) {
          setSectionMenu(null);
        } else if (selected.size > 0) {
          setSelected(new Set());
        }
        return;
      }
      if (event.key === "a" && (event.metaKey || event.ctrlKey) && !event.altKey) {
        event.preventDefault();
        setSelected(toggleParent(ids, selected));
        return;
      }
      // Cmd+R, Ctrl+F and the like belong to the system, not to the list.
      if (!isPlainShortcut(event)) {
        return;
      }
      const current = focus >= 0 ? flat[focus] : undefined;
      // Shift+x reports "X" in a browser.
      const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
      // ↑/↓ also move while the detail panel is open, so it can step through PRs.
      // Inside the panel the arrows still scroll it.
      const arrow = detailId !== null && (key === "ArrowDown" || key === "ArrowUp") && !target?.closest("[data-detail-panel]");
      if (((key === "j" || key === "k") && !event.shiftKey) || arrow) {
        event.preventDefault();
        if (flat.length === 0) {
          return;
        }
        const delta = key === "j" || key === "ArrowDown" ? 1 : -1;
        const next = Math.min(flat.length - 1, Math.max(0, focus + delta));
        setFocusIndex(next);
        if (detailId !== null && flat[next].row.id !== detailId) {
          onDetails(flat[next].row);
        }
      } else if (key === "x" && event.shiftKey) {
        if (current) {
          setSelected(toggleParent(current.section.ids, selected));
        }
      } else if (key === "x") {
        if (current) {
          toggleRow(current.row.id);
        }
      } else if (key === "r") {
        if (selected.size > 0) {
          requestSelection();
        } else if (current) {
          onRequestReview(current.row);
        }
      } else if (key === "s") {
        const link = current?.row.slack?.permalink;
        if (link) {
          onOpen(link);
        }
      } else if (event.key === "Enter" && wantsRowEnter(target)) {
        if (current) {
          event.preventDefault();
          onDetails(current.row);
        }
      } else if (key === "o") {
        const url = current?.row.url;
        if (url) {
          onOpen(url);
        }
      } else if (key === "f") {
        setMenu((open) => (open ? null : "root"));
      } else if (key === "p") {
        // With nothing selected, P acts on the focused row.
        if (selected.size === 0 && current) {
          setSelected(new Set([current.row.id]));
        }
        if (selected.size > 0 || current) {
          event.preventDefault();
          setAddMenu((open) => !open);
        }
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const header =
    checkState(ids, selected) === "none" && hidden === 0
      ? `${ids.length} open ${ids.length === 1 ? "PR" : "PRs"}`
      : `${selectedCount(ids, selected)} of ${ids.length} selected`;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex items-start justify-between gap-4 px-4 py-3">
        <div>
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
            <h1
              className="text-lg font-semibold"
              title={currentProject !== null ? "Double-click to rename" : undefined}
              onDoubleClick={() => {
                if (currentProject !== null && projects.some((project) => project.id === currentProject)) {
                  setEditing({ id: currentProject, where: "title", name: viewTitle(viewKey, projects), error: null });
                }
              }}
            >
              {viewTitle(viewKey, projects)}
            </h1>
          )}
          {editing?.error && editing.where === "title" && <p className="text-xs text-rose-300">{editing.error}</p>}
          <p className="text-sm text-slate-400">{page ? summaryText(page.summary, viewKey, syncProgress) : (syncProgress ?? "\u00a0")}</p>
        </div>
        <div className="flex items-center gap-2">
          <label className="flex w-80 items-center gap-2 rounded-md border border-slate-700 bg-slate-900 px-3 py-1.5 text-sm focus-within:border-violet-400">
            <SearchIcon className="shrink-0 text-slate-500" />
            <input {...plainText}
              ref={searchRef}
              aria-label="Search"
              className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-slate-500"
              placeholder="Search projects, PRs, descriptions…"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
            {searching && (
              <span role="status" className="shrink-0 text-xs text-slate-500">
                Searching…
              </span>
            )}
          </label>
          <FilterMenu view={view} orgs={page?.orgs ?? []} menu={menu} setMenu={setMenu} onViewChange={onViewChange} />
        </div>
      </header>
      <div className="flex items-center gap-2 border-y border-slate-800 px-4 py-2 text-sm text-slate-300">
        <TriCheckbox label="Select all" state={global} onToggle={() => setSelected(toggleParent(ids, selected))} />
        <span>{header}</span>
        <QuickFilters
          active={view.filters.quick}
          counts={page?.quick ?? null}
          onChange={(quick) => onViewChange({ ...view, filters: { ...view.filters, quick } })}
        />
      </div>
      <div
        ref={listRef}
        className="min-h-0 flex-1 overflow-auto px-3 pb-20 pt-2"
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
        {sections.map((section) => {
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
                  <span
                    className="text-sm font-medium"
                    title={section.projectId !== null ? "Double-click to rename" : undefined}
                    onDoubleClick={() => {
                      if (section.projectId !== null) {
                        setEditing({ id: section.projectId, where: "section", name: section.title, error: null });
                      }
                    }}
                  >
                    {section.title}
                  </span>
                )}
                <span className="text-xs text-slate-500">({section.count})</span>
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
                        const ids = dragIds(row.id, selected);
                        event.dataTransfer.setData(DRAG_MIME, JSON.stringify(ids));
                        event.dataTransfer.setData("text/plain", ids.length === 1 ? row.url : `${ids.length} pull requests`);
                        event.dataTransfer.effectAllowed = "link";
                        setDragBadge(event.dataTransfer, ids.length);
                      }}
                      onDragEnd={() => setDropTarget(null)}
                      onMouseDown={() => {
                        setFocusIndex(index);
                        // With the panel open, clicking a row shows that PR.
                        if (detailId !== null && detailId !== row.id) {
                          onDetails(row);
                        }
                      }}
                      aria-label={`#${row.number} ${row.title}`}
                      aria-current={index === focus ? "true" : undefined}
                      data-focused={index === focus ? "true" : undefined}
                      className={`flex items-center gap-3 border-t border-l-2 border-t-slate-800 px-2 py-1.5 ${index === focus ? "border-l-violet-400 bg-slate-900" : "border-l-transparent"}`}
                    >
                      <TriCheckbox
                        label={`Select ${row.title}`}
                        state={selected.has(row.id) ? "all" : "none"}
                        onToggle={() => toggleRow(row.id)}
                      />
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2 text-sm">
                          {view.show.numberTitle && (
                            <button
                              type="button"
                              className="truncate text-left hover:underline"
                              title="Readiness and code owners (Enter)"
                              onClick={() => onDetails(row)}
                            >
                              <span className="text-slate-500">#{row.number}</span> {row.title}
                            </button>
                          )}
                          {row.state !== "open" && (
                            <span className="shrink-0 rounded bg-slate-800 px-1 text-xs text-slate-300">{row.state === "merged" ? "Merged" : "Closed"}</span>
                          )}
                          {view.show.badges && row.project_ids.length > 1 && (
                            <span
                              className="shrink-0 rounded bg-slate-800 px-1 text-xs text-slate-400"
                              title={`Also in ${row.project_ids.length - 1} other project${row.project_ids.length > 2 ? "s" : ""}`}
                            >
                              +{row.project_ids.length - 1}
                            </span>
                          )}
                        </div>
                        <p className="flex items-center gap-1 truncate text-xs text-slate-500">
                          {view.show.repo && <span>{row.repo}</span>}
                          {view.show.repo && view.show.ci && <span aria-hidden="true">·</span>}
                          {view.show.ci && (
                            <span className="flex items-center gap-1">
                              <Dot tone={ciTone(row.checks)} />
                              {ciLabel(row.checks)}
                            </span>
                          )}
                          {view.show.activity && <span> · updated {ago(row.updated_at, now)}</span>}
                        </p>
                      </div>
                      <div className="flex w-40 shrink-0 items-center">
                        {view.show.review && <ReviewPill decision={row.review_decision} />}
                      </div>
                      {view.show.review && (
                        <div className="flex w-20 shrink-0 items-center justify-end">
                          <ReviewerAvatars reviewers={row.pending_reviewers} />
                        </div>
                      )}
                      {view.show.threads && <Threads row={row} />}
                      <div className="flex w-44 shrink-0 items-center justify-end">
                        <SlackMark row={row} now={now} showMissing={view.show.notRequested} onOpen={onOpen} />
                      </div>
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
        <div ref={sentinelRef} data-testid="list-sentinel" aria-hidden="true" className="h-px" />
        {page && !list.pending && sections.length === 0 && (
          <EmptyList
            title={viewTitle(viewKey, projects)}
            inView={inView}
            query={query}
            filtered={filtersActive(view.filters)}
            elsewhere={page.elsewhere}
            viewTitle={(key) => viewTitle(key, projects)}
            onClearSearch={() => setQuery("")}
            onResetFilters={() => onViewChange(resetFilters(view))}
            onShowView={onShowView}
            onShowFinished={() => onViewChange({ ...view, filters: { ...view.filters, archived: true } })}
          />
        )}
      </div>
      {selected.size > 0 && (
        <div className="pointer-events-none absolute inset-x-0 bottom-4 flex justify-center">
          <div
            role="region"
            aria-label="Selection actions"
            className="pointer-events-auto flex items-center gap-4 rounded-lg border border-slate-700 bg-slate-900 px-5 py-2.5 text-sm shadow-lg"
          >
            <span aria-live="polite" className="text-slate-300">
              {bulkIds(selected).length} selected{hidden > 0 ? ` (${hidden} hidden)` : ""}
            </span>
            <span aria-hidden="true" className="h-6 w-px bg-slate-700" />
            <Tooltip label="Request / bump on Slack" shortcut="r" detail="Preview the review request for each selected PR before anything is sent" side="top">
              <button
                type="button"
                className="flex items-center gap-2 rounded-md bg-violet-600 px-4 py-1.5 font-medium text-white hover:bg-violet-500 disabled:bg-violet-500/40"
                disabled={bulkIds(selected).length === 0}
                onClick={requestSelection}
              >
                <SlackIcon size={16} />
                Request / bump on Slack
              </button>
            </Tooltip>
            <div className="relative">
              <Tooltip label="Add to project" shortcut="p" detail="Link the selected PRs to a project, or start a new one with them" side="top">
                <button
                  type="button"
                  aria-haspopup="menu"
                  aria-expanded={addMenu}
                  className="flex items-center gap-2 rounded-md border border-slate-700 px-4 py-1.5 text-slate-200 hover:bg-slate-800"
                  onClick={() => setAddMenu((open) => !open)}
                >
                  <FolderPlusIcon />
                  Add to project
                </button>
              </Tooltip>
              {addMenu && (
                <AddToProjectMenu
                  projects={projects}
                  count={bulkIds(selected).length}
                  onClose={() => setAddMenu(false)}
                  onPick={(projectId) => {
                    setAddMenu(false);
                    onLinkPrs(projectId, bulkIds(selected));
                  }}
                  onCreate={async (name) => {
                    await onCreateProjectWithPrs(name, bulkIds(selected));
                    setAddMenu(false);
                  }}
                />
              )}
            </div>
            <Tooltip label="Clear selection" shortcut="Esc" detail="Unselect every PR in this view" side="top">
              <button
                type="button"
                className="flex items-center gap-1.5 rounded-md border border-slate-700 px-4 py-1.5 text-slate-200 hover:bg-slate-800"
                onClick={() => setSelected(new Set())}
              >
                <CloseIcon />
                Clear
              </button>
            </Tooltip>
          </div>
        </div>
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
  elsewhere,
  viewTitle,
  onClearSearch,
  onResetFilters,
  onShowView,
  onShowFinished,
}: {
  title: string;
  inView: number;
  query: string;
  filtered: boolean;
  elsewhere: Elsewhere | null;
  viewTitle: (viewKey: string) => string;
  onClearSearch: () => void;
  onResetFilters: () => void;
  onShowView?: (viewKey: string) => void;
  onShowFinished: () => void;
}) {
  const searching = query.trim() !== "";
  if (inView === 0 && !searching) {
    return (
      <div className="px-2 py-10 text-center text-sm text-slate-400">
        <p className="font-medium text-slate-300">No open pull requests in {title}.</p>
        <p className="mt-1 text-xs text-slate-500">New ones appear after the next sync. Refresh from the sidebar to sync now.</p>
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

function SlackMark({
  row,
  now,
  showMissing,
  onOpen,
}: {
  row: ListRow;
  now: Date;
  showMissing: boolean;
  onOpen: (url: string) => void;
}) {
  const status = slackStatus(row.slack, now);
  if (!status) {
    if (!showMissing) {
      return null;
    }
    return (
      <span className="flex items-center gap-1.5 text-xs text-slate-600">
        <SlackIcon active={false} />
        Not requested
      </span>
    );
  }
  const link = row.slack?.permalink;
  return (
    <span className="flex items-center gap-1.5 text-xs text-slate-300" title={status.title}>
      <SlackIcon />
      {status.label}
      {row.slack?.workspace_name && <span className="text-slate-500">· {row.slack.workspace_name}</span>}
      {status.disabled && <span className="text-slate-500">· workspace disabled</span>}
      {link && (
        <Tooltip label="Open in Slack" shortcut="s" detail="Jump to the review request thread" align="end">
          <button
            type="button"
            aria-label={`Open Slack request for ${row.title}`}
            className="rounded p-0.5 text-slate-400 hover:text-slate-100"
            onClick={() => onOpen(link)}
          >
            <ExternalIcon size={12} />
          </button>
        </Tooltip>
      )}
    </span>
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

function ReviewPill({ decision }: { decision: string | null }) {
  const label = reviewLabel(decision);
  if (!label) {
    return null;
  }
  const tone = reviewTone(decision);
  const mark = tone === "good" ? "✓" : tone === "bad" ? "✕" : null;
  return (
    <span className="inline-flex items-center gap-1.5 rounded-full border border-slate-700 bg-slate-900 px-2 py-0.5 text-xs text-slate-200">
      {mark ? (
        <span aria-hidden="true" className={`text-[10px] font-bold ${TONE_TEXT[tone]}`}>
          {mark}
        </span>
      ) : (
        <Dot tone={tone} />
      )}
      {label}
    </span>
  );
}

function Threads({ row }: { row: ListRow }) {
  const { text, title } = threadsLabel(row.unresolved_threads, row.total_threads);
  return (
    <span
      className={`flex w-12 shrink-0 items-center gap-1 text-xs ${row.unresolved_threads > 0 ? "text-slate-300" : "text-slate-600"}`}
      title={title}
      aria-label={title}
    >
      <CommentIcon size={14} />
      {text}
    </span>
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
}: {
  view: SavedView;
  orgs: OrgCount[];
  menu: string | null;
  setMenu: (menu: string | null) => void;
  onViewChange: (view: SavedView) => void;
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
    <div ref={ref} className="relative">
      <Tooltip label="View options" shortcut="f" detail="Grouping, ordering, visible columns and filters" align="end">
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
          <MenuRow label="Ordering" value={labelOf(ORDERINGS, view.ordering)} onClick={() => setMenu(menu === "ordering" ? "root" : "ordering")} />
          {menu === "ordering" && (
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

function selectedCount(ids: readonly number[], selected: ReadonlySet<number>): number {
  return ids.filter((id) => selected.has(id)).length;
}
