import { useEffect, useId, useLayoutEffect, useRef, useState, type DragEvent as ReactDragEvent, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { ChevronDownIcon, ChevronRightIcon, ClockIcon, HomeIcon, ListIcon, PlusIcon, SlackLogo } from "../icons";
import { plainText } from "../textInput";
import { Tooltip } from "../Tooltip";
import { CountSlot, opensRowMenu } from "./CountBadge";
import { SearchField } from "./SearchField";
import { SectionHeader } from "./SectionHeader";
import { formatViewCount, knownTotal, sidebarCountLabel } from "./viewCount";
import {
  builtinKey,
  defaultCatalog,
  duplicateOf,
  joinRules,
  nextCustomKey,
  normalizeCatalog,
  orderedViews,
  parentKey,
  viewNameError,
  viewQueryError,
  type ParentKey,
  type SidebarView,
} from "./viewCatalog";
import { applyViewMove, moveConfirmMessage, planViewMove, VIEW_DRAG_MIME, type ViewMove } from "./viewMove";

interface Props {
  views: readonly SidebarView[];
  current: string;
  /** Match counts by view key. `null` is unknown and stays blank; zero stays blank too. */
  counts: Readonly<Record<string, number | null>>;
  onSelect: (key: string) => void;
  onChange: (views: SidebarView[]) => void;
  /** Copies the source view's saved Sort, Group, and Filter onto the new key. */
  onDuplicateState?: (from: string, to: string) => void;
  /** The Views section is folded. The rows stay in the catalog, so shortcuts can still open them. */
  collapsed?: boolean;
  onToggleCollapsed?: () => void;
  /** Parent keys whose children are hidden. */
  collapsedParents?: readonly string[];
  onToggleParent?: (key: string) => void;
  /** Captured when New view opens. Duplicate does not use it. */
  createDraft?: { parent: ParentKey | null; query: string };
  /** The section fills leftover sidebar space and scrolls its own list. */
  fill?: boolean;
  /**
   * Local counts for the rule before and after a move. Null when either rule
   * needs GitHub; the confirm then names the added qualifiers without numbers.
   */
  countMove?: (before: string, after: string) => Promise<{ before: number; after: number } | null>;
}

type Dialog =
  | { kind: "create"; parent: ParentKey | null; query: string }
  | { kind: "edit"; view: SidebarView }
  | { kind: "rename"; view: SidebarView }
  | { kind: "delete"; view: SidebarView };

interface MenuState {
  key: string;
  x: number;
  y: number;
  anchor: HTMLElement | null;
}

interface ConfirmState {
  move: ViewMove;
  before: string | null;
  counts: { before: number; after: number } | null;
  x: number;
  y: number;
}

interface RowDrag {
  enabled: boolean;
  dragging: boolean;
  active: boolean;
  onStart: (event: ReactDragEvent<HTMLElement>) => void;
  onEnd: () => void;
  onOver: (event: ReactDragEvent<HTMLElement>) => void;
  onLeave: (event: ReactDragEvent<HTMLElement>) => void;
  onDrop: (event: ReactDragEvent<HTMLElement>) => void;
}

/**
 * Default and custom sidebar views. The … menu and a right-click share one
 * menu. Built-in views reset instead of deleting.
 */
export function SidebarViews({
  views,
  current,
  counts,
  onSelect,
  onChange,
  onDuplicateState,
  collapsed = false,
  onToggleCollapsed,
  collapsedParents = [],
  onToggleParent,
  createDraft = { parent: null, query: "" },
  fill = false,
  countMove,
}: Props) {
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const [drag, setDrag] = useState<{ key: string; x: number; y: number } | null>(null);
  const [over, setOver] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<ConfirmState | null>(null);
  const anchors = useRef(new Map<string, HTMLButtonElement>());
  const dragging = useRef<string | null>(null);
  const commitId = useRef(0);

  function openMenu(key: string, x: number, y: number, anchor: HTMLElement | null) {
    setMenu({ key, x, y, anchor });
  }

  function closeMenu(restore: boolean) {
    const anchor = menu?.anchor ?? null;
    setMenu(null);
    if (restore) {
      anchor?.focus();
    }
  }

  function parentLabel(key: ParentKey): string {
    return views.find((view) => view.key === key)?.name ?? (key === "mine" ? "My PRs" : "To review");
  }

  function finish(move: ViewMove, before: string | null) {
    onChange(applyViewMove(views, move.key, move.parent, move.query, before));
    if (move.parent && collapsedParents.includes(move.parent)) {
      onToggleParent?.(move.parent);
    }
  }

  async function commit(move: ViewMove, before: string | null, x: number, y: number) {
    if (!move.needsConfirm || move.parent === null) {
      finish(move, before);
      return;
    }
    const token = commitId.current + 1;
    commitId.current = token;
    let counts: { before: number; after: number } | null = null;
    if (countMove) {
      try {
        counts = await countMove(move.beforeQuery, move.afterQuery);
      } catch {
        counts = null;
      }
    }
    if (commitId.current !== token) {
      return;
    }
    setConfirm({ move, before, counts, x, y });
  }

  function requestMove(key: string, parent: ParentKey | null, before: string | null, x: number, y: number) {
    const move = planViewMove(views, key, parent, before);
    if (!move) {
      return;
    }
    void commit(move, before, x, y);
  }

  function carriesView(event: ReactDragEvent<HTMLElement>): boolean {
    if (dragging.current) {
      return true;
    }
    return Array.from(event.dataTransfer.types).includes(VIEW_DRAG_MIME);
  }

  function startDrag(view: SidebarView, event: ReactDragEvent<HTMLElement>) {
    if (view.builtin) {
      event.preventDefault();
      return;
    }
    dragging.current = view.key;
    event.dataTransfer.setData(VIEW_DRAG_MIME, view.key);
    event.dataTransfer.setData("text/plain", view.name);
    event.dataTransfer.effectAllowed = "move";
    hideNativeDragImage(event.dataTransfer);
    setDrag({ key: view.key, x: event.clientX, y: event.clientY });
  }

  function endDrag() {
    dragging.current = null;
    setDrag(null);
    setOver(null);
  }

  function hoverDrop(event: ReactDragEvent<HTMLElement>, id: string) {
    if (!carriesView(event)) {
      return;
    }
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
    setOver(id);
    setDrag((current) => (current ? { ...current, x: event.clientX, y: event.clientY } : current));
  }

  function leaveDrop(event: ReactDragEvent<HTMLElement>, id: string) {
    if (event.currentTarget.contains(event.relatedTarget as Node | null)) {
      return;
    }
    setOver((current) => (current === id ? null : current));
  }

  function dropOn(parent: ParentKey | null, before: string | null, event: ReactDragEvent<HTMLElement>) {
    if (!carriesView(event)) {
      return;
    }
    event.preventDefault();
    const key = dragging.current || event.dataTransfer.getData(VIEW_DRAG_MIME);
    const point = { x: event.clientX, y: event.clientY };
    endDrag();
    if (key) {
      requestMove(key, parent, before, point.x, point.y);
    }
  }

  function rowDrag(view: SidebarView, slot: "parent" | "child" | "top"): RowDrag | null {
    if (slot === "parent" && parentKey(view.key) === null) {
      return null;
    }
    const id = slot === "parent" ? `parent:${view.key}` : `before:${view.key}`;
    const parent = slot === "parent" ? parentKey(view.key) : slot === "top" ? null : parentKey(view.parent);
    const before = slot === "parent" ? null : view.key;
    return {
      enabled: !view.builtin,
      dragging: drag?.key === view.key,
      active: over === id && drag?.key !== view.key,
      onStart: (event) => startDrag(view, event),
      onEnd: endDrag,
      onOver: (event) => hoverDrop(event, id),
      onLeave: (event) => leaveDrop(event, id),
      onDrop: (event) => dropOn(parent, before, event),
    };
  }

  const active = views.find((view) => view.key === menu?.key) ?? null;
  const rows = orderedViews(views);
  const hiddenParent = new Set(collapsedParents);

  return (
    <section aria-labelledby="views-heading" className={`flex flex-col px-2 text-sm ${fill && !collapsed ? "min-h-0 flex-1" : ""}`}>
      <SectionHeader
        id="views-heading"
        icon={<ListIcon size={14} />}
        label="Views"
        collapsed={collapsed}
        hint={knownTotal(views.map((view) => counts[view.key] ?? null))}
        onToggle={() => onToggleCollapsed?.()}
        controls={
          <Tooltip label="New view" detail="Save a name and a query" align="end">
            <button
              type="button"
              aria-label="New view"
              className="rounded p-0.5 text-slate-500 hover:bg-slate-800 hover:text-slate-200"
              onClick={() => {
                setMenu(null);
                setDialog({ kind: "create", parent: createDraft.parent, query: createDraft.query });
              }}
            >
              <PlusIcon />
            </button>
          </Tooltip>
        }
      />
      {!collapsed && (
        <nav aria-label="Views" className={`flex flex-col gap-0.5 ${fill ? "min-h-0 overflow-auto" : ""}`}>
          {rows
            .filter((view) => view.builtin || !parentKey(view.parent))
            .map((view) => {
            const children = views.filter((item) => !item.builtin && parentKey(item.parent) === view.key);
            const folded = hiddenParent.has(view.key);
            const slot = view.builtin ? "parent" : "top";
            return (
              <div key={view.key}>
                <ViewRow
                  view={view}
                  selected={current === view.key}
                  count={counts[view.key] ?? null}
                  menuOpen={menu?.key === view.key}
                  child={false}
                  canFold={children.length > 0}
                  folded={folded}
                  drag={rowDrag(view, slot)}
                  onFold={() => onToggleParent?.(view.key)}
                  onSelect={() => onSelect(view.key)}
                  onMenu={(x, y, anchor) => openMenu(view.key, x, y, anchor)}
                  register={(node) => {
                    if (node) {
                      anchors.current.set(view.key, node);
                    } else {
                      anchors.current.delete(view.key);
                    }
                  }}
                />
                {children.length > 0 && !folded && (
                  <div data-connector="" className="relative">
                    {children.map((child, index) => (
                      <div key={child.key} className="relative ml-4 pl-2">
                        <span
                          aria-hidden="true"
                          className={`absolute left-0 w-px bg-slate-700 ${index === children.length - 1 ? "top-0 h-1/2" : "inset-y-0"}`}
                        />
                        <span aria-hidden="true" className="absolute left-0 top-1/2 h-px w-2 bg-slate-700" />
                        <ViewRow
                          view={child}
                          selected={current === child.key}
                          count={counts[child.key] ?? null}
                          menuOpen={menu?.key === child.key}
                          child
                          canFold={false}
                          folded={false}
                          drag={rowDrag(child, "child")}
                          onFold={() => undefined}
                          onSelect={() => onSelect(child.key)}
                          onMenu={(x, y, anchor) => openMenu(child.key, x, y, anchor)}
                          register={(node) => {
                            if (node) {
                              anchors.current.set(child.key, node);
                            } else {
                              anchors.current.delete(child.key);
                            }
                          }}
                        />
                      </div>
                    ))}
                  </div>
                )}
              </div>
            );
          })}
          {drag && (
            <div
              data-drop-zone="top"
              data-drop-target={over === "top" ? "active" : undefined}
              className={`rounded-md text-xs text-slate-300 ${over === "top" ? "bg-violet-500/20 px-2 py-1.5 ring-1 ring-inset ring-violet-400" : "h-6"}`}
              onDragOver={(event) => hoverDrop(event, "top")}
              onDragLeave={(event) => leaveDrop(event, "top")}
              onDrop={(event) => dropOn(null, null, event)}
            >
              {over === "top" ? "Top level" : ""}
            </div>
          )}
        </nav>
      )}
      {drag &&
        createPortal(
          <div
            data-testid="view-drag-ghost"
            className="pointer-events-none fixed z-50 cursor-grabbing rounded-md bg-slate-800 px-2 py-1 text-sm text-slate-100 shadow-lg ring-1 ring-violet-400"
            style={{ left: drag.x + 12, top: drag.y + 8 }}
          >
            {views.find((view) => view.key === drag.key)?.name}
          </div>,
          document.body,
        )}
      {menu && active && (
        <ViewMenu
          view={active}
          views={views}
          x={menu.x}
          y={menu.y}
          onClose={closeMenu}
          onRename={() => setDialog({ kind: "rename", view: active })}
          onEdit={() => setDialog({ kind: "edit", view: active })}
          onDuplicate={() => {
            const copy = duplicateOf(views, active);
            onChange(normalizeCatalog([...views, copy]));
            onDuplicateState?.(active.key, copy.key);
          }}
          onDelete={() => setDialog({ kind: "delete", view: active })}
          onMove={(parent) => requestMove(active.key, parent, null, menu.x, menu.y)}
          onReset={() => {
            const fallback = defaultCatalog().find((item) => item.key === active.key);
            if (!fallback) {
              return;
            }
            // normalizeCatalog renames a custom view that took the default name meanwhile.
            onChange(normalizeCatalog(views.map((item) => (item.key === active.key ? { ...item, name: fallback.name, query: fallback.query } : item))));
          }}
        />
      )}
      {confirm && confirm.move.parent && (
        <MoveConfirm
          message={moveConfirmMessage(parentLabel(confirm.move.parent), confirm.move.added, confirm.counts)}
          x={confirm.x}
          y={confirm.y}
          onCancel={() => setConfirm(null)}
          onMove={() => {
            finish(confirm.move, confirm.before);
            setConfirm(null);
          }}
        />
      )}
      {dialog && (
        <ViewDialog
          dialog={dialog}
          views={views}
          onClose={() => setDialog(null)}
          onSave={(next) => {
            onChange(next);
            if (dialog.kind === "create") {
              const created = next.find((view) => !views.some((old) => old.key === view.key));
              if (created) {
                onSelect(created.key);
              }
            }
            setDialog(null);
          }}
        />
      )}
    </section>
  );
}

function ViewRow({
  view,
  selected,
  count,
  menuOpen,
  child,
  canFold,
  folded,
  drag,
  onFold,
  onSelect,
  onMenu,
  register,
}: {
  view: SidebarView;
  selected: boolean;
  count: number | null;
  menuOpen: boolean;
  child: boolean;
  canFold: boolean;
  folded: boolean;
  drag: RowDrag | null;
  onFold: () => void;
  onSelect: () => void;
  onMenu: (x: number, y: number, anchor: HTMLElement | null) => void;
  register: (node: HTMLButtonElement | null) => void;
}) {
  const badgeId = useId();
  const tone = drag?.active
    ? "bg-violet-500/20 text-white ring-1 ring-inset ring-violet-400"
    : selected
      ? "bg-slate-800 text-white"
      : "text-slate-300 hover:bg-slate-900";
  const label = sidebarCountLabel(view.name, count, view.key === "bumps_due");
  const described = count != null && formatViewCount(count) != null;
  return (
    <div
      role="group"
      aria-label={label}
      data-view-key={view.key}
      data-drop-target={drag?.active ? "active" : undefined}
      data-dragging={drag?.dragging ? "true" : undefined}
      draggable={drag?.enabled ?? false}
      className={`group relative flex items-center ${drag?.enabled ? "cursor-grab" : ""} ${drag?.dragging ? "opacity-40" : ""}`}
      onContextMenu={(event) => {
        event.preventDefault();
        onMenu(event.clientX, event.clientY, null);
      }}
      onDragStart={drag?.enabled ? drag.onStart : undefined}
      onDragEnd={drag?.enabled ? drag.onEnd : undefined}
      onDragOver={drag ? drag.onOver : undefined}
      onDragLeave={drag ? drag.onLeave : undefined}
      onDrop={drag ? drag.onDrop : undefined}
    >
      {canFold && (
        <button
          type="button"
          aria-expanded={!folded}
          aria-label={`${folded ? "Expand" : "Collapse"} ${view.name}`}
          className="mr-0.5 grid h-6 w-4 shrink-0 place-items-center rounded text-slate-500 hover:bg-slate-800 hover:text-slate-200"
          onClick={onFold}
        >
          {folded ? <ChevronRightIcon size={14} /> : <ChevronDownIcon size={14} />}
        </button>
      )}
      <button
        type="button"
        draggable={drag?.enabled ?? false}
        aria-current={selected ? "page" : undefined}
        aria-describedby={described ? badgeId : undefined}
        className={`flex min-w-0 flex-1 items-center gap-2 rounded-md py-1.5 pr-11 text-left ${canFold || child ? "pl-1" : "pl-2"} ${tone} ${drag?.enabled ? "cursor-grab active:cursor-grabbing" : ""}`}
        onDragStart={
          drag?.enabled
            ? (event) => {
                event.stopPropagation();
                drag.onStart(event);
              }
            : undefined
        }
        onDragEnd={drag?.enabled ? drag.onEnd : undefined}
        onClick={onSelect}
        onKeyDown={(event) => {
          if (opensRowMenu(event)) {
            event.preventDefault();
            const rect = event.currentTarget.getBoundingClientRect();
            onMenu(rect.right, rect.bottom, event.currentTarget);
          }
        }}
      >
        <ViewIcon view={view} />
        <Tooltip label={view.name} wide align="start" className="min-w-0 flex-1">
          <span className="block min-w-0 truncate">{view.name}</span>
        </Tooltip>
      </button>
      <CountSlot
        count={count}
        badgeId={badgeId}
        accent={view.key === "bumps_due"}
        menuOpen={menuOpen}
        menuLabel="View menu"
        register={register}
        onMenu={onMenu}
      />
    </div>
  );
}

function ViewIcon({ view }: { view: SidebarView }) {
  if (view.key === "mine") {
    return <HomeIcon />;
  }
  if (view.key === "to_review") {
    return <ClockIcon />;
  }
  if (view.key === "bumps_due") {
    return <SlackLogo variant="mono" size={16} />;
  }
  return <span aria-hidden="true" className="ml-1 mr-0.5 h-2 w-2 rounded-sm bg-slate-500" />;
}

const ITEM = "flex w-full items-center rounded px-2 py-1.5 text-left text-sm text-slate-200 hover:bg-slate-800";

function ViewMenu({
  view,
  views,
  x,
  y,
  onClose,
  onRename,
  onEdit,
  onDuplicate,
  onDelete,
  onMove,
  onReset,
}: {
  view: SidebarView;
  views: readonly SidebarView[];
  x: number;
  y: number;
  onClose: (restore: boolean) => void;
  onRename: () => void;
  onEdit: () => void;
  onDuplicate: () => void;
  onDelete: () => void;
  onMove: (parent: ParentKey | null) => void;
  onReset: () => void;
}) {
  const menu = useRef<HTMLDivElement>(null);
  const closeRef = useRef(onClose);
  const focusSubmenu = useRef(false);
  const [box, setBox] = useState({ left: x, top: y });
  const [moveOpen, setMoveOpen] = useState(false);
  const here = parentKey(view.parent);

  useEffect(() => {
    closeRef.current = onClose;
  });

  useLayoutEffect(() => {
    const node = menu.current;
    if (!node) {
      return;
    }
    const rect = node.getBoundingClientRect();
    const left = Math.max(8, Math.min(x - rect.width, window.innerWidth - rect.width - 8));
    const top = y + rect.height > window.innerHeight - 8 ? Math.max(8, y - rect.height) : y;
    setBox({ left, top });
  }, [x, y]);

  useEffect(() => {
    if (!moveOpen || !focusSubmenu.current) {
      return;
    }
    focusSubmenu.current = false;
    menu.current?.querySelector<HTMLButtonElement>('[data-submenu="move"] button:not(:disabled)')?.focus();
  }, [moveOpen]);

  useEffect(() => {
    const items = menu.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]');
    items?.[0]?.focus();
    function onPointer(event: MouseEvent) {
      if (!menu.current?.contains(event.target as Node)) {
        closeRef.current(false);
      }
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        closeRef.current(true);
      }
    }
    document.addEventListener("mousedown", onPointer);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onPointer);
      document.removeEventListener("keydown", onKey);
    };
  }, []);

  function onMenuKey(event: ReactKeyboardEvent) {
    const submenu = menu.current?.querySelector('[data-submenu="move"]');
    const inSubmenu = submenu?.contains(document.activeElement) ?? false;
    if (event.key === "ArrowRight" && !inSubmenu && (document.activeElement as HTMLElement | null)?.dataset.move === "true") {
      event.preventDefault();
      focusSubmenu.current = true;
      setMoveOpen(true);
      return;
    }
    if (event.key === "ArrowLeft" && inSubmenu) {
      event.preventDefault();
      setMoveOpen(false);
      menu.current?.querySelector<HTMLButtonElement>("[data-move]")?.focus();
      return;
    }
    const items = [...(menu.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? [])].filter((item) => !item.disabled);
    const index = items.indexOf(document.activeElement as HTMLButtonElement);
    const move = (next: number) => {
      event.preventDefault();
      items[(next + items.length) % items.length]?.focus();
    };
    switch (event.key) {
      case "ArrowDown":
        move(index + 1);
        break;
      case "ArrowUp":
        move(index - 1);
        break;
      case "Home":
        move(0);
        break;
      case "End":
        move(items.length - 1);
        break;
      case "Tab":
        onClose(false);
        break;
      default:
        break;
    }
  }

  function pick(action: () => void) {
    onClose(true);
    action();
  }

  return createPortal(
    <div
      ref={menu}
      role="menu"
      aria-label={`Actions for ${view.name}`}
      style={{ left: box.left, top: box.top }}
      onKeyDown={onMenuKey}
      className="fixed z-40 w-44 rounded-lg border border-slate-700 bg-slate-900 p-1 shadow-xl"
    >
      <button type="button" role="menuitem" tabIndex={-1} className={ITEM} onClick={() => pick(onRename)}>
        Rename
      </button>
      <button type="button" role="menuitem" tabIndex={-1} className={ITEM} onClick={() => pick(onEdit)}>
        Edit filters
      </button>
      <button type="button" role="menuitem" tabIndex={-1} className={ITEM} onClick={() => pick(onDuplicate)}>
        Duplicate
      </button>
      {!view.builtin && (
        <div className="relative" onMouseLeave={() => setMoveOpen(false)}>
          <button
            type="button"
            role="menuitem"
            tabIndex={-1}
            data-move="true"
            aria-haspopup="menu"
            aria-expanded={moveOpen}
            className={ITEM}
            onMouseEnter={() => setMoveOpen(true)}
            onClick={() => setMoveOpen(true)}
          >
            Move to
          </button>
          {moveOpen && (
            <div role="menu" aria-label="Move to" data-submenu="move" className="absolute left-[calc(100%-6px)] top-0 z-10 w-36 rounded-lg border border-slate-700 bg-slate-900 p-1 shadow-xl">
              <MoveDestination label={views.find((item) => item.key === "mine")?.name ?? "My PRs"} current={here === "mine"} onPick={() => pick(() => onMove("mine"))} />
              <MoveDestination label={views.find((item) => item.key === "to_review")?.name ?? "To review"} current={here === "to_review"} onPick={() => pick(() => onMove("to_review"))} />
              <MoveDestination label="Top level" current={here === null} onPick={() => pick(() => onMove(null))} />
            </div>
          )}
        </div>
      )}
      {view.builtin ? (
        <button type="button" role="menuitem" tabIndex={-1} className={ITEM} onClick={() => pick(onReset)}>
          Reset to default
        </button>
      ) : (
        <button type="button" role="menuitem" tabIndex={-1} className={`${ITEM} text-rose-200`} onClick={() => pick(onDelete)}>
          Delete
        </button>
      )}
    </div>,
    document.body,
  );
}

function ViewDialog({
  dialog,
  views,
  onClose,
  onSave,
}: {
  dialog: Dialog;
  views: readonly SidebarView[];
  onClose: () => void;
  onSave: (views: SidebarView[]) => void;
}) {
  const titleId = useId();
  const queryRef = useRef<HTMLInputElement>(null);
  const editing = dialog.kind === "create" ? null : dialog.view;
  const [name, setName] = useState(dialog.kind === "create" ? "" : dialog.view.name);
  const [query, setQuery] = useState(dialog.kind === "create" ? dialog.query : dialog.kind === "edit" ? dialog.view.query : "");
  const [parent, setParent] = useState<ParentKey | "">(
    dialog.kind === "create" ? (dialog.parent ?? "") : dialog.kind === "edit" ? (parentKey(dialog.view.parent) ?? "") : "",
  );
  const [error, setError] = useState<string | null>(null);
  const showQuery = dialog.kind === "create" || dialog.kind === "edit";
  const showName = dialog.kind !== "edit" && dialog.kind !== "delete";
  const showParent = dialog.kind === "create" || (dialog.kind === "edit" && !dialog.view.builtin);
  const parentQuery = parent === "" ? "" : (views.find((view) => view.key === parent)?.query ?? "");
  const preview = parent === "" ? "" : joinRules(parentQuery, query);

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  function submit() {
    if (dialog.kind === "delete") {
      if (builtinKey(dialog.view.key)) {
        return;
      }
      onSave(normalizeCatalog(views.filter((view) => view.key !== dialog.view.key)));
      return;
    }
    const nextName = dialog.kind === "edit" ? dialog.view.name : name;
    const nameError = viewNameError(views, nextName, editing?.key);
    if (nameError) {
      setError(nameError);
      return;
    }
    const nextQuery = dialog.kind === "rename" ? dialog.view.query : query;
    const queryError = viewQueryError(nextQuery);
    if (queryError) {
      setError(queryError);
      return;
    }
    const nextParent = parent === "" ? null : parent;
    if (dialog.kind === "create") {
      onSave(
        normalizeCatalog([
          ...views,
          {
            key: nextCustomKey(views),
            name: nextName.trim(),
            query: nextQuery.trim(),
            builtin: false,
            sort_order: views.length,
            parent: nextParent,
          },
        ]),
      );
      return;
    }
    onSave(
      normalizeCatalog(
        views.map((view) =>
          view.key === dialog.view.key ? { ...view, name: nextName.trim(), query: nextQuery.trim(), parent: view.builtin ? null : nextParent } : view,
        ),
      ),
    );
  }

  const title = dialogTitle(dialog);
  const confirm = dialog.kind === "create" ? "Create view" : dialog.kind === "delete" ? "Delete" : dialog.kind === "rename" ? "Rename" : "Save";

  return createPortal(
    <div className="fixed inset-0 z-40 flex items-start justify-center bg-slate-950/70 px-4 pt-24" onMouseDown={onClose}>
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="w-full max-w-md rounded-lg border border-slate-700 bg-slate-900 p-4 shadow-xl"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <h2 id={titleId} className="text-sm font-medium">
          {title}
        </h2>
        {dialog.kind === "delete" ? (
          <p className="mt-2 text-sm text-slate-300">Delete “{dialog.view.name}”?</p>
        ) : (
          <div className="mt-3 flex flex-col gap-3">
            {showName && (
              <label className="flex flex-col gap-1 text-xs text-slate-400">
                Name
                <input
                  {...plainText}
                  autoFocus
                  value={name}
                  aria-label="Name"
                  className="h-8 rounded-md border border-slate-700 bg-slate-950 px-2 text-sm text-slate-100"
                  onChange={(event) => {
                    setName(event.target.value);
                    setError(null);
                  }}
                />
              </label>
            )}
            {showParent && (
              <label className="flex flex-col gap-1 text-xs text-slate-400">
                Parent
                <select
                  aria-label="Parent"
                  value={parent}
                  className="h-8 rounded-md border border-slate-700 bg-slate-950 px-2 text-sm text-slate-100"
                  onChange={(event) => {
                    const value = event.target.value;
                    setParent(value === "mine" || value === "to_review" ? value : "");
                    setError(null);
                  }}
                >
                  <option value="">None</option>
                  <option value="mine">{views.find((view) => view.key === "mine")?.name ?? "My PRs"}</option>
                  <option value="to_review">{views.find((view) => view.key === "to_review")?.name ?? "To review"}</option>
                </select>
              </label>
            )}
            {showQuery && (
              <div className="flex flex-col gap-1">
                <span className="text-xs text-slate-400">Query</span>
                <SearchField layout="dialog" label="Query" value={query} onChange={(value) => { setQuery(value); setError(null); }} inputRef={queryRef} />
                {preview !== "" && <p className="text-xs text-slate-500">Shows {preview}</p>}
              </div>
            )}
          </div>
        )}
        {error && (
          <p role="alert" className="mt-2 text-xs text-rose-300">
            {error}
          </p>
        )}
        <div className="mt-4 flex justify-end gap-2">
          <button type="button" className="rounded border border-slate-700 px-3 py-1.5 text-sm hover:bg-slate-800" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className={`rounded px-3 py-1.5 text-sm ${dialog.kind === "delete" ? "bg-rose-500 text-white" : "bg-violet-500 text-white"}`}
            onClick={submit}
          >
            {confirm}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}

function MoveDestination({ label, current, onPick }: { label: string; current: boolean; onPick: () => void }) {
  return (
    <button
      type="button"
      role="menuitem"
      tabIndex={-1}
      disabled={current}
      className={`${ITEM} disabled:cursor-default disabled:text-slate-500 disabled:hover:bg-transparent`}
      onClick={onPick}
    >
      {label}
    </button>
  );
}

function MoveConfirm({
  message,
  x,
  y,
  onCancel,
  onMove,
}: {
  message: string;
  x: number;
  y: number;
  onCancel: () => void;
  onMove: () => void;
}) {
  const panel = useRef<HTMLDivElement>(null);
  const [box, setBox] = useState({ left: x, top: y });

  useLayoutEffect(() => {
    const node = panel.current;
    if (!node) {
      return;
    }
    const rect = node.getBoundingClientRect();
    const left = Math.max(8, Math.min(x, window.innerWidth - rect.width - 8));
    const top = y + 8 + rect.height > window.innerHeight - 8 ? Math.max(8, y - rect.height - 8) : y + 8;
    setBox({ left, top });
  }, [x, y, message]);

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        onCancel();
      }
    }
    function onPointer(event: MouseEvent) {
      if (!panel.current?.contains(event.target as Node)) {
        onCancel();
      }
    }
    document.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onPointer);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onPointer);
    };
  }, [onCancel]);

  return createPortal(
    <div
      ref={panel}
      role="dialog"
      aria-label="Move view"
      style={{ left: box.left, top: box.top }}
      className="fixed z-50 w-64 rounded-lg border border-slate-700 bg-slate-900 p-3 text-sm text-slate-200 shadow-xl"
    >
      <p>{message}</p>
      <div className="mt-3 flex justify-end gap-2">
        <button type="button" className="rounded border border-slate-700 px-3 py-1.5 text-sm hover:bg-slate-800" onClick={onCancel}>
          Cancel
        </button>
        <button type="button" className="rounded bg-violet-500 px-3 py-1.5 text-sm text-white" onClick={onMove}>
          Move
        </button>
      </div>
    </div>,
    document.body,
  );
}

/** Replaces the browser's row snapshot. A label follows the pointer instead. */
function hideNativeDragImage(dataTransfer: DataTransfer) {
  if (typeof dataTransfer.setDragImage !== "function") {
    return;
  }
  try {
    const blank = document.createElement("canvas");
    blank.width = 1;
    blank.height = 1;
    dataTransfer.setDragImage(blank, 0, 0);
  } catch {
    // A drag that is not a real browser session has no image to replace.
  }
}

function dialogTitle(dialog: Dialog): string {
  switch (dialog.kind) {
    case "create":
      return "New view";
    case "edit":
      return "Edit filters";
    case "rename":
      return "Rename";
    case "delete":
      return "Delete view";
    default: {
      const unexpected: never = dialog;
      return unexpected;
    }
  }
}
