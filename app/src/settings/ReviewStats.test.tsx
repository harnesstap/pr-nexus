import { fireEvent, render, screen, within } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { fakeMcp, sampleReviewStats } from "./fakes";
import { McpActivity } from "./McpActivity";
import { formatDuration } from "./ReviewStats";

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
