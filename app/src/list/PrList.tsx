import { useEffect, useMemo, useRef, useState } from "react";
import type { ListRow, ProjectDto } from "../api";
import { CommentIcon, ExternalIcon, FunnelIcon, SearchIcon, SlackIcon } from "../icons";
import { isPlainShortcut, wantsRowEnter } from "./keys";
import {
  ago,
  bulkIds,
  checkState,
  ciLabel,
  DRAG_MIME,
  dragIds,
  headerSummary,
  keptEmptyProjects,
  parseDragIds,
  projectIdOf,
  ciTone,
  filtersActive,
  groupRows,
  hiddenSelectedCount,
  renameError,
  resetFilters,
  reviewLabel,
  reviewTone,
  sectionIds,
  slackStatus,
  threadsLabel,
  toggleParent,
  visibleIds,
  visibleRows,
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

interface Props {
  rows: ListRow[];
  projects: ProjectDto[];
  viewKey: string;
  view: SavedView;
  onViewChange: (view: SavedView) => void;
  now: Date;
  onOpen: (url: string) => void;
  onRename: (id: number, name: string) => Promise<void>;
  onArchive: (id: number) => void;
  onDelete: (id: number) => void;
  /** Opens the project dialog (rules, linked PRs) from the section ⋯ menu. */
  onEditProject: (id: number) => void;
  /** Manual links from dropping rows on a project section. */
  onLinkPrs: (projectId: number, prIds: number[]) => void;
  /** Opens the readiness and code-owner details for one row. */
  onDetails: (row: ListRow) => void;
  /** Opens the Slack request/bump preview for one row. */
  onRequestReview: (row: ListRow) => void;
  /** Two or more PRs from the bottom bar (§7.9). */
  onBulkReview: (ids: number[]) => void;
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
  rows,
  projects,
  viewKey,
  view,
  onViewChange,
  now,
  onOpen,
  onRename,
  onArchive,
  onDelete,
  onEditProject,
  onLinkPrs,
  onDetails,
  onRequestReview,
  onBulkReview,
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
  const searchRef = useRef<HTMLInputElement>(null);
  const commitRename = useRef(false);
  const selected = useMemo(() => new Set(selectedByView[viewKey] ?? []), [selectedByView, viewKey]);

  useEffect(() => {
    const timer = window.setTimeout(() => setDebounced(query), 150);
    return () => window.clearTimeout(timer);
  }, [query]);

  const shown = useMemo(
    () => visibleRows(rows, projects, viewKey, view, debounced, now),
    [rows, projects, viewKey, view, debounced, now],
  );
  const keepEmpty = useMemo(
    () => keptEmptyProjects(projects, viewKey, view.filters, debounced),
    [projects, viewKey, view.filters, debounced],
  );
  const sections = useMemo(() => groupRows(shown, projects, view, now, keepEmpty), [shown, projects, view, now, keepEmpty]);
  const currentProject = projectIdOf(viewKey);
  const flat = useMemo(() => sections.flatMap((section) => section.rows.map((row) => ({ section, row }))), [sections]);
  const ids = visibleIds(sections);
  const hidden = hiddenSelectedCount(selected, ids);
  const global = checkState(ids, selected);
  const singleSelected = selected.size === 1 ? (rows.find((row) => selected.has(row.id)) ?? null) : null;
  // The focused row stays in range when filters or a sync shrink the list.
  const focus = flat.length === 0 ? -1 : Math.min(focusIndex, flat.length - 1);
  const listRef = useRef<HTMLDivElement>(null);
  const inView = useMemo(
    () => visibleRows(rows, projects, viewKey, resetFilters(view), "", now).length,
    [rows, projects, viewKey, view, now],
  );

  useEffect(() => {
    const element = listRef.current?.querySelector<HTMLElement>('[data-focused="true"]');
    element?.scrollIntoView?.({ block: "nearest" });
  }, [focus]);

  function setSelected(next: Set<number>) {
    setSelectedByView((current) => ({ ...current, [viewKey]: [...next] }));
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
      const typing = target?.tagName === "INPUT" || target?.tagName === "TEXTAREA" || target?.tagName === "SELECT";
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
        if (sectionMenu !== null) {
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
      if ((key === "j" || key === "k") && !event.shiftKey) {
        event.preventDefault();
        setFocusIndex((index) => {
          if (flat.length === 0) {
            return 0;
          }
          const delta = key === "j" ? 1 : -1;
          return Math.min(flat.length - 1, Math.max(0, Math.min(index, flat.length - 1) + delta));
        });
      } else if (key === "x" && event.shiftKey) {
        if (current) {
          setSelected(toggleParent(sectionIds(current.section), selected));
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
          <p className="text-sm text-slate-400">{headerSummary(shown, projects, viewKey, view.filters)}</p>
        </div>
        <div className="flex items-center gap-2">
          <label className="flex w-80 items-center gap-2 rounded-md border border-slate-700 bg-slate-900 px-3 py-1.5 text-sm focus-within:border-violet-400">
            <SearchIcon className="shrink-0 text-slate-500" />
            <input
              ref={searchRef}
              aria-label="Search"
              className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-slate-500"
              placeholder="Search projects, PRs, descriptions…"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
          </label>
          <FilterMenu view={view} rows={rows} orgs={orgOptions(rows)} menu={menu} setMenu={setMenu} onViewChange={onViewChange} />
        </div>
      </header>
      <div className="flex items-center gap-2 border-y border-slate-800 px-4 py-2 text-sm text-slate-300">
        <TriCheckbox label="Select all" state={global} onToggle={() => setSelected(toggleParent(ids, selected))} />
        <span>{header}</span>
      </div>
      <div ref={listRef} className="min-h-0 flex-1 overflow-auto px-3 pb-20 pt-2" aria-label="Pull requests" role="region">
        {sections.map((section) => {
          const open = !collapsed[section.key];
          const state = checkState(sectionIds(section), selected);
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
                <button
                  type="button"
                  aria-expanded={open}
                  aria-label={`${open ? "Collapse" : "Expand"} ${section.title}`}
                  className="w-5 text-slate-400"
                  onClick={() => setCollapsed((current) => ({ ...current, [section.key]: open }))}
                >
                  {open ? "▾" : "▸"}
                </button>
                <TriCheckbox
                  label={`Select ${section.title}`}
                  state={state}
                  onToggle={() => setSelected(toggleParent(sectionIds(section), selected))}
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
                <span className="text-xs text-slate-500">({section.rows.length})</span>
                {editing?.error && editing.where === "section" && editing.id === section.projectId && (
                  <span className="text-xs text-rose-300">{editing.error}</span>
                )}
                <span className="flex-1" />
                {section.projectId !== null && (
                  <div className="relative">
                    <button
                      type="button"
                      aria-label={`Actions for ${section.title}`}
                      className="rounded px-2 text-slate-400"
                      onClick={() => setSectionMenu((current) => (current === section.key ? null : section.key))}
                    >
                      ⋯
                    </button>
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
              {open && section.rows.length === 0 && (
                <p className="border-t border-slate-800 px-3 py-3 text-xs text-slate-500">
                  No PRs here yet. Drag PRs onto this project, or add an auto-link rule from ⋯.
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
                      }}
                      onDragEnd={() => setDropTarget(null)}
                      onMouseDown={() => setFocusIndex(index)}
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
        {sections.length === 0 && (
          <EmptyList
            title={viewTitle(viewKey, projects)}
            inView={inView}
            query={query}
            filtered={filtersActive(view.filters)}
            onClearSearch={() => setQuery("")}
            onResetFilters={() => onViewChange(resetFilters(view))}
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
            <button
              type="button"
              title="Request / bump on Slack (r)"
              className="flex items-center gap-2 rounded-md bg-violet-600 px-4 py-1.5 font-medium text-white hover:bg-violet-500 disabled:bg-violet-500/40"
              disabled={bulkIds(selected).length === 0}
              onClick={requestSelection}
            >
              <SlackIcon />
              Request / bump on Slack
            </button>
            <button
              type="button"
              title="Clear selection (Esc)"
              className="flex items-center gap-1.5 rounded-md border border-slate-700 px-4 py-1.5 text-slate-200 hover:bg-slate-800"
              onClick={() => setSelected(new Set())}
            >
              <span aria-hidden="true">✕</span>
              Clear
            </button>
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
  onClearSearch,
  onResetFilters,
}: {
  title: string;
  inView: number;
  query: string;
  filtered: boolean;
  onClearSearch: () => void;
  onResetFilters: () => void;
}) {
  if (inView === 0 && query.trim() === "") {
    return (
      <div className="px-2 py-10 text-center text-sm text-slate-400">
        <p className="font-medium text-slate-300">No open pull requests in {title}.</p>
        <p className="mt-1 text-xs text-slate-500">New ones appear after the next sync. Refresh from the sidebar to sync now.</p>
      </div>
    );
  }
  return (
    <div className="px-2 py-10 text-center text-sm text-slate-400">
      <p className="font-medium text-slate-300">No pull requests match {query.trim() !== "" && filtered ? "your search and filters" : query.trim() !== "" ? "your search" : "your filters"}.</p>
      <div className="mt-3 flex justify-center gap-2">
        {query.trim() !== "" && (
          <button type="button" className="rounded border border-slate-700 px-3 py-1 hover:bg-slate-800" onClick={onClearSearch}>
            Clear search
          </button>
        )}
        {filtered && (
          <button type="button" className="rounded border border-slate-700 px-3 py-1 hover:bg-slate-800" onClick={onResetFilters}>
            Reset filters
          </button>
        )}
      </div>
    </div>
  );
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
    <input
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
      {link && (
        <button
          type="button"
          aria-label={`Open Slack request for ${row.title}`}
          className="rounded p-0.5 text-slate-400 hover:text-slate-100"
          onClick={() => onOpen(link)}
        >
          <ExternalIcon size={12} />
        </button>
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
  rows,
  orgs,
  menu,
  setMenu,
  onViewChange,
}: {
  view: SavedView;
  rows: ListRow[];
  orgs: string[];
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
      <button
        type="button"
        aria-label="Filter menu"
        aria-haspopup="menu"
        aria-expanded={open}
        title="Grouping, ordering, and filters (f)"
        className={`relative flex h-8 w-8 items-center justify-center rounded-md border text-slate-300 hover:text-white ${open ? "border-violet-400 bg-slate-800" : "border-slate-700 bg-slate-900"}`}
        onClick={() => setMenu(menu ? null : "root")}
      >
        <FunnelIcon />
        {filtersActive(view.filters) && <span aria-label="Filters active" className="absolute -right-1 -top-1 h-2 w-2 rounded-full bg-violet-400" />}
      </button>
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
          <FilterChoices view={view} rows={rows} orgs={orgs} menu={menu} setMenu={setMenu} onViewChange={onViewChange} />
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
  rows,
  orgs,
  menu,
  setMenu,
  onViewChange,
}: {
  view: SavedView;
  rows: ListRow[];
  orgs: string[];
  menu: string;
  setMenu: (menu: string | null) => void;
  onViewChange: (view: SavedView) => void;
}) {
  return (
    <div>
      <MenuRow label="Org" value={String(view.filters.orgs.length || "")} onClick={() => setMenu(menu === "org" ? "root" : "org")} />
      {menu === "org" && (
        <ToggleList
          options={orgs.map((org) => ({ id: org, label: `${org} (${rows.filter((row) => row.owner === org).length})` }))}
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
        Archived
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

function orgOptions(rows: readonly ListRow[]): string[] {
  return [...new Set(rows.map((row) => row.owner))].sort();
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
