import { expect, test } from "vitest";
import { defaultFilters, withSearch } from "./model";
import { applyFilterChange, applySearchChange, omitRuleQualifiers, tokensFor } from "./filterQuery";

const viewRule = "is:open review-requested:@me";

test("a menu choice is appended, and removing it leaves the rest in place", () => {
  const start = { ...defaultFilters(), search: "passkeys author:ada" };
  const drafted = applyFilterChange(start.search, start, { ...start, statuses: ["draft", "ready"] }, viewRule);
  expect(drafted).toBe("passkeys author:ada is:draft");
  const reviewed = applyFilterChange(drafted, { ...start, statuses: ["draft", "ready"] }, { ...start, statuses: ["draft", "ready"], reviews: ["approved"] }, viewRule);
  expect(reviewed).toBe("passkeys author:ada is:draft review:approved");
  const dropped = applyFilterChange(reviewed, { ...start, statuses: ["draft", "ready"], reviews: ["approved"] }, { ...start, statuses: ["ready"], reviews: ["approved"] }, viewRule);
  expect(dropped).toBe("passkeys author:ada review:approved");
  const teams = applyFilterChange(dropped, start, { ...start, teams: ["acme/web-platform", "acme/payments"] }, viewRule);
  expect(teams).toBe("passkeys author:ada review:approved team-review-requested:acme/web-platform team-review-requested:acme/payments");
  const assigned = applyFilterChange(teams, { ...start, teams: ["acme/web-platform", "acme/payments"] }, { ...start, teams: ["acme/web-platform", "acme/payments"], assignees: ["@me"], unassigned: true }, viewRule);
  expect(assigned).toBe(
    "passkeys author:ada review:approved team-review-requested:acme/web-platform team-review-requested:acme/payments assignee:@me no:assignee",
  );
});

test("the view rule is not copied into the field", () => {
  const start = defaultFilters();
  const next = applyFilterChange("", start, { ...start, statuses: ["draft"] }, "is:open is:draft author:@me");
  expect(next).toBe("");
  expect(omitRuleQualifiers("repo:acme/api is:draft review:approved", "is:open is:draft")).toBe("repo:acme/api review:approved");
});

test("editing the field updates the menu and the menu updates the field", () => {
  const start = defaultFilters();
  const text = "org:acme review:approved status:failure assignee:@me no:assignee team-review-requested:acme/web-platform passkeys";
  const typed = applySearchChange(start, "", text);
  expect(typed.orgs).toEqual(["acme"]);
  expect(typed.reviews).toEqual(["approved"]);
  expect(typed.ci).toEqual(["failing"]);
  expect(typed.assignees).toEqual(["@me"]);
  expect(typed.unassigned).toBe(true);
  expect(typed.teams).toEqual(["acme/web-platform"]);
  expect(typed.starred).toBe(false);
  const removed = applySearchChange(typed, text, "org:acme passkeys");
  expect(removed.orgs).toEqual(["acme"]);
  expect(removed.reviews).toEqual([]);
  expect(removed.ci).toEqual([]);
  expect(removed.assignees).toEqual([]);
  expect(removed.unassigned).toBe(false);
  expect(removed.teams).toEqual([]);
  const saved = withSearch({ ...defaultViewish(), filters: { ...start, starred: true, statuses: ["ready"] } }, "status:failure is:draft");
  expect(saved.filters.starred).toBe(true);
  expect(saved.filters.statuses).toEqual(["ready", "draft"]);
  expect(saved.filters.ci).toEqual(["failing"]);
  expect(saved.filters.github).toBe("");
  expect(saved.filters.search).toBe("status:failure is:draft");
});

test("a negated menu token stays out of the menu", () => {
  const next = applySearchChange(defaultFilters(), "", "-org:acme org:acme-labs");
  expect(next.orgs).toEqual(["acme-labs"]);
  expect(tokensFor(next)).toEqual(["org:acme-labs"]);
});

function defaultViewish() {
  return { grouping: "project" as const, ordering: "attention" as const, direction: "desc" as const, show: { numberTitle: true, repo: true, ci: true, review: true, threads: true, teams: false, activity: false, badges: true, notRequested: true }, filters: defaultFilters() };
}
