import { useEffect, useRef, useState, type ReactElement } from "react";
import { AlertIcon, CheckIcon, CloseIcon, DeclinedIcon, DraftIcon, EyeOffIcon, MergeIcon, MoreIcon, StarIcon } from "../icons";
import { Tooltip } from "../Tooltip";
import { QUICK_FILTERS, type QuickFilter } from "./model";
import { hint } from "./shortcuts";

interface QuickSpec {
  /** Short text on the chip. */
  chip: string;
  /** Tooltip title. */
  label: string;
  detail: string;
  icon: ReactElement;
  /** Tailwind classes for the chip when it is on. */
  tone: string;
}

export const QUICK_SPECS: Record<QuickFilter, QuickSpec> = {
  draft: {
    chip: "Draft",
    label: "Draft",
    detail: "Show only draft PRs",
    icon: <DraftIcon size={14} />,
    tone: "border-slate-300 bg-slate-700 text-white",
  },
  no_review: {
    chip: "No review",
    label: "No review requested",
    detail: "Open PRs nobody has been asked to review yet, on GitHub or Slack",
    icon: <EyeOffIcon size={14} />,
    tone: "border-sky-400 bg-sky-500/20 text-sky-100",
  },
  ci_trouble: {
    chip: "CI or conflicts",
    label: "CI errors or conflicts",
    detail: "PRs with failing checks or merge conflicts",
    icon: <AlertIcon size={14} />,
    tone: "border-amber-400 bg-amber-500/20 text-amber-100",
  },
  declined: {
    chip: "Changes requested",
    label: "Review declined",
    detail: "PRs where a reviewer requested changes",
    icon: <DeclinedIcon size={14} />,
    tone: "border-rose-400 bg-rose-500/20 text-rose-100",
  },
  ready: {
    chip: "Ready",
    label: "Ready to merge",
    detail: "Approved, checks green and no conflicts",
    icon: <MergeIcon size={14} />,
    tone: "border-emerald-400 bg-emerald-500/20 text-emerald-100",
  },
};

/** Chips from this index on fold into **More** on a narrow list (unless they are on). */
export const CHIPS_ALWAYS_SHOWN = 3;

type ChipKey = QuickFilter | "starred";

interface Props {
  active: QuickFilter | null;
  /** Rows each filter would leave, from the backend; null while the first page loads. */
  counts: Record<QuickFilter, number> | null;
  onChange: (quick: QuickFilter | null) => void;
  starred: boolean;
  starredCount: number | null;
  onStarredChange: (on: boolean) => void;
}

const BASE = "flex h-7 shrink-0 cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-full border px-2.5 text-xs transition-colors";
const IDLE = "border-slate-700 bg-slate-900 text-slate-300 hover:border-slate-500 hover:text-white";

function Count({ count, on }: { count: number | null | undefined; on: boolean }) {
  if (count === null || count === undefined) {
    return null;
  }
  return (
    <span
      aria-hidden="true"
      className={`min-w-4 rounded-full px-1 text-center tabular-nums ${on ? "bg-black/25" : count === 0 ? "text-slate-600" : "bg-slate-800 text-slate-200"}`}
    >
      {count}
    </span>
  );
}

/**
 * Quick status filters (§8.2) as labelled chips: one quick filter at a time (clicking the
 * active one again clears it), plus Starred, which stacks with them. Counts cover the current
 * search and menu filters, so a chip says what clicking leaves. On a narrow list the later
 * chips move into a **More** menu; a chip that is on always stays visible.
 */
export function QuickFilters({ active, counts, onChange, starred, starredCount, onStarredChange }: Props) {
  const keys: ChipKey[] = [...QUICK_FILTERS, "starred"];
  const isOn = (key: ChipKey) => (key === "starred" ? starred : active === key);
  const toggle = (key: ChipKey) => (key === "starred" ? onStarredChange(!starred) : onChange(active === key ? null : key));
  const folded = keys.filter((key, index) => index >= CHIPS_ALWAYS_SHOWN && !isOn(key));
  const anyOn = active !== null || starred;
  return (
    <div role="group" aria-label="Quick filters" className="@container flex min-w-0 flex-1 flex-wrap items-center gap-1.5">
      {keys.map((key, index) => {
        const on = isOn(key);
        const foldable = index >= CHIPS_ALWAYS_SHOWN && !on;
        return <Chip key={key} chipKey={key} on={on} counts={counts} starredCount={starredCount} className={foldable ? "hidden @xl:flex" : ""} onToggle={() => toggle(key)} />;
      })}
      {folded.length > 0 && <MoreChips keys={folded} counts={counts} starredCount={starredCount} onToggle={toggle} />}
      {anyOn && (
        <Tooltip label="Clear quick filters" detail="Turn off every chip; menu filters stay">
          <button
            type="button"
            aria-label="Clear quick filters"
            className="flex h-7 shrink-0 cursor-pointer items-center gap-1 whitespace-nowrap rounded-full px-2 text-xs text-violet-300 hover:bg-slate-800 hover:text-violet-200"
            onClick={() => {
              onChange(null);
              if (starred) {
                onStarredChange(false);
              }
            }}
          >
            <CloseIcon size={12} />
            Clear
          </button>
        </Tooltip>
      )}
    </div>
  );
}

function chipText(key: ChipKey): { chip: string; label: string; detail: string; icon: ReactElement; tone: string } {
  if (key === "starred") {
    return { chip: "Starred", label: "Starred only", detail: "Show only the PRs you starred", icon: <StarIcon size={14} />, tone: "border-amber-400 bg-amber-500/20 text-amber-100" };
  }
  return QUICK_SPECS[key];
}

function Chip({
  chipKey,
  on,
  counts,
  starredCount,
  className,
  onToggle,
}: {
  chipKey: ChipKey;
  on: boolean;
  counts: Record<QuickFilter, number> | null;
  starredCount: number | null;
  className: string;
  onToggle: () => void;
}) {
  const spec = chipText(chipKey);
  const count = chipKey === "starred" ? starredCount : counts?.[chipKey];
  const icon = chipKey === "starred" ? <StarIcon size={14} filled={on} /> : spec.icon;
  return (
    <Tooltip
      label={spec.label}
      shortcut={chipKey === "starred" ? hint("star") : undefined}
      detail={on ? "Click again to show every PR" : spec.detail}
      align="start"
    >
      <button
        type="button"
        aria-label={count === null || count === undefined ? spec.chip : `${spec.chip} (${count})`}
        aria-pressed={on}
        className={`${BASE} ${on ? `${spec.tone} font-medium` : IDLE} ${className}`}
        onClick={onToggle}
      >
        {on ? <CheckIcon size={14} /> : icon}
        {spec.chip}
        <Count count={count} on={on} />
      </button>
    </Tooltip>
  );
}

function MoreChips({
  keys,
  counts,
  starredCount,
  onToggle,
}: {
  keys: ChipKey[];
  counts: Record<QuickFilter, number> | null;
  starredCount: number | null;
  onToggle: (key: ChipKey) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) {
      return;
    }
    function onPointer(event: MouseEvent) {
      if (ref.current && event.target instanceof Node && !ref.current.contains(event.target)) {
        setOpen(false);
      }
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.stopPropagation();
        setOpen(false);
      }
    }
    document.addEventListener("mousedown", onPointer);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onPointer);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);
  return (
    <div ref={ref} className="relative @xl:hidden">
      <Tooltip label="More quick filters" detail={keys.map((key) => chipText(key).chip).join(", ")}>
        <button
          type="button"
          aria-label={`More quick filters (${keys.length})`}
          aria-haspopup="menu"
          aria-expanded={open}
          className={`${BASE} ${IDLE}`}
          onClick={() => setOpen(!open)}
        >
          <MoreIcon size={14} />
          More
        </button>
      </Tooltip>
      {open && (
        <div role="menu" aria-label="More quick filters" className="absolute right-0 top-8 z-30 min-w-48 rounded-md border border-slate-700 bg-slate-900 p-1 shadow-xl">
          {keys.map((key) => {
            const spec = chipText(key);
            const count = key === "starred" ? starredCount : counts?.[key];
            return (
              <button
                key={key}
                type="button"
                role="menuitemcheckbox"
                aria-checked={false}
                className="flex w-full cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-left text-xs text-slate-200 hover:bg-slate-800"
                onClick={() => {
                  setOpen(false);
                  onToggle(key);
                }}
              >
                {spec.icon}
                <span className="flex-1">{spec.chip}</span>
                {count !== null && count !== undefined && <span className="tabular-nums text-slate-400">{count}</span>}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
