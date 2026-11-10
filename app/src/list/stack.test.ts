import { expect, test } from "vitest";
import { chainOf, stackIndex, stackMergeNote, type StackRef, type StackState } from "./stack";

function pr(id: number, number: number, state: StackState, head: string, base: string, repo = "acme/widgets"): StackRef {
  return { id, repo, number, state, head_ref: head, base_ref: base };
}

const open = (id: number, number: number, head: string, base: string) => pr(id, number, "open", head, base);

function ids(id: number, refs: StackRef[]): number[] | null {
  return chainOf(stackIndex(refs), id)?.member_ids ?? null;
}

test("a linear stack is bottom to top and the bottom blocks", () => {
  const built = stackIndex([
    open(1, 10, "feat-a", "main"),
    open(2, 11, "feat-b", "feat-a"),
    open(3, 12, "feat-c", "feat-b"),
    open(4, 13, "feat-d", "feat-c"),
  ]);
  expect(chainOf(built, 3)?.member_ids).toEqual([1, 2, 3, 4]);
  const second = built.places.get(2);
  expect(second).toMatchObject({ position: 2, size: 4, depth: 1, parent_id: 1, parent_number: 10, parent_head: "feat-a", blocking: false });
  expect(built.places.get(1)?.blocking).toBe(true);
  expect(built.places.get(4)?.position).toBe(4);
  expect(stackMergeNote("feat-a", second)).toBe("Merging now lands this on feat-a, the branch of #10. That pull request is still open.");
  expect(stackMergeNote("main", second)).toBeNull();
  expect(stackMergeNote("main", built.places.get(1))).toBeNull();
});

test("a branch lists each child after its parent in number order", () => {
  const built = stackIndex([
    open(1, 10, "feat-a", "main"),
    open(3, 12, "feat-c", "feat-a"),
    open(2, 11, "feat-b", "feat-a"),
    open(4, 13, "feat-d", "feat-c"),
  ]);
  expect(chainOf(built, 4)?.member_ids).toEqual([1, 2, 3, 4]);
  expect(built.places.get(2)?.depth).toBe(1);
  expect(built.places.get(4)?.depth).toBe(2);
  expect(built.places.get(4)?.position).toBe(4);
});

test("a cycle stops and the lowest number is the bottom", () => {
  const built = stackIndex([open(1, 20, "feat-a", "feat-b"), open(2, 10, "feat-b", "feat-a")]);
  expect(chainOf(built, 1)?.member_ids).toEqual([2, 1]);
  expect(built.places.get(2)?.blocking).toBe(true);
  expect(built.places.get(2)?.parent_id).toBeNull();
});

test("a three pull request cycle also stops", () => {
  expect(
    ids(1, [open(1, 30, "feat-a", "feat-c"), open(2, 10, "feat-b", "feat-a"), open(3, 20, "feat-c", "feat-b")]),
  ).toEqual([2, 3, 1]);
});

test("a merged parent retargets the base onto the next open pull request", () => {
  const built = stackIndex([
    open(1, 10, "feat-a", "main"),
    pr(2, 11, "merged", "feat-b", "feat-a"),
    open(3, 12, "feat-c", "feat-b"),
  ]);
  expect(chainOf(built, 3)?.member_ids).toEqual([1, 3]);
  expect(built.places.has(2)).toBe(false);
  expect(built.places.get(3)?.parent_id).toBe(1);
  expect(built.places.get(3)?.position).toBe(2);
});

test("a merged parent whose child already points at main is not a stack", () => {
  const built = stackIndex([pr(1, 10, "merged", "feat-a", "main"), open(2, 11, "feat-b", "main")]);
  expect(built.chains).toEqual([]);
});

test("a merged parent with no further open base leaves the child unstacked", () => {
  const built = stackIndex([pr(1, 10, "merged", "feat-a", "main"), open(2, 11, "feat-b", "feat-a")]);
  expect(built.places.has(2)).toBe(false);
});

test("the same branch names in another repository do not link", () => {
  const built = stackIndex([open(1, 10, "feat-a", "main"), pr(2, 11, "open", "feat-b", "feat-a", "acme-labs/widgets")]);
  expect(built.chains).toEqual([]);
});

test("a closed pull request is not a parent", () => {
  const built = stackIndex([pr(1, 10, "closed", "feat-a", "main"), open(2, 11, "feat-b", "feat-a")]);
  expect(built.places.has(2)).toBe(false);
});

test("two open pull requests with one head pick the lower number as parent", () => {
  const built = stackIndex([open(1, 20, "feat-a", "main"), open(2, 10, "feat-a", "main"), open(3, 30, "feat-b", "feat-a")]);
  expect(built.places.get(3)?.parent_id).toBe(2);
  expect(chainOf(built, 3)?.member_ids).toEqual([2, 3]);
  expect(built.places.has(1)).toBe(false);
});
