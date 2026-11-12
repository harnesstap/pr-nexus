import { useEffect, useId, useLayoutEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { ClockIcon, HomeIcon, MoreIcon, PlusIcon, SlackLogo } from "../icons";
import { plainText } from "../textInput";
import { Tooltip } from "../Tooltip";
import { SearchField } from "./SearchField";
import {
  builtinKey,
  defaultCatalog,
  duplicateOf,
  nextCustomKey,
  normalizeCatalog,
  viewNameError,
  viewQueryError,
  type SidebarView,
} from "./viewCatalog";

interface Props {
  views: readonly SidebarView[];
  current: string;
  dueCount: number;
  onSelect: (key: string) => void;
  onChange: (views: SidebarView[]) => void;
  /** Copies the source view's saved Sort, Group, and Filter onto the new key. */
  onDuplicateState?: (from: string, to: string) => void;
}

type Dialog =
  | { kind: "create" }
  | { kind: "edit"; view: SidebarView }
  | { kind: "rename"; view: SidebarView }
  | { kind: "delete"; view: SidebarView };

interface MenuState {
  key: string;
  x: number;
  y: number;
  anchor: HTMLElement | null;
}

/**
 * Default and custom sidebar views. The … menu and a right-click share one
 * menu. Built-in views reset instead of deleting.
 */
export function SidebarViews({ views, current, dueCount, onSelect, onChange, onDuplicateState }: Props) {
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const anchors = useRef(new Map<string, HTMLButtonElement>());

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

  const active = views.find((view) => view.key === menu?.key) ?? null;

  return (
    <nav aria-label="Views" className="flex flex-col gap-0.5 px-2 text-sm">
      {views.map((view) => (
        <ViewRow
          key={view.key}
          view={view}
          selected={current === view.key}
          badge={view.key === "bumps_due" ? dueCount : 0}
          menuOpen={menu?.key === view.key}
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
      ))}
      <Tooltip label="New view" detail="Save a name and a query" align="end">
        <button
          type="button"
          aria-label="New view"
          className="mt-0.5 flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-slate-400 hover:bg-slate-900 hover:text-slate-200"
          onClick={() => {
            setMenu(null);
            setDialog({ kind: "create" });
          }}
        >
          <PlusIcon />
          New view
        </button>
      </Tooltip>
      {menu && active && (
        <ViewMenu
          view={active}
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
          onReset={() => {
            const fallback = defaultCatalog().find((item) => item.key === active.key);
            if (!fallback) {
              return;
            }
            onChange(views.map((item) => (item.key === active.key ? { ...item, name: fallback.name, query: fallback.query } : item)));
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
    </nav>
  );
}

function ViewRow({
  view,
  selected,
  badge,
  menuOpen,
  onSelect,
  onMenu,
  register,
}: {
  view: SidebarView;
  selected: boolean;
  badge: number;
  menuOpen: boolean;
  onSelect: () => void;
  onMenu: (x: number, y: number, anchor: HTMLElement | null) => void;
  register: (node: HTMLButtonElement | null) => void;
}) {
  const tone = selected ? "bg-slate-800 text-white" : "text-slate-300 hover:bg-slate-900";
  return (
    <div
      role="group"
      aria-label={badge > 0 ? `${view.name}, ${badge} due` : view.name}
      className="group flex items-center gap-0.5"
      onContextMenu={(event) => {
        event.preventDefault();
        onMenu(event.clientX, event.clientY, null);
      }}
    >
      <button
        type="button"
        aria-current={selected ? "page" : undefined}
        className={`flex min-w-0 flex-1 items-center gap-2 rounded-md px-2 py-1.5 text-left ${tone}`}
        onClick={onSelect}
        onKeyDown={(event) => {
          if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) {
            event.preventDefault();
            const rect = event.currentTarget.getBoundingClientRect();
            onMenu(rect.right, rect.bottom, event.currentTarget);
          }
        }}
      >
        <ViewIcon view={view} />
        <span className="min-w-0 flex-1 truncate">{view.name}</span>
        {badge > 0 && (
          <span aria-hidden="true" className="rounded-full bg-slate-700 px-1.5 text-xs tabular-nums text-slate-200">
            {badge}
          </span>
        )}
      </button>
      <button
        ref={register}
        type="button"
        aria-label="View menu"
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        className={`rounded p-1 text-slate-500 hover:bg-slate-800 hover:text-slate-200 ${
          menuOpen ? "opacity-100" : "opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
        }`}
        onClick={(event) => {
          const rect = event.currentTarget.getBoundingClientRect();
          onMenu(rect.right, rect.bottom, event.currentTarget);
        }}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp" || event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) {
            event.preventDefault();
            const rect = event.currentTarget.getBoundingClientRect();
            onMenu(rect.right, rect.bottom, event.currentTarget);
          }
        }}
      >
        <MoreIcon size={14} />
      </button>
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
  x,
  y,
  onClose,
  onRename,
  onEdit,
  onDuplicate,
  onDelete,
  onReset,
}: {
  view: SidebarView;
  x: number;
  y: number;
  onClose: (restore: boolean) => void;
  onRename: () => void;
  onEdit: () => void;
  onDuplicate: () => void;
  onDelete: () => void;
  onReset: () => void;
}) {
  const menu = useRef<HTMLDivElement>(null);
  const closeRef = useRef(onClose);
  const [box, setBox] = useState({ left: x, top: y });

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
    const items = [...(menu.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? [])];
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
  const [query, setQuery] = useState(dialog.kind === "edit" || dialog.kind === "create" ? (dialog.kind === "create" ? "" : dialog.view.query) : "");
  const [error, setError] = useState<string | null>(null);
  const showQuery = dialog.kind === "create" || dialog.kind === "edit";
  const showName = dialog.kind !== "edit" && dialog.kind !== "delete";

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
    if (dialog.kind === "create") {
      onSave(
        normalizeCatalog([
          ...views,
          { key: nextCustomKey(views), name: nextName.trim(), query: nextQuery.trim(), builtin: false, sort_order: views.length },
        ]),
      );
      return;
    }
    onSave(
      normalizeCatalog(views.map((view) => (view.key === dialog.view.key ? { ...view, name: nextName.trim(), query: nextQuery.trim() } : view))),
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
            {showQuery && (
              <div className="flex flex-col gap-1">
                <span className="text-xs text-slate-400">Query</span>
                <SearchField layout="dialog" label="Query" value={query} onChange={(value) => { setQuery(value); setError(null); }} inputRef={queryRef} />
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
