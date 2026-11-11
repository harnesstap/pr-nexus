import { expect, test } from "vitest";
import type { ListRow } from "../api";
import { lastWorkingDayStart, standupRecap } from "./standup";

const tuesday = new Date(2026, 10, 10, 9, 0, 0);

function iso(date: Date): string {
  return date.toISOString();
}

function hoursBefore(now: Date, hours: number): string {
  return iso(new Date(now.getTime() - hours * 3_600_000));
}

function row(partial: Partial<ListRow> & Pick<ListRow, "id" | "title">): ListRow {
  return {
    number: partial.id,
    url: `https://github.com/acme/widgets/pull/${partial.id}`,
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
    involvement: 1,
    updated_at: hoursBefore(tuesday, 5),
    created_at: hoursBefore(tuesday, 30),
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

test("last working day is Monday on Tuesday and Friday on Monday", () => {
  const monday = lastWorkingDayStart(tuesday);
  expect(monday.getDay()).toBe(1);
  expect(monday.getDate()).toBe(9);
  const fromMonday = lastWorkingDayStart(new Date(2026, 10, 9, 9, 0, 0));
  expect(fromMonday.getDay()).toBe(5);
  expect(fromMonday.getDate()).toBe(6);
  const fromSunday = lastWorkingDayStart(new Date(2026, 10, 8, 15, 0, 0));
  expect(fromSunday.getDay()).toBe(5);
  expect(fromSunday.getDate()).toBe(6);
});

test("the recap groups one line per PR and skips empty sections", () => {
  const mergedAt = new Date(2026, 10, 9, 16, 0, 0);
  const oldMerge = new Date(2026, 10, 5, 12, 0, 0);
  const text = standupRecap(
    [
      row({ id: 1, title: "Ship auth", state: "merged", updated_at: iso(mergedAt), review_decision: "approved" }),
      row({ id: 2, title: "Old ship", state: "merged", updated_at: iso(oldMerge) }),
      row({ id: 3, title: "Passkeys", review_decision: "approved", checks: "success", reviewers_requested: false }),
      row({ id: 4, title: "Tax rounding", created_at: hoursBefore(tuesday, 80) }),
      row({ id: 4, title: "Tax rounding duplicate", created_at: hoursBefore(tuesday, 80) }),
      row({ id: 5, title: "Address comments", review_decision: "changes_requested", checks: "failure" }),
      row({ id: 6, title: "Red build", checks: "failure", review_decision: null, reviewers_requested: false }),
      row({ id: 7, title: "Notes draft", is_draft: true, checks: "failure" }),
      row({ id: 8, title: "Just closed", state: "closed", updated_at: iso(mergedAt) }),
      row({ id: 9, title: "Bracket [api]", review_decision: "approved", updated_at: hoursBefore(tuesday, 1) }),
    ],
    tuesday,
    [{ title: "Look at billing", url: "https://github.com/acme/billing/pull/3", at: iso(new Date(2026, 10, 9, 11, 0, 0)) }],
  );
  expect(text).toBe(
    [
      "## Stand-up · since Monday",
      "",
      "### Merged",
      "- [Ship auth](https://github.com/acme/widgets/pull/1)",
      "",
      "### Ready to merge",
      "- [Bracket \\[api\\]](https://github.com/acme/widgets/pull/9)",
      "- [Passkeys](https://github.com/acme/widgets/pull/3)",
      "",
      "### Waiting for review",
      "- [Tax rounding](https://github.com/acme/widgets/pull/4) · 3d",
      "",
      "### Changes requested",
      "- [Address comments](https://github.com/acme/widgets/pull/5)",
      "",
      "### CI failing",
      "- [Red build](https://github.com/acme/widgets/pull/6)",
      "",
      "### Drafts",
      "- [Notes draft](https://github.com/acme/widgets/pull/7)",
      "",
      "### Reviewed by me",
      "- [Look at billing](https://github.com/acme/billing/pull/3)",
      "",
    ].join("\n"),
  );
});

test("reviewed by me is omitted when no review time is known", () => {
  const text = standupRecap([], tuesday);
  expect(text).toBe("## Stand-up · since Monday\n\nNothing to report.\n");
  expect(text).not.toContain("Reviewed by me");
});

test("an old review stays out of the recap", () => {
  const text = standupRecap([], tuesday, [
    { title: "Last week", url: "https://github.com/acme/widgets/pull/2", at: iso(new Date(2026, 10, 5, 12, 0, 0)) },
  ]);
  expect(text).not.toContain("Reviewed by me");
  expect(text).toContain("Nothing to report.");
});
