import type { KeyboardEvent as ReactKeyboardEvent, ReactNode } from "react";
import { ChevronDownIcon, ChevronRightIcon } from "../icons";

/**
 * A Views or Projects header. The label toggles the section. The chevron
 * sits on the icon: it shows on hover, and it stays visible while collapsed.
 * The trailing control (New view, New project) is not part of the toggle.
 */
export function SectionHeader({
  id,
  icon,
  label,
  collapsed,
  hint,
  controls,
  onToggle,
}: {
  id: string;
  icon: ReactNode;
  label: string;
  collapsed: boolean;
  hint: string | null;
  controls: ReactNode;
  onToggle: () => void;
}) {
  function onKeyDown(event: ReactKeyboardEvent) {
    if (event.key !== "Enter" && event.key !== " ") {
      return;
    }
    event.preventDefault();
    onToggle();
  }
  return (
    <div className="group/section flex items-center justify-between px-2 pb-1">
      <h2 id={id} className="min-w-0 text-xs font-medium uppercase tracking-wide text-slate-500">
        <button
          type="button"
          aria-expanded={!collapsed}
          className="flex min-w-0 items-center gap-2 rounded text-left hover:text-slate-300"
          onClick={onToggle}
          onKeyDown={onKeyDown}
        >
          <span className="relative grid h-3.5 w-3.5 shrink-0 place-items-center">
            <span className={collapsed ? "invisible" : "group-hover/section:invisible"}>{icon}</span>
            <span className={`absolute ${collapsed ? "" : "invisible group-hover/section:visible"}`}>
              {collapsed ? <ChevronRightIcon size={14} /> : <ChevronDownIcon size={14} />}
            </span>
          </span>
          {label}
          {collapsed && hint && (
            <span aria-hidden="true" className="rounded-full bg-slate-800 px-1.5 text-[11px] font-normal normal-case tracking-normal text-slate-400">
              {hint}
            </span>
          )}
        </button>
      </h2>
      {controls}
    </div>
  );
}
