import { expect, test } from "vitest";
import { defaultCatalog, duplicateOf, effectiveQuery, newViewDraft, nextCustomKey, normalizeCatalog, orderedViews, viewNameError, viewQueryError, type SidebarView } from "./viewCatalog";
import { normalizeCollapse } from "./sidebarCollapse";
import { BUMP_QUERY, MINE_QUERY, REVIEW_QUERY } from "./viewRule";

test("a missing catalog is the three defaults", () => {
  expect(normalizeCatalog(null).map((view) => [view.key, view.query])).toEqual([
    ["mine", MINE_QUERY],
    ["to_review", REVIEW_QUERY],
    ["bumps_due", BUMP_QUERY],
  ]);
});

test("saved custom views and renamed defaults survive a reload", () => {
  const saved: SidebarView[] = [
    { key: "mine", name: "Authored", query: "is:open author:@me is:draft", builtin: true, sort_order: 0 },
    { key: "custom:2", name: "Drafts", query: "is:open is:draft", builtin: false, sort_order: 4 },
    { key: "custom:2", name: "Again", query: "is:open", builtin: false, sort_order: 5 },
    { key: "nope", name: "Dropped", query: "is:open", builtin: false, sort_order: 6 },
  ];
  const views = normalizeCatalog(saved);
  expect(views.map((view) => view.key)).toEqual(["mine", "to_review", "bumps_due", "custom:2"]);
  expect(views[0]?.name).toBe("Authored");
  expect(views[0]?.query).toBe("is:open author:@me is:draft");
  expect(views[1]?.name).toBe("To review");
  expect(views.find((view) => view.key === "custom:2")?.name).toBe("Drafts");
});

test("an unfinished quote is emptied rather than breaking the list", () => {
  const views = normalizeCatalog([{ key: "custom:1", name: "Broken", query: 'label:"needs', builtin: false, sort_order: 3 }]);
  expect(views.find((view) => view.key === "custom:1")?.query).toBe("");
});

test("names stay unique and custom keys count up", () => {
  const views = defaultCatalog();
  expect(viewNameError(views, "  my prs ")).toBe("A view already has that name.");
  expect(viewNameError(views, "   ")).toBe("Enter a name.");
  expect(viewQueryError('label:"needs')).toBe("The query has an unfinished quote.");
  const copy = duplicateOf(views, views[0]!);
  expect(copy.key).toBe("custom:1");
  expect(copy.name).toBe("My PRs copy");
  expect(nextCustomKey([...views, copy])).toBe("custom:2");
  const again = duplicateOf([...views, copy], views[0]!);
  expect(again.name).toBe("My PRs copy 2");
  expect(again.parent).toBeNull();
});

test("a stored parent survives, and anything else stays at the top level", () => {
  const views = normalizeCatalog([
    { key: "custom:1", name: "Drafts", query: "is:draft", builtin: false, sort_order: 4, parent: "mine" },
    { key: "custom:2", name: "Loose", query: "is:open", builtin: false, sort_order: 3 },
    { key: "custom:3", name: "Nope", query: "is:open", builtin: false, sort_order: 5, parent: "bumps_due" } as unknown as SidebarView,
  ]);
  expect(views.find((view) => view.key === "custom:1")?.parent).toBe("mine");
  expect(views.find((view) => view.key === "custom:2")?.parent).toBeNull();
  expect(views.find((view) => view.key === "custom:3")?.parent).toBeNull();
  expect(orderedViews(views).map((view) => view.key)).toEqual(["mine", "custom:1", "to_review", "bumps_due", "custom:2", "custom:3"]);
  const drafts = views.find((view) => view.key === "custom:1");
  expect(drafts && effectiveQuery(drafts, views)).toBe(`${MINE_QUERY} is:draft`);
});

test("a new view under To review keeps that parent and only the extra rule", () => {
  const views = [
    ...defaultCatalog(),
    { key: "custom:1", name: "Platform", query: "team-review-requested:acme/web-platform", builtin: false, sort_order: 3, parent: "to_review" as const },
  ];
  expect(newViewDraft(views, "to_review", "is:open review-requested:@me team-review-requested:acme/web-platform")).toEqual({
    parent: "to_review",
    query: "team-review-requested:acme/web-platform",
  });
  expect(newViewDraft(views, "custom:1", "assignee:@me")).toEqual({
    parent: "to_review",
    query: "team-review-requested:acme/web-platform assignee:@me",
  });
  expect(newViewDraft(views, "bumps_due", "is:draft")).toEqual({ parent: null, query: `${BUMP_QUERY} is:draft` });
  expect(newViewDraft(views, "project:1", "")).toEqual({ parent: null, query: "" });
  const copy = duplicateOf(views, views[3]!);
  expect(copy.query).toBe("team-review-requested:acme/web-platform");
  expect(copy.parent).toBe("to_review");
});

test("collapsed sections round-trip and drop unknown parents", () => {
  expect(normalizeCollapse(null)).toEqual({ views: false, projects: false, parents: [] });
  const saved = normalizeCollapse({ views: true, projects: false, parents: ["mine", "bumps_due", "mine", "to_review"] });
  expect(saved).toEqual({ views: true, projects: false, parents: ["mine", "to_review"] });
  expect(normalizeCollapse(JSON.parse(JSON.stringify(saved)))).toEqual(saved);
});
