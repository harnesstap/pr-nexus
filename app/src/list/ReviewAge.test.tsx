import { act, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { ReviewAge } from "./ReviewAge";
import { DEFAULT_REVIEW_SLA } from "./sla";

afterEach(() => {
  vi.useRealTimers();
});

test("a request under an hour ticks each second, and an older one each minute", () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-11-16T12:00:00Z"));
  const spy = vi.spyOn(window, "setInterval");
  const { rerender } = render(
    <ReviewAge wait={{ at: "2026-11-16T11:59:15Z", exact: true, basis: "request" }} sla={DEFAULT_REVIEW_SLA} warnHours={12} />,
  );
  expect(screen.getByText("45s")).toBeInTheDocument();
  expect(spy).toHaveBeenCalledWith(expect.any(Function), 1000);
  act(() => {
    vi.advanceTimersByTime(1000);
  });
  expect(screen.getByText("46s")).toBeInTheDocument();

  spy.mockClear();
  rerender(<ReviewAge wait={{ at: "2026-11-16T10:00:00Z", exact: true, basis: "request" }} sla={DEFAULT_REVIEW_SLA} warnHours={12} />);
  expect(screen.getByText("2h")).toBeInTheDocument();
  expect(spy).toHaveBeenCalledWith(expect.any(Function), 60_000);
  act(() => {
    vi.advanceTimersByTime(1000);
  });
  expect(screen.getByText("2h")).toBeInTheDocument();
  act(() => {
    vi.advanceTimersByTime(60_000);
  });
  expect(screen.getByText("2h 1m")).toBeInTheDocument();
});
