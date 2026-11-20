import { fireEvent, render, screen, within } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { StatsFilter, StatsPeriod } from "../api";
import { fakeMcp, sampleReviewStats } from "./fakes";
import { McpActivity } from "./McpActivity";
import { formatDuration, ReviewStats } from "./ReviewStats";

test("the Stats sub-tab loads on open and shows cards and reviewers", async () => {
  const api = fakeMcp();
  const read = vi.spyOn(api, "reviewStats");
  render(<McpActivity api={api} />);
  expect(read).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("tab", { name: "Stats" }));
  expect(await screen.findByText("Time to first review")).toBeTruthy();
  expect(read).toHaveBeenCalledWith(30, {}, false);
  const table = screen.getByRole("table", { name: "Most requested reviewers" });
  expect(within(table).getByText("@grace")).toBeTruthy();
  expect(within(table).getByText("#team-reviews")).toBeTruthy();
});

test("period and repository change the request", async () => {
  const api = fakeMcp();
  const read = vi.spyOn(api, "reviewStats");
  render(<McpActivity api={api} initialView="stats" />);
  await screen.findByText("Time to first review");
  fireEvent.click(screen.getByRole("button", { name: "7d" }));
  await vi.waitFor(() => expect(read).toHaveBeenLastCalledWith(7, {}, false));
  fireEvent.change(screen.getByRole("combobox", { name: "Repository or organization" }), { target: { value: "repo:acme/web" } });
  await vi.waitFor(() => expect(read).toHaveBeenLastCalledWith(7, { repo: "acme/web" }, false));
  await screen.findByRole("button", { name: "Refresh" });
  fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
  await vi.waitFor(() => expect(read).toHaveBeenLastCalledWith(7, { repo: "acme/web" }, true));
});

test.each([
  ["repo:acme/web", { repo: "acme/web" }, "acme/web"],
  ["org:acme-labs", { org: "acme-labs" }, "acme-labs (all)"],
])("the menu keeps %s when the new period has no pull requests there", async (value, filter, label) => {
  const reviewStats = vi.fn((days: StatsPeriod, requested: StatsFilter) => {
    const stats = sampleReviewStats(days, requested);
    // acme/web and acme-labs only have pull requests older than 7 days.
    stats.repos = days === 7 ? ["acme/api"] : ["acme/api", "acme/web", "acme-labs/site"];
    return Promise.resolve({ stats, fetched_at: new Date().toISOString(), partial: false });
  });
  render(<ReviewStats api={{ reviewStats }} />);
  await screen.findByText("Time to first review");
  const select = screen.getByRole("combobox", { name: "Repository or organization" }) as HTMLSelectElement;
  fireEvent.change(select, { target: { value } });
  await vi.waitFor(() => expect(reviewStats).toHaveBeenLastCalledWith(30, filter, false));
  fireEvent.click(screen.getByRole("button", { name: "7d" }));
  await vi.waitFor(() => expect(reviewStats).toHaveBeenLastCalledWith(7, filter, false));
  await screen.findByRole("button", { name: "Refresh" });
  expect(select.value).toBe(value);
  expect(select.selectedOptions[0]?.textContent).toBe(label);
});

test("an error shows instead of the cards", async () => {
  const api = { ...fakeMcp(), reviewStats: () => Promise.reject(new Error("rate limited")) };
  render(<McpActivity api={api} initialView="stats" />);
  expect((await screen.findByRole("alert")).textContent).toContain("rate limited");
});

test("durations read compactly", () => {
  expect(formatDuration(null)).toBe("—");
  expect(formatDuration(45 * 60)).toBe("45m");
  expect(formatDuration(3 * 3600 + 20 * 60)).toBe("3h 20m");
  expect(formatDuration(52 * 3600)).toBe("2d 4h");
  expect(sampleReviewStats(90).opened).toBeGreaterThan(sampleReviewStats(7).opened);
});
