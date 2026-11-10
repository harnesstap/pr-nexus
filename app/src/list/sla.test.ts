import { expect, test } from "vitest";
import { reviewWaitExceeded, weekdayElapsedMs, wipHint, type ReviewSla } from "./sla";

const HOUR = 60 * 60 * 1000;

function ms(iso: string): number {
  return new Date(iso).getTime();
}

test("a weekend adds no weekday hours", () => {
  const since = ms("2026-11-07T00:00:00Z");
  const now = ms("2026-11-09T00:00:00Z");
  expect(weekdayElapsedMs(since, now, 0)).toBe(0);
  expect(weekdayElapsedMs(now, since, 0)).toBe(0);
});

test("friday noon to monday noon is one weekday", () => {
  const since = ms("2026-11-06T12:00:00Z");
  const now = new Date("2026-11-09T12:00:00Z");
  expect(weekdayElapsedMs(since, now.getTime(), 0)).toBe(24 * HOUR);
  const weekdays: ReviewSla = { hours: 48, workingDays: true };
  expect(reviewWaitExceeded(since, now, weekdays, 0)).toBe(false);
  expect(reviewWaitExceeded(since, now, { hours: 48, workingDays: false }, 0)).toBe(true);
});

test("the open PR hint appears only once the cap is passed", () => {
  expect(wipHint(8, 8)).toBeNull();
  expect(wipHint(8, null)).toBeNull();
  expect(wipHint(9, 8)).toBe("Over the open PR limit (9 of 8)");
});

test("the threshold is strictly greater than the sla", () => {
  const now = new Date("2026-11-10T12:00:00Z");
  const exact = now.getTime() - 48 * HOUR;
  expect(reviewWaitExceeded(exact, now, { hours: 48, workingDays: false }, 0)).toBe(false);
  expect(reviewWaitExceeded(exact - 1, now, { hours: 48, workingDays: false }, 0)).toBe(true);
});
