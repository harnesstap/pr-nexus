import { useEffect, useLayoutEffect, useRef, useState, type ReactElement } from "react";
import { AlertIcon, CheckIcon, ClockIcon, DeclinedIcon, DraftIcon, EyeOffIcon, MergeIcon, MoreIcon, StarIcon } from "../icons";
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
  /** Tailwind text colour for the idle icon, so each chip keeps its status hue. */
  tint: string;
}

export const QUICK_SPECS: Record<QuickFilter, QuickSpec> = {
  draft: {
    chip: "Draft",
    label: "Draft",
    detail: "Show only draft PRs",
    icon: <DraftIcon size={12} />,
    tint: "text-slate-400",
  },
  no_review: {
    chip: "No review",
    label: "No review requested",
    detail: "Open PRs nobody has been asked to review yet, on GitHub or Slack",
    icon: <EyeOffIcon size={12} />,
    tint: "text-sky-400",
  },
  ci_trouble: {
    chip: "CI or conflicts",
    label: "CI errors or conflicts",
    detail: "PRs with failing checks or merge conflicts",
    icon: <AlertIcon size={12} />,
    tint: "text-amber-400",
  },
  declined: {
    chip: "Changes requested",
    label: "Review declined",
    detail: "PRs where a reviewer requested changes",
    icon: <DeclinedIcon size={12} />,
    tint: "text-rose-400",
  },
  ready: {
    chip: "Ready",
    label: "Ready to merge",
    detail: "Approved, checks green and no conflicts",
    icon: <MergeIcon size={12} />,
    tint: "text-emerald-400",
  },
};

type ChipKey = QuickFilter | "starred" | "unread" | "snoozed";

const KEYS: ChipKey[] = [...QUICK_FILTERS, "starred", "unread", "snoozed"];
/** Gap between chips, in px; matches `gap-1`. */
const GAP = 4;

interface Props {
  active: QuickFilter | null;
  /** Rows each filter would leave, from the backend; null while the first page loads. */
  counts: Record<QuickFilter, number> | null;
  onChange: (quick: QuickFilter | null) => void;
  starred: boolean;
  starredCount: number | null;
  onStarredChange: (on: boolean) => void;
  unread: boolean;
  unreadCount: number | null;
  onUnreadChange: (on: boolean) => void;
  snoozed: boolean;
  snoozedCount: number | null;
  onSnoozedChange: (on: boolean) => void;
}

// Same 28px height and rounded-md as the toolbar's inputs, a hairline border, and the accent only when on.
const BASE =
  "flex h-7 shrink-0 cursor-pointer items-center gap-1 whitespace-nowrap rounded-md border px-1.5 text-xs transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-violet-400";
const IDLE = "border-slate-800 bg-transparent text-slate-300 hover:border-slate-600 hover:bg-slate-900 hover:text-white";
const ON = "border-violet-500/50 bg-violet-500/15 text-violet-100 hover:bg-violet-500/25";
const EMPTY = "opacity-60";

function Count({ count, on }: { count: number | null | undefined; on: boolean }) {
  if (count === null || count === undefined) {
    return null;
  }
  return (
    <span aria-hidden="true" className={`text-[11px] tabular-nums ${on ? "text-violet-300" : "text-slate-500"}`}>
      {count}
    </span>
  );
}

/**
 * Which chips fit in `available` px, in toolbar order. Active chips are placed first so they
 * stay visible; the rest fill in order until one does not fit, and everything after it folds
 * into More (which needs `moreWidth` px once anything folds).
 */
export function fitChips(keys: readonly ChipKey[], widths: readonly number[], available: number, moreWidth: number, isOn: (key: ChipKey) => boolean): Set<ChipKey> {
  const total = widths.reduce((sum, width) => sum + width, 0) + GAP * Math.max(0, widths.length - 1);
  if (total <= available) {
    return new Set(keys);
  }
  let budget = available - moreWidth - GAP;
  const shown = new Set<ChipKey>();
  const take = (index: number) => {
    const cost = widths[index] + GAP;
    if (cost > budget) {
      return false;
    }
    budget -= cost;
    shown.add(keys[index]);
    return true;
  };
  keys.forEach((key, index) => isOn(key) && take(index));
  for (let index = 0; index < keys.length; index += 1) {
    if (!isOn(keys[index]) && !take(index)) {
      break;
    }
  }
  return shown;
}

/**
 * Quick status filters (§8.2) as compact chips inside the toolbar row: one quick filter at a
 * time (clicking the active one again clears it), plus Starred, Unread and Snoozed, which stack with them. Counts
 * cover the current search and menu filters, so a chip says what clicking leaves; a chip with
 * nothing to show is muted. Chips that do not fit the row's width fold into **More (N)**, with
 * active chips kept visible first. Reset in the filter menu turns them off.
 */
export function QuickFilters({ active, counts, onChange, starred, starredCount, onStarredChange, unread, unreadCount, onUnreadChange, snoozed, snoozedCount, onSnoozedChange }: Props) {
  const isOn = (key: ChipKey) => stackOn(key, { starred, unread, snoozed, active });
  const toggle = (key: ChipKey) => {
    if (key === "starred") {
      onStarredChange(!starred);
    } else if (key === "unread") {
      onUnreadChange(!unread);
    } else if (key === "snoozed") {
      onSnoozedChange(!snoozed);
    } else {
      onChange(active === key ? null : key);
    }
  };
  const countOf = (key: ChipKey) => stackCount(key, { starredCount, unreadCount, snoozedCount, counts });
  const box = useRef<HTMLDivElement>(null);
  const ruler = useRef<HTMLDivElement>(null);
  const [shown, setShown] = useState<Set<ChipKey>>(() => new Set(KEYS));
  const onKey = KEYS.filter(isOn).join(",");

  useLayoutEffect(() => {
    const container = box.current;
    const measure = ruler.current;
    if (!container || !measure) {
      return;
    }
    const fit = () => {
      // Not laid out yet (hidden, or a test DOM): show every chip rather than folding them all.
      if (container.clientWidth <= 0) {
        setShown((prev) => (prev.size === KEYS.length ? prev : new Set(KEYS)));
        return;
      }
      const items = Array.from(measure.children) as HTMLElement[];
      const widths = items.slice(0, KEYS.length).map((item) => item.offsetWidth);
      const moreWidth = items[KEYS.length]?.offsetWidth ?? 0;
      const next = fitChips(KEYS, widths, container.clientWidth, moreWidth, (key) => onKey.split(",").includes(key));
      setShown((prev) => (prev.size === next.size && [...next].every((key) => prev.has(key)) ? prev : next));
    };
    fit();
    if (typeof ResizeObserver === "undefined") {
      return;
    }
    const observer = new ResizeObserver(fit);
    observer.observe(container);
    observer.observe(measure);
    return () => observer.disconnect();
  }, [onKey]);

  const folded = KEYS.filter((key) => !shown.has(key));
  return (
    <div ref={box} role="group" aria-label="Quick filters" className="relative flex h-8 min-w-20 flex-1 flex-nowrap items-center gap-1 overflow-visible">
      {/* A zero-size clip so the measuring row cannot widen the toolbar's scroll width. */}
      <div className="pointer-events-none absolute left-0 top-0 h-0 w-0 overflow-hidden" inert>
        <div ref={ruler} aria-hidden="true" className="invisible flex w-max gap-1">
          {KEYS.map((key) => (
            <ChipFace key={key} chipKey={key} on={isOn(key)} count={countOf(key)} />
          ))}
          <span className={`${BASE} ${IDLE}`}>
            <MoreIcon size={12} />
            More ({KEYS.length})
          </span>
        </div>
      </div>
      {KEYS.filter((key) => shown.has(key)).map((key) => (
        <Chip key={key} chipKey={key} on={isOn(key)} count={countOf(key)} onToggle={() => toggle(key)} />
      ))}
      {folded.length > 0 && <MoreChips keys={folded} countOf={countOf} isOn={isOn} onToggle={toggle} />}
    </div>
  );
}

function stackOn(key: ChipKey, state: { starred: boolean; unread: boolean; snoozed: boolean; active: QuickFilter | null }): boolean {
  switch (key) {
    case "starred":
      return state.starred;
    case "unread":
      return state.unread;
    case "snoozed":
      return state.snoozed;
    case "draft":
    case "no_review":
    case "ci_trouble":
    case "declined":
    case "ready":
      return state.active === key;
    default: {
      const unexpected: never = key;
      return unexpected;
    }
  }
}

function stackCount(
  key: ChipKey,
  counts: { starredCount: number | null; unreadCount: number | null; snoozedCount: number | null; counts: Record<QuickFilter, number> | null },
): number | null | undefined {
  switch (key) {
    case "starred":
      return counts.starredCount;
    case "unread":
      return counts.unreadCount;
    case "snoozed":
      return counts.snoozedCount;
    case "draft":
    case "no_review":
    case "ci_trouble":
    case "declined":
    case "ready":
      return counts.counts?.[key];
    default: {
      const unexpected: never = key;
      return unexpected;
    }
  }
}

function chipText(key: ChipKey): QuickSpec {
  switch (key) {
    case "starred":
      return { chip: "Starred", label: "Starred only", detail: "Show only the PRs you starred", icon: <StarIcon size={12} />, tint: "text-amber-400" };
    case "unread":
      return {
        chip: "Unread",
        label: "Unread only",
        detail: "PRs with new commits, comments, reviews, or CI since you last opened them",
        icon: <span className="inline-block h-1.5 w-1.5 rounded-full bg-violet-400" />,
        tint: "text-violet-400",
      };
    case "snoozed":
      return { chip: "Snoozed", label: "Snoozed only", detail: "PRs you hid until a time or the next update", icon: <ClockIcon size={12} />, tint: "text-slate-400" };
    case "draft":
    case "no_review":
    case "ci_trouble":
    case "declined":
    case "ready":
      return QUICK_SPECS[key];
    default: {
      const unexpected: never = key;
      return unexpected;
    }
  }
}

function chipIcon(key: ChipKey, spec: QuickSpec, on: boolean): ReactElement {
  if (key === "starred") {
    return <StarIcon size={12} filled={on} />;
  }
  return spec.icon;
}

function ChipFace({ chipKey, on, count }: { chipKey: ChipKey; on: boolean; count: number | null | undefined }) {
  const spec = chipText(chipKey);
  const icon = chipIcon(chipKey, spec, on);
  return (
    <span className={`${BASE} ${on ? `${ON} font-medium` : IDLE} ${!on && count === 0 ? EMPTY : ""}`}>
      <span className={`flex shrink-0 ${on ? "text-violet-300" : spec.tint}`}>{on ? <CheckIcon size={12} /> : icon}</span>
      {spec.chip}
      <Count count={count} on={on} />
    </span>
  );
}

function Chip({ chipKey, on, count, onToggle }: { chipKey: ChipKey; on: boolean; count: number | null | undefined; onToggle: () => void }) {
  const spec = chipText(chipKey);
  const icon = chipIcon(chipKey, spec, on);
  return (
    <Tooltip
      label={spec.label}
      shortcut={chipKey === "starred" ? hint("star") : undefined}
      detail={on ? "Click again to show every PR" : count === 0 ? `${spec.detail}. None right now` : spec.detail}
      align="start"
    >
      <button
        type="button"
        aria-label={count === null || count === undefined ? spec.chip : `${spec.chip} (${count})`}
        aria-pressed={on}
        className={`${BASE} ${on ? `${ON} font-medium` : IDLE} ${!on && count === 0 ? EMPTY : ""}`}
        onClick={onToggle}
      >
        <span className={`flex shrink-0 ${on ? "text-violet-300" : spec.tint}`}>{on ? <CheckIcon size={12} /> : icon}</span>
        {spec.chip}
        <Count count={count} on={on} />
      </button>
    </Tooltip>
  );
}

function MoreChips({
  keys,
  countOf,
  isOn,
  onToggle,
}: {
  keys: ChipKey[];
  countOf: (key: ChipKey) => number | null | undefined;
  isOn: (key: ChipKey) => boolean;
  onToggle: (key: ChipKey) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const anyOn = keys.some(isOn);
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
    <div ref={ref} className="relative shrink-0">
      <Tooltip label="More quick filters" detail={keys.map((key) => chipText(key).chip).join(", ")}>
        <button
          type="button"
          aria-label={`More quick filters (${keys.length})`}
          aria-haspopup="menu"
          aria-expanded={open}
          className={`${BASE} ${anyOn ? ON : IDLE}`}
          onClick={() => setOpen(!open)}
        >
          <MoreIcon size={12} />
          More ({keys.length})
        </button>
      </Tooltip>
      {open && (
        <div role="menu" aria-label="More quick filters" className="absolute right-0 top-8 z-30 min-w-48 rounded-md border border-slate-700 bg-slate-900 p-1 shadow-xl">
          {keys.map((key) => {
            const spec = chipText(key);
            const count = countOf(key);
            const on = isOn(key);
            return (
              <button
                key={key}
                type="button"
                role="menuitemcheckbox"
                aria-checked={on}
                className={`flex w-full cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-left text-xs hover:bg-slate-800 ${on ? "text-violet-100" : "text-slate-200"} ${!on && count === 0 ? EMPTY : ""}`}
                onClick={() => {
                  setOpen(false);
                  onToggle(key);
                }}
              >
                <span className={`flex shrink-0 ${on ? "text-violet-300" : spec.tint}`}>{on ? <CheckIcon size={12} /> : spec.icon}</span>
                <span className="flex-1">{spec.chip}</span>
                {count !== null && count !== undefined && <span className="tabular-nums text-slate-500">{count}</span>}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
