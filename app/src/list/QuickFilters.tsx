import type { ReactElement } from "react";
import { AlertIcon, DeclinedIcon, DraftIcon, EyeOffIcon, MergeIcon } from "../icons";
import { Tooltip } from "../Tooltip";
import { QUICK_FILTERS, type QuickFilter } from "./model";

interface QuickSpec {
  label: string;
  detail: string;
  icon: ReactElement;
  /** Tailwind text color when the filter is on. */
  tone: string;
}

export const QUICK_SPECS: Record<QuickFilter, QuickSpec> = {
  draft: { label: "Draft", detail: "Show only draft PRs", icon: <DraftIcon />, tone: "text-slate-200" },
  no_review: {
    label: "No review requested",
    detail: "Open PRs nobody has been asked to review yet, on GitHub or Slack",
    icon: <EyeOffIcon />,
    tone: "text-sky-300",
  },
  ci_trouble: { label: "CI errors or conflicts", detail: "PRs with failing checks or merge conflicts", icon: <AlertIcon />, tone: "text-amber-300" },
  declined: { label: "Review declined", detail: "PRs where a reviewer requested changes", icon: <DeclinedIcon />, tone: "text-rose-300" },
  ready: { label: "Ready to merge", detail: "Approved, checks green and no conflicts", icon: <MergeIcon />, tone: "text-emerald-300" },
};

interface Props {
  active: QuickFilter | null;
  /** Rows each filter would leave, from the backend; null while the first page loads. */
  counts: Record<QuickFilter, number> | null;
  onChange: (quick: QuickFilter | null) => void;
}

/**
 * Quick status filters (§8.2): one at a time, clicking the active one again clears it.
 * Counts cover the current search and menu filters, so a badge says what clicking leaves.
 */
export function QuickFilters({ active, counts, onChange }: Props) {
  return (
    <div role="group" aria-label="Quick filters" className="ml-auto flex items-center gap-1">
      {QUICK_FILTERS.map((key) => {
        const spec = QUICK_SPECS[key];
        const on = active === key;
        const count = counts?.[key];
        return (
          <Tooltip key={key} label={spec.label} detail={on ? "Click again to show every PR" : spec.detail} align={key === "ready" ? "end" : "center"}>
            <button
              type="button"
              aria-label={count === undefined ? spec.label : `${spec.label} (${count})`}
              aria-pressed={on}
              className={`flex h-7 items-center gap-1 rounded-md border px-1.5 text-xs ${
                on ? `border-violet-400 bg-slate-800 ${spec.tone}` : "border-slate-700 bg-slate-900 text-slate-400 hover:text-white"
              }`}
              onClick={() => onChange(on ? null : key)}
            >
              {spec.icon}
              {count !== undefined && (
                <span aria-hidden="true" className={`min-w-4 tabular-nums ${count === 0 && !on ? "text-slate-600" : ""}`}>
                  {count}
                </span>
              )}
            </button>
          </Tooltip>
        );
      })}
    </div>
  );
}
