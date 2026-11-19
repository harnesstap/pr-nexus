import { expect, test } from "vitest";
import { defaultCatalog, orderedViews, type SidebarView } from "./viewCatalog";
import { applyViewMove, moveConfirmMessage, planViewMove } from "./viewMove";
import { MINE_QUERY, REVIEW_QUERY } from "./viewRule";

function custom(partial: Partial<SidebarView> & Pick<SidebarView, "key" | "name" | "query">): SidebarView {
  return { builtin: false, sort_order: 3, parent: null, ...partial };
}

const tree: SidebarView[] = [
  ...defaultCatalog(),
  custom({ key: "custom:1", name: "Drafts", query: "is:draft", sort_order: 3, parent: "mine" }),
  custom({ key: "custom:2", name: "Web platform", query: "team-review-requested:acme/web-platform", sort_order: 4, parent: "to_review" }),
  custom({ key: "custom:3", name: "Payments", query: "team-review-requested:acme/payments", sort_order: 5, parent: "to_review" }),
  custom({ key: "custom:4", name: "Bugs", query: `${REVIEW_QUERY} label:bug`, sort_order: 6 }),
];

test("moving a top-level view under a parent keeps only the extra rule", () => {
  const move = planViewMove(tree, "custom:4", "to_review", null);
  expect(move).toMatchObject({
    parent: "to_review",
    query: "label:bug",
    beforeQuery: `${REVIEW_QUERY} label:bug`,
    afterQuery: `${REVIEW_QUERY} label:bug`,
    added: [],
    needsConfirm: false,
  });
  const next = applyViewMove(tree, "custom:4", "to_review", move?.query ?? "", null);
  const bugs = next.find((view) => view.key === "custom:4");
  expect(bugs).toMatchObject({ parent: "to_review", query: "label:bug" });
  expect(orderedViews(next).map((view) => view.key)).toEqual(["mine", "custom:1", "to_review", "custom:2", "custom:3", "custom:4", "bumps_due"]);
});

test("a qualifier the parent adds asks before the move, and is:open stays quiet", () => {
  const views = [...defaultCatalog(), custom({ key: "custom:1", name: "Ada", query: "author:someone" })];
  const move = planViewMove(views, "custom:1", "to_review", null);
  expect(move).toMatchObject({
    parent: "to_review",
    query: "author:someone",
    added: ["review-requested:@me"],
    needsConfirm: true,
  });
  expect(move?.afterQuery).toBe(`${REVIEW_QUERY} author:someone`);
  expect(moveConfirmMessage("To review", move?.added ?? [], { before: 4, after: 1 })).toBe(
    "Under To review it will also need review-requested:@me (4 → 1 PRs). Move anyway?",
  );
  expect(moveConfirmMessage("To review", move?.added ?? [], null)).toBe("Under To review it will also need review-requested:@me. Move anyway?");
});

test("moving a child to the top level stores the full effective rule", () => {
  const move = planViewMove(tree, "custom:1", null, null);
  expect(move).toMatchObject({
    parent: null,
    query: `${MINE_QUERY} is:draft`,
    beforeQuery: `${MINE_QUERY} is:draft`,
    afterQuery: `${MINE_QUERY} is:draft`,
    added: [],
    needsConfirm: false,
  });
});

test("re-parenting composes the old rule, then strips the new parent", () => {
  const covered = planViewMove(
    [
      ...defaultCatalog(),
      custom({ key: "custom:1", name: "Both", query: "author:@me label:bug", parent: "to_review" }),
    ],
    "custom:1",
    "mine",
    null,
  );
  expect(covered).toMatchObject({
    parent: "mine",
    query: "review-requested:@me label:bug",
    beforeQuery: `${REVIEW_QUERY} author:@me label:bug`,
    afterQuery: `${MINE_QUERY} review-requested:@me label:bug`,
    added: [],
    needsConfirm: false,
  });

  const drafts = planViewMove(tree, "custom:1", "to_review", null);
  expect(drafts).toMatchObject({
    query: "author:@me is:draft",
    added: ["review-requested:@me"],
    needsConfirm: true,
  });
  expect(drafts?.beforeQuery).toBe(`${MINE_QUERY} is:draft`);
  expect(drafts?.afterQuery).toBe(`${REVIEW_QUERY} author:@me is:draft`);
});

test("a closed rule conflicts with an open parent, so the confirm names both added qualifiers", () => {
  const views = [...defaultCatalog(), custom({ key: "custom:1", name: "Done", query: "is:closed" })];
  const move = planViewMove(views, "custom:1", "mine", null);
  expect(move?.added).toEqual(["is:open", "author:@me"]);
  expect(move?.needsConfirm).toBe(true);
  expect(move?.query).toBe("is:closed");
});

test("qualifier matching ignores case, and a quoted value is kept", () => {
  const views = [...defaultCatalog(), custom({ key: "custom:1", name: "Quoted", query: 'IS:OPEN Review-Requested:@Me label:"needs review"' })];
  const move = planViewMove(views, "custom:1", "to_review", null);
  expect(move).toMatchObject({ query: 'label:"needs review"', added: [], needsConfirm: false });
});

test("siblings reorder inside one parent, and a parent row does not reshuffle its own child", () => {
  const reorder = planViewMove(tree, "custom:3", "to_review", "custom:2");
  expect(reorder).toMatchObject({
    parent: "to_review",
    query: "team-review-requested:acme/payments",
    needsConfirm: false,
    added: [],
  });
  const next = applyViewMove(tree, "custom:3", "to_review", reorder?.query ?? "", "custom:2");
  expect(orderedViews(next).map((view) => view.key)).toEqual(["mine", "custom:1", "to_review", "custom:3", "custom:2", "bumps_due", "custom:4"]);

  expect(planViewMove(tree, "custom:2", "to_review", null)).toBeNull();
  expect(planViewMove(tree, "mine", "to_review", null)).toBeNull();
  expect(planViewMove(tree, "custom:3", "bumps_due" as "mine", null)).toBeNull();
});

test("a view that already has children cannot be nested", () => {
  const views = [
    ...defaultCatalog(),
    custom({ key: "custom:1", name: "Group", query: "label:a" }),
    custom({ key: "custom:2", name: "Nested", query: "label:b", sort_order: 4, parent: "custom:1" as unknown as SidebarView["parent"] }),
  ];
  expect(planViewMove(views, "custom:1", "mine", null)).toBeNull();
  expect(applyViewMove(views, "custom:1", "mine", "label:a", null).find((view) => view.key === "custom:1")?.parent ?? null).toBeNull();
});

test("dropping a top-level view at the end of its own level is a no-op, and dropping it earlier reorders", () => {
  expect(planViewMove(tree, "custom:4", null, null)).toBeNull();
  const earlier = planViewMove(
    [...tree, custom({ key: "custom:5", name: "Later", query: "is:draft", sort_order: 7 })],
    "custom:5",
    null,
    "custom:4",
  );
  expect(earlier).toMatchObject({ parent: null, query: "is:draft", needsConfirm: false });
});
