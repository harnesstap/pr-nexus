import { afterEach, expect, test, vi } from "vitest";
import { every } from "./every";

afterEach(() => {
  vi.useRealTimers();
});

test("the first tick waits out the interval", () => {
  vi.useFakeTimers();
  const tick = vi.fn();
  const stop = every(1_000, tick);
  vi.advanceTimersByTime(999);
  expect(tick).not.toHaveBeenCalled();
  vi.advanceTimersByTime(1);
  expect(tick).toHaveBeenCalledTimes(1);
  stop();
});

test("only one timer is pending, so a wake cannot stack callbacks", () => {
  vi.useFakeTimers();
  const tick = vi.fn();
  const stop = every(1_000, tick);
  expect(vi.getTimerCount()).toBe(1);
  vi.advanceTimersByTime(1_000);
  expect(tick).toHaveBeenCalledTimes(1);
  expect(vi.getTimerCount()).toBe(1);
  vi.advanceTimersByTime(49_000);
  expect(tick).toHaveBeenCalledTimes(50);
  expect(vi.getTimerCount()).toBe(1);
  stop();
  expect(vi.getTimerCount()).toBe(0);
});
