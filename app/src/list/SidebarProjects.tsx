import { useEffect, useId, useLayoutEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { Tooltip } from "../Tooltip";
import { CountSlot, opensRowMenu } from "./CountBadge";
import { DRAG_MIME, parseDragIds } from "./model";
import { formatViewCount, sidebarCountLabel } from "./viewCount";

const ITEM = "flex w-full items-center rounded px-2 py-1.5 text-left text-sm text-slate-200 hover:bg-slate-800";

/**
 * One sidebar project. The count uses the same slot as a view row: the …
 * menu replaces it on hover, on the control's focus, and while the menu is open.
 */
export function ProjectRow({
  name,
  color,
  selected,
  count,
  menuOpen,
  flash = false,
  projectId,
  onSelect,
  onMenu,
  onDropPrs,
  register,
}: {
  name: string;
  color: string;
  selected: boolean;
  count: number | null;
  menuOpen: boolean;
  flash?: boolean;
  projectId: number;
  onSelect: () => void;
  onMenu: (x: number, y: number, anchor: HTMLElement | null) => void;
  onDropPrs: (ids: number[]) => void;
  register?: (node: HTMLButtonElement | null) => void;
}) {
  const badgeId = useId();
  const [over, setOver] = useState(false);
  const tone = over || flash ? "bg-violet-500/20 text-white ring-1 ring-violet-400" : selected ? "bg-slate-800 text-white" : "text-slate-300 hover:bg-slate-900";
  const described = count != null && formatViewCount(count) != null;
  return (
    <div
      role="group"
      aria-label={sidebarCountLabel(name, count)}
      className="group relative flex w-full min-w-0 items-center"
      onContextMenu={(event) => {
        event.preventDefault();
        onMenu(event.clientX, event.clientY, null);
      }}
      onDragOver={(event) => {
        if (event.dataTransfer.types.includes(DRAG_MIME)) {
          event.preventDefault();
          event.dataTransfer.dropEffect = "link";
          setOver(true);
        }
      }}
      onDragLeave={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
          setOver(false);
        }
      }}
      onDrop={(event) => {
        setOver(false);
        const ids = parseDragIds(event.dataTransfer.getData(DRAG_MIME));
        if (ids.length > 0) {
          event.preventDefault();
          onDropPrs(ids);
        }
      }}
    >
      <button
        type="button"
        aria-current={selected ? "page" : undefined}
        aria-describedby={described ? badgeId : undefined}
        data-project-id={projectId}
        className={`flex min-w-0 flex-1 items-center gap-2 rounded-md py-1.5 pl-2 pr-11 text-left ${tone}`}
        onClick={onSelect}
        onKeyDown={(event) => {
          if (opensRowMenu(event)) {
            event.preventDefault();
            const rect = event.currentTarget.getBoundingClientRect();
            onMenu(rect.right, rect.bottom, event.currentTarget);
          }
        }}
      >
        <span className="flex w-4 shrink-0 justify-center">
          <span aria-hidden="true" className="ml-1 mr-0.5 h-2 w-2 rounded-sm" style={{ background: color }} />
        </span>
        <Tooltip label={name} wide align="start" className="min-w-0 flex-1">
          <span className="block min-w-0 truncate">{name}</span>
        </Tooltip>
      </button>
      <CountSlot count={count} badgeId={badgeId} menuOpen={menuOpen} menuLabel="Project menu" register={register} onMenu={onMenu} />
    </div>
  );
}

export function ProjectMenu({
  name,
  x,
  y,
  onClose,
  onEdit,
  onArchive,
  onDelete,
}: {
  name: string;
  x: number;
  y: number;
  onClose: (restore: boolean) => void;
  onEdit: () => void;
  onArchive: () => void;
  onDelete: () => void;
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
      aria-label={`Actions for ${name}`}
      style={{ left: box.left, top: box.top }}
      onKeyDown={onMenuKey}
      className="fixed z-40 w-48 rounded-lg border border-slate-700 bg-slate-900 p-1 shadow-xl"
    >
      <button type="button" role="menuitem" tabIndex={-1} className={ITEM} onClick={() => pick(onEdit)}>
        Edit auto-link rules
      </button>
      <button type="button" role="menuitem" tabIndex={-1} className={ITEM} onClick={() => pick(onArchive)}>
        Archive project
      </button>
      <button type="button" role="menuitem" tabIndex={-1} className={`${ITEM} text-rose-200`} onClick={() => pick(onDelete)}>
        Delete project
      </button>
    </div>,
    document.body,
  );
}

export function DeleteProjectDialog({ name, onCancel, onConfirm }: { name: string; onCancel: () => void; onConfirm: () => void }) {
  const titleId = useId();
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        onCancel();
      }
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onCancel]);

  return createPortal(
    <div className="fixed inset-0 z-40 flex items-start justify-center bg-slate-950/70 px-4 pt-24" onMouseDown={onCancel}>
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="w-full max-w-md rounded-lg border border-slate-700 bg-slate-900 p-4 shadow-xl"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <p id={titleId} className="text-sm text-slate-200">
          Delete {name}? This cannot be undone.
        </p>
        <div className="mt-4 flex justify-end gap-2">
          <button type="button" className="rounded border border-slate-700 px-3 py-1.5 text-sm hover:bg-slate-800" onClick={onCancel}>
            Cancel
          </button>
          <button type="button" className="rounded bg-rose-500 px-3 py-1.5 text-sm text-white" onClick={onConfirm}>
            Delete
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
