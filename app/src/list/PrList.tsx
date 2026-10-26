import { useEffect, useMemo, useRef, useState } from "react";
import type { ListRow, ProjectDto } from "../api";
import {
  ago,
  bulkIds,
  checkState,
  ciLabel,
  filtersActive,
  groupRows,
  hiddenSelectedCount,
  renameError,
  resetFilters,
  reviewLabel,
  sectionIds,
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

export function PrList({ rows, projects, viewKey, view, onViewChange, now, onOpen, onRename, onArchive, onDelete }: Props) {
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  const [selectedByView, setSelectedByView] = useState<Record<string, number[]>>({});
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [menu, setMenu] = useState<string | null>(null);
  const [sectionMenu, setSectionMenu] = useState<string | null>(null);
  const [editing, setEditing] = useState<{ id: number; name: string; error: string | null } | null>(null);
  const [rulesFor, setRulesFor] = useState<string | null>(null);
  const [deleteFor, setDeleteFor] = useState<Section | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
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
  const sections = useMemo(() => groupRows(shown, projects, view, now), [shown, projects, view, now]);
  const flat = useMemo(() => sections.flatMap((section) => section.rows.map((row) => ({ section, row }))), [sections]);
  const ids = visibleIds(sections);
  const hidden = hiddenSelectedCount(selected, ids);
  const global = checkState(ids, selected);

  function setSelected(next: Set<number>) {
    setSelectedByView((current) => ({ ...current, [viewKey]: [...next] }));
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
      const typing = target?.tagName === "INPUT" || target?.tagName === "TEXTAREA";
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
      if (event.key === "j" || event.key === "k") {
        event.preventDefault();
        setFocusIndex((index) => {
          if (flat.length === 0) {
            return 0;
          }
          const delta = event.key === "j" ? 1 : -1;
          return Math.min(flat.length - 1, Math.max(0, index + delta));
        });
      } else if (event.key === "x" && event.shiftKey) {
        const current = flat[focusIndex];
        if (current) {
          setSelected(toggleParent(sectionIds(current.section), selected));
        }
      } else if (event.key === "x") {
        const current = flat[focusIndex];
        if (current) {
          toggleRow(current.row.id);
        }
      } else if (event.key === "a" && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        setSelected(toggleParent(ids, selected));
      } else if (event.key === "r") {
        setNotice("Slack preview comes in M10");
      } else if (event.key === "s") {
        const link = flat[focusIndex]?.row.slack?.permalink;
        if (link) {
          onOpen(link);
        }
      } else if (event.key === "o") {
        const url = flat[focusIndex]?.row.url;
        if (url) {
          onOpen(url);
        }
      } else if (event.key === "f") {
        setMenu((open) => (open ? null : "root"));
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const projectCount = new Set(shown.flatMap((row) => row.project_ids)).size;
  const header =
    checkState(ids, selected) === "none" && hidden === 0 ? `${ids.length} PRs` : `${selectedCount(ids, selected)} of ${ids.length} selected`;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex items-start justify-between gap-4 px-4 py-3">
        <div>
          <h1 className="text-lg font-semibold">{viewTitle(viewKey, projects)}</h1>
          <p className="text-sm text-slate-400">
            {shown.length} open PRs across {projectCount} projects
          </p>
        </div>
        <div className="flex items-center gap-2">
          <input
            ref={searchRef}
            aria-label="Search"
            className="w-72 rounded border border-slate-700 bg-slate-900 px-3 py-1.5 text-sm"
            placeholder="Search projects, PRs, descriptions…"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
          <FilterMenu view={view} rows={rows} orgs={orgOptions(rows)} menu={menu} setMenu={setMenu} onViewChange={onViewChange} />
        </div>
      </header>
      <div className="flex items-center gap-2 border-y border-slate-800 px-4 py-2 text-sm text-slate-300">
        <TriCheckbox label="Select all" state={global} onToggle={() => setSelected(toggleParent(ids, selected))} />
        <span>{header}</span>
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-3 py-2">
        {sections.map((section) => {
          const open = !collapsed[section.key];
          const state = checkState(sectionIds(section), selected);
          return (
            <section key={section.key} className="mb-2 rounded-lg border border-slate-800">
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
                {editing?.id === section.projectId ? (
                  <input
                    aria-label={`Rename ${section.title}`}
                    className="rounded border border-slate-600 bg-slate-900 px-2 py-0.5 text-sm"
                    value={editing.name}
                    autoFocus
                    onChange={(event) => setEditing({ ...editing, name: event.target.value, error: null })}
                    onKeyDown={(event) => {
                      if (event.key === "Escape") {
                        setEditing(null);
                      }
                      if (event.key === "Enter") {
                        event.preventDefault();
                        void commit(section);
                      }
                    }}
                    onBlur={() => {
                      if (!commitRename.current) {
                        setEditing(null);
                      }
                    }}
                  />
                ) : (
                  <span
                    className="text-sm font-medium"
                    onDoubleClick={() => {
                      if (section.projectId !== null) {
                        setEditing({ id: section.projectId, name: section.title, error: null });
                      }
                    }}
                  >
                    {section.title}
                  </span>
                )}
                <span className="text-xs text-slate-500">({section.rows.length})</span>
                {editing?.error && editing.id === section.projectId && (
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
                        <button type="button" className="block w-full px-3 py-1.5 text-left" onClick={() => { setRulesFor(section.title); setSectionMenu(null); }}>
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
              {open &&
                section.rows.map((row) => {
                  const index = flat.findIndex((item) => item.row.id === row.id && item.section.key === section.key);
                  return (
                    <article
                      key={`${section.key}-${row.id}`}
                      className={`flex gap-2 border-t border-slate-800 px-2 py-1.5 ${index === focusIndex ? "bg-slate-900" : ""}`}
                    >
                      <TriCheckbox
                        label={`Select ${row.title}`}
                        state={selected.has(row.id) ? "all" : "none"}
                        onToggle={() => toggleRow(row.id)}
                      />
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2 text-sm">
                          {view.show.numberTitle && (
                            <span className="truncate">
                              <span className="text-slate-500">#{row.number}</span> {row.title}
                            </span>
                          )}
                          {view.show.badges && row.project_ids.length > 1 && (
                            <span className="text-xs text-slate-500">+{row.project_ids.length - 1}</span>
                          )}
                          {view.show.review && reviewLabel(row.review_decision) && (
                            <span className="rounded bg-slate-800 px-1.5 text-xs text-slate-300">{reviewLabel(row.review_decision)}</span>
                          )}
                          {view.show.threads && (
                            <span className="text-xs text-slate-500">
                              {row.unresolved_threads}/{row.total_threads}
                            </span>
                          )}
                          <SlackMark row={row} now={now} showMissing={view.show.notRequested} onOpen={onOpen} />
                        </div>
                        <p className="text-xs text-slate-500">
                          {view.show.repo && <span>{row.repo}</span>}
                          {view.show.repo && view.show.ci && <span> · </span>}
                          {view.show.ci && <span>{ciLabel(row.checks)}</span>}
                          {view.show.activity && <span> · {ago(row.updated_at, now)}</span>}
                          {view.show.teams && <span className="sr-only">owners arrive later</span>}
                        </p>
                      </div>
                    </article>
                  );
                })}
            </section>
          );
        })}
        {sections.length === 0 && <p className="px-2 py-6 text-sm text-slate-500">No pull requests match.</p>}
      </div>
      {selected.size > 0 && (
        <div className="pointer-events-none absolute inset-x-0 bottom-14 flex justify-center">
          <div className="pointer-events-auto flex items-center gap-3 rounded-full border border-slate-700 bg-slate-900 px-4 py-2 text-sm shadow-lg">
            <span>
              {bulkIds(selected).length} selected{hidden > 0 ? ` (${hidden} hidden)` : ""}
            </span>
            <button type="button" className="rounded bg-violet-500/40 px-3 py-1 text-white" disabled title="Slack requests come in M10">
              Request / bump on Slack
            </button>
            <button type="button" onClick={() => setSelected(new Set())}>
              Clear
            </button>
          </div>
        </div>
      )}
      {rulesFor && (
        <dialog open className="rounded border border-slate-700 bg-slate-900 p-4 text-sm text-slate-200">
          <p>Auto-link rules for {rulesFor} arrive in a later milestone. Archive and delete work from this menu.</p>
          <button type="button" className="mt-3 rounded bg-slate-800 px-3 py-1" onClick={() => setRulesFor(null)}>
            Close
          </button>
        </dialog>
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
      {notice && <p className="px-4 pb-2 text-sm text-amber-200">{notice}</p>}
    </div>
  );

  async function commit(section: Section) {
    if (!editing || section.projectId === null) {
      return;
    }
    const error = renameError(editing.name, projects, section.projectId);
    if (error) {
      setEditing({ ...editing, error });
      return;
    }
    commitRename.current = true;
    try {
      await onRename(section.projectId, editing.name.trim());
      setEditing(null);
    } catch (err) {
      setEditing({ ...editing, error: err instanceof Error ? err.message : "Could not rename" });
    } finally {
      commitRename.current = false;
    }
  }
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
  if (!row.slack) {
    if (!showMissing) {
      return null;
    }
    return <span className="text-xs text-slate-600">Not requested</span>;
  }
  const bumped = row.slack.last_bumped_at ? ` · bumped ${ago(row.slack.last_bumped_at, now)}` : "";
  return (
    <span className="ml-auto flex items-center gap-1 text-xs text-slate-300" title={`${row.slack.workspace} · #${row.slack.channel_name}`}>
      Requested · {ago(row.slack.requested_at, now)}
      {bumped}
      {row.slack.permalink && (
        <button type="button" aria-label={`Open Slack request for ${row.title}`} className="text-slate-400" onClick={() => onOpen(row.slack!.permalink!)}>
          ↗
        </button>
      )}
    </span>
  );
}

function TriCheckbox({ label, state, onToggle }: { label: string; state: CheckState; onToggle: () => void }) {
  return (
    <input
      type="checkbox"
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
  return (
    <div className="relative">
      <button
        type="button"
        aria-label="Filter menu"
        className="relative rounded border border-slate-700 px-2 py-1.5 text-sm"
        onClick={() => setMenu(menu ? null : "root")}
      >
        ▾
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
