import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { ListRow } from "../api";
import { AUTHOR } from "./model";
import { NextActionBadge, badgeUrl, BADGE_SPECS } from "./NextActionBadge";
import { NEXT_ACTIONS } from "./nextAction";

const now = new Date("2026-10-26T12:00:00Z");

function row(partial: Partial<ListRow> = {}): ListRow {
  return {
    id: 7,
    number: 7,
    title: "Row",
    url: "https://github.com/acme/widgets/pull/7",
    owner: "acme",
    repo: "acme/widgets",
    author_login: "ada",
    head_ref: "topic",
    is_draft: false,
    state: "open",
    checks: "success",
    review_decision: "review_required",
    unresolved_threads: 0,
    total_threads: 0,
    involvement: AUTHOR,
    updated_at: "2026-10-26T11:00:00Z",
    created_at: "2026-10-20T11:00:00Z",
    hidden: false,
    starred: false,
    unread: false,
    muted: false,
    snoozed: false,
    project_ids: [],
    readiness: "unknown",
    readiness_notes: [],
    conflicts: false,
    reviewers_requested: true,
    pending_reviewers: [],
    slack: null,
    ...partial,
  };
}

function setup(partial: Partial<ListRow>, viewKey = "mine") {
  const handlers = { onOpen: vi.fn(), onRequestReview: vi.fn(), onDetails: vi.fn() };
  render(<NextActionBadge row={row(partial)} viewKey={viewKey} now={now} {...handlers} />);
  return handlers;
}

test("Fix CI opens the checks page", () => {
  const { onOpen } = setup({ checks: "failure" });
  const badge = screen.getByRole("button", { name: /Fix CI/ });
  expect(badge.className).toContain("cursor-pointer");
  fireEvent.click(badge);
  expect(onOpen).toHaveBeenCalledWith("https://github.com/acme/widgets/pull/7/checks");
});

test("Follow up opens the Slack reminder flow", () => {
  const { onRequestReview, onOpen } = setup({});
  fireEvent.click(screen.getByRole("button", { name: /Follow up/ }));
  expect(onRequestReview).toHaveBeenCalledWith(expect.objectContaining({ id: 7 }));
  expect(onOpen).not.toHaveBeenCalled();
});

test("Ready to merge opens the details panel", () => {
  const { onDetails } = setup({ review_decision: "approved" });
  fireEvent.click(screen.getByRole("button", { name: /Ready to merge/ }));
  expect(onDetails).toHaveBeenCalled();
});

test("waiting states are not buttons and explain why", () => {
  setup({ involvement: 2, review_decision: "changes_requested" }, "to_review");
  expect(screen.queryByRole("button")).toBeNull();
  expect(screen.getByRole("img", { name: "Waiting on author: Changes were requested: waiting on the author" })).toBeTruthy();
});

test("merged PRs show Merged, not Done", () => {
  setup({ state: "merged" });
  expect(screen.getByRole("img", { name: /^Merged/ }).textContent).toBe("Merged");
});

test("every action has a spec, and clickable ones say what a click does", () => {
  for (const action of NEXT_ACTIONS) {
    const spec = BADGE_SPECS[action];
    expect(spec.label.length).toBeGreaterThan(0);
    if (spec.click) {
      expect(spec.hint).toMatch(/^Click to/);
    }
  }
  expect(badgeUrl(row(), "files")).toBe("https://github.com/acme/widgets/pull/7/files");
  expect(badgeUrl(row(), "slack")).toBeNull();
});
