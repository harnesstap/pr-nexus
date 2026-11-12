import { expect, test } from "vitest";
import { defaultCatalog, duplicateOf, nextCustomKey, normalizeCatalog, viewNameError, viewQueryError, type SidebarView } from "./viewCatalog";
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
});
