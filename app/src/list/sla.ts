/**
 * Review wait from Settings. Mirrors `pr_nexus_core::sla`.
 * `review_sla_hours` is the shared threshold (default 48). `workingDays`
 * is `review_sla_working_days`: only Monday–Friday hours count.
 */
export interface ReviewSla {
  hours: number;
  workingDays: boolean;
}

export const DEFAULT_REVIEW_SLA_HOURS = 48;

export const DEFAULT_REVIEW_SLA: ReviewSla = {
  hours: DEFAULT_REVIEW_SLA_HOURS,
  workingDays: false,
};

const HOUR_MS = 60 * 60 * 1000;
const DAY_MS = 24 * HOUR_MS;

/** Minutes east of UTC for `now` (the same sign as the Rust helper). */
export function offsetEastMinutes(now: Date): number {
  return -now.getTimezoneOffset();
}

/** Milliseconds from `sinceMs` to `nowMs` on Monday–Friday. Both are UTC millis. */
export function weekdayElapsedMs(sinceMs: number, nowMs: number, offsetEastMinutes: number): number {
  if (nowMs <= sinceMs) {
    return 0;
  }
  const shift = offsetEastMinutes * 60_000;
  let cursor = sinceMs + shift;
  const end = nowMs + shift;
  if (end <= cursor) {
    return 0;
  }
  let total = 0;
  while (cursor < end) {
    const dayIndex = Math.floor(cursor / DAY_MS);
    const dayStart = dayIndex * DAY_MS;
    const next = Math.min(dayStart + DAY_MS, end);
    if (isWeekday(dayIndex)) {
      total += next - cursor;
    }
    cursor = next;
  }
  return total;
}

/** Unix day 0 (1970-01-01) is Thursday. 0 is Sunday. */
function isWeekday(dayIndex: number): boolean {
  const weekday = (dayIndex + 4) % 7;
  const normalized = weekday < 0 ? weekday + 7 : weekday;
  return normalized >= 1 && normalized <= 5;
}

/** Elapsed time the SLA compares against. */
export function reviewElapsedMs(sinceMs: number, now: Date, sla: ReviewSla, zone = offsetEastMinutes(now)): number {
  const nowMs = now.getTime();
  if (sla.workingDays) {
    return weekdayElapsedMs(sinceMs, nowMs, zone);
  }
  return Math.max(0, nowMs - sinceMs);
}

/** True when the wait is strictly longer than the SLA. */
export function reviewWaitExceeded(sinceMs: number, now: Date, sla: ReviewSla, zone = offsetEastMinutes(now)): boolean {
  return reviewElapsedMs(sinceMs, now, sla, zone) > sla.hours * HOUR_MS;
}

/** Soft hint for My PRs when open pull requests pass the optional cap. */
export function wipHint(open: number, limit: number | null | undefined): string | null {
  if (limit == null || limit <= 0 || open <= limit) {
    return null;
  }
  return `Over the open PR limit (${open} of ${limit})`;
}
