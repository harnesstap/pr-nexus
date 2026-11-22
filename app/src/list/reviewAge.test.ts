import { expect, test } from "vitest";
import { DEFAULT_REVIEW_SLA, type ReviewSla } from "./sla";
import { formatElapsed, formatRequestWhen, reviewWaitText } from "./reviewAge";

const sla: ReviewSla = DEFAULT_REVIEW_SLA;
const hour = 60 * 60 * 1000;

test("elapsed time uses two units and drops a zero remainder", () => {
  expect(formatElapsed(0)).toBe("0s");
  expect(formatElapsed(999)).toBe("0s");
  expect(formatElapsed(45_000)).toBe("45s");
  expect(formatElapsed(80_000)).toBe("1m 20s");
  expect(formatElapsed(60_000)).toBe("1m");
  expect(formatElapsed(61_000)).toBe("1m 1s");
  expect(formatElapsed(90 * 60_000)).toBe("1h 30m");
  expect(formatElapsed(2 * hour)).toBe("2h");
  expect(formatElapsed((2 * 24 + 4) * hour)).toBe("2d 4h");
  expect(formatElapsed((3 * 7 + 2) * 24 * hour)).toBe("3w 2d");
});

test("a request is on time, then amber, then late", () => {
  const now = new Date("2026-11-10T12:00:00Z");
  const text = (hoursAgo: number) =>
    reviewWaitText({ at: new Date(now.getTime() - hoursAgo * hour).toISOString(), exact: true, basis: "request" }, now, sla, 12);
  expect(text(1).urgency).toBe("on_time");
  expect(text(1).label).toContain("due in");
  expect(text(36).urgency).toBe("approaching");
  expect(text(48).urgency).toBe("approaching");
  expect(text(48).label).toContain("due now");
  expect(text(48 + 1 / 3600).urgency).toBe("late");
  const late = text(50);
  expect(late.urgency).toBe("late");
  expect(late.short).toBe("2d 2h");
  expect(late.label).toContain("late by 2h");
});

test("the tooltip names the request and when it is due", () => {
  const at = new Date("2026-11-16T14:02:00Z");
  const now = new Date(at.getTime() + 90 * 60_000);
  const onTime = reviewWaitText({ at: at.toISOString(), exact: true, basis: "request" }, now, sla, 12);
  expect(onTime.short).toBe("1h 30m");
  expect(onTime.label).toBe(`Review requested 1h 30m ago (${formatRequestWhen(at)}) · due in 1d 22h`);
  expect(onTime.detail).toBeUndefined();

  const due = new Date(at.getTime() + 48 * hour - 30 * 60_000);
  const approaching = reviewWaitText({ at: at.toISOString(), exact: true, basis: "request" }, due, sla, 12);
  expect(approaching.urgency).toBe("approaching");
  expect(approaching.label).toBe(`Review requested 1d 23h ago (${formatRequestWhen(at)}) · due in 30m`);

  const lateAt = new Date(at.getTime() + 50 * hour);
  const late = reviewWaitText({ at: at.toISOString(), exact: true, basis: "request" }, lateAt, sla, 12);
  expect(late.label).toContain("late by 2h");

  const opened = reviewWaitText({ at: at.toISOString(), exact: false, basis: "opened" }, now, { hours: 48, workingDays: true }, 12);
  expect(opened.detail).toContain("when the pull request was opened");
  expect(opened.detail).toContain("weekday hours");
  const ready = reviewWaitText({ at: at.toISOString(), exact: false, basis: "ready" }, now, sla, 12);
  expect(ready.detail).toContain("marked ready for review");
});

test("a missing or unknown basis still paints the age when the time is approximate", () => {
  const at = new Date("2026-11-16T14:02:00Z");
  const now = new Date(at.getTime() + 90 * 60_000);
  const missing = reviewWaitText({ at: at.toISOString(), exact: false } as { at: string; exact: boolean; basis: "request" }, now, sla, 12);
  expect(missing.short).toBe("1h 30m");
  expect(missing.detail).toBe("Approximate: the review request time may not be exact.");
  const unknown = reviewWaitText({ at: at.toISOString(), exact: false, basis: "timeline" as "request" }, now, sla, 12);
  expect(unknown.detail).toBe("Approximate: the review request time may not be exact.");
});
