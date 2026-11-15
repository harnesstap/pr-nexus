import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import { MoreIcon } from "../icons";
import { formatViewCount } from "./viewCount";

const MUTED_PILL = "rounded-full bg-slate-700 px-1.5 text-xs tabular-nums text-slate-400";
const ACCENT_PILL = "rounded-full bg-slate-700 px-1.5 text-xs tabular-nums text-slate-200";

/**
 * The count and the … control share one slot at the end of a sidebar row.
 * The count is blank at zero. The control is hover-only, and it replaces the
 * count while the row is hovered, the control has focus, or its menu is open.
 */
export function CountSlot({
  count,
  badgeId,
  accent = false,
  menuOpen,
  menuLabel,
  register,
  onMenu,
}: {
  count: number | null;
  badgeId: string;
  /** Bumps due keeps the stronger pill. Project and view counts stay muted. */
  accent?: boolean;
  menuOpen: boolean;
  menuLabel: string;
  register?: (node: HTMLButtonElement | null) => void;
  onMenu: (x: number, y: number, anchor: HTMLElement | null) => void;
}) {
  const text = count == null ? null : formatViewCount(count);
  const pill = accent ? ACCENT_PILL : MUTED_PILL;
  return (
    <div className="group/slot pointer-events-none absolute right-1 top-1/2 grid h-6 w-9 -translate-y-1/2 place-items-center">
      {text && (
        <span
          id={badgeId}
          className={`${pill} pointer-events-none col-start-1 row-start-1 ${menuOpen ? "invisible" : "group-hover:invisible group-focus-within/slot:invisible"}`}
        >
          {text}
        </span>
      )}
      <button
        ref={register}
        type="button"
        aria-label={menuLabel}
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        className={`absolute inset-0 grid place-items-center rounded text-slate-500 hover:bg-slate-800 hover:text-slate-200 ${
          menuOpen
            ? "pointer-events-auto opacity-100"
            : "pointer-events-none opacity-0 group-hover:pointer-events-auto group-hover:opacity-100 focus-visible:pointer-events-auto focus-visible:opacity-100"
        }`}
        onClick={(event) => {
          const rect = event.currentTarget.getBoundingClientRect();
          onMenu(rect.right, rect.bottom, event.currentTarget);
        }}
        onKeyDown={(event) => {
          if (opensSlotMenu(event)) {
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

function opensSlotMenu(event: ReactKeyboardEvent): boolean {
  return event.key === "ArrowDown" || event.key === "ArrowUp" || opensRowMenu(event);
}

/** Context menu key on the row itself, matching a right-click. */
export function opensRowMenu(event: ReactKeyboardEvent): boolean {
  return event.key === "ContextMenu" || (event.shiftKey && event.key === "F10");
}
