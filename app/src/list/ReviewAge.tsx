import { useEffect, useState } from "react";
import type { ReviewWait } from "../api";
import { Tooltip } from "../Tooltip";
import { reviewWaitText } from "./reviewAge";
import type { ReviewSla, ReviewUrgency } from "./sla";

const HOUR_MS = 60 * 60 * 1000;

interface Props {
  wait: ReviewWait | null | undefined;
  sla: ReviewSla;
  warnHours: number;
  /** Frozen clock for tests. Live rows tick on their own. */
  now?: Date;
}

function ageColor(urgency: ReviewUrgency): string {
  switch (urgency) {
    case "on_time":
      return "text-slate-400";
    case "approaching":
      return "text-amber-300";
    case "late":
      return "text-rose-300";
    default: {
      const unreachable: never = urgency;
      return unreachable;
    }
  }
}

/**
 * How long ago this review was requested. Ticks every second under an hour
 * and every minute after, without asking the row to redraw.
 */
export function ReviewAge({ wait, sla, warnHours, now }: Props) {
  const [clock, setClock] = useState(() => Date.now());
  const atMs = wait ? Date.parse(wait.at) : Number.NaN;
  const live = now == null && wait != null && !Number.isNaN(atMs);
  const fast = live && clock - atMs < HOUR_MS;
  useEffect(() => {
    if (!live) {
      return;
    }
    const id = window.setInterval(() => setClock(Date.now()), fast ? 1000 : 60_000);
    return () => window.clearInterval(id);
  }, [live, fast, atMs]);
  if (!wait || Number.isNaN(atMs)) {
    return null;
  }
  const at = now ?? new Date(clock);
  const text = reviewWaitText(wait, at, sla, warnHours);
  return (
    <Tooltip label={text.label} detail={text.detail} align="end" className="shrink-0">
      <span data-review-age={text.urgency} aria-label={text.label} className={`shrink-0 text-xs tabular-nums whitespace-nowrap ${ageColor(text.urgency)}`}>
        {text.short}
      </span>
    </Tooltip>
  );
}
