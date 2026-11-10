/**
 * Stacked pull requests, from head and base refs already on each row.
 * Mirrors `pr_nexus_core::stack`: same parent rule, retarget, and cycle break.
 */

export type StackState = "open" | "closed" | "merged";

export interface StackRef {
  id: number;
  /** `owner/name`. Branch names only match inside one repository. */
  repo: string;
  number: number;
  state: StackState;
  head_ref: string;
  base_ref: string;
}

/** Where one open pull request sits. Absent when it is alone. */
export interface StackPlace {
  /** 1-based index in merge order (bottom is 1). */
  position: number;
  size: number;
  root_id: number;
  root_number: number;
  parent_id: number | null;
  parent_number: number | null;
  /** Head branch of the parent. The child merges into this until the parent lands. */
  parent_head: string | null;
  /** 0 at the bottom of the stack. */
  depth: number;
  /** The bottom of the stack, which has to merge before the rest. */
  blocking: boolean;
}

export interface StackChain {
  root_id: number;
  blocking_id: number;
  member_ids: number[];
}

export interface StackIndex {
  places: Map<number, StackPlace>;
  chains: StackChain[];
}

/** Build the forest. A pull request that is not in a stack of two or more has no place. */
export function stackIndex(prs: readonly StackRef[]): StackIndex {
  const byId = new Map(prs.map((pr) => [pr.id, pr]));
  const byRepo = new Map<string, StackRef[]>();
  for (const pr of prs) {
    const group = byRepo.get(pr.repo);
    if (group) {
      group.push(pr);
    } else {
      byRepo.set(pr.repo, [pr]);
    }
  }

  const parentOf = new Map<number, number>();
  for (const group of byRepo.values()) {
    const openByHead = byHead(group, "open");
    const mergedByHead = byHead(group, "merged");
    for (const pr of group) {
      if (pr.state !== "open") {
        continue;
      }
      const base = effectiveBase(pr, openByHead, mergedByHead);
      const parent = openByHead.get(base);
      if (!parent || parent.id === pr.id) {
        continue;
      }
      parentOf.set(pr.id, parent.id);
    }
  }
  breakCycles(parentOf, byId);

  const children = new Map<number, number[]>();
  const hasParent = new Set<number>();
  for (const [child, parent] of parentOf) {
    const kids = children.get(parent);
    if (kids) {
      kids.push(child);
    } else {
      children.set(parent, [child]);
    }
    hasParent.add(child);
  }
  for (const kids of children.values()) {
    kids.sort((left, right) => cmpPr(left, right, byId));
  }

  const roots = [...byId.values()]
    .filter((pr) => pr.state === "open" && !hasParent.has(pr.id))
    .map((pr) => pr.id)
    .sort((left, right) => cmpPr(left, right, byId));

  const places = new Map<number, StackPlace>();
  const chains: StackChain[] = [];
  for (const root of roots) {
    const ordered: { id: number; depth: number; parent: number | null }[] = [];
    walk(root, 0, null, children, ordered);
    if (ordered.length < 2) {
      continue;
    }
    const rootNumber = byId.get(root)?.number ?? 0;
    const memberIds: number[] = [];
    ordered.forEach((entry, index) => {
      const parent = entry.parent === null ? undefined : byId.get(entry.parent);
      places.set(entry.id, {
        position: index + 1,
        size: ordered.length,
        root_id: root,
        root_number: rootNumber,
        parent_id: parent?.id ?? null,
        parent_number: parent?.number ?? null,
        parent_head: parent?.head_ref ?? null,
        depth: entry.depth,
        blocking: entry.id === root,
      });
      memberIds.push(entry.id);
    });
    chains.push({ root_id: root, blocking_id: root, member_ids: memberIds });
  }
  return { places, chains };
}

export function chainOf(index: StackIndex, id: number): StackChain | null {
  const place = index.places.get(id);
  if (!place) {
    return null;
  }
  return index.chains.find((chain) => chain.root_id === place.root_id) ?? null;
}

/**
 * Grey note when Merge would land on the parent branch. Null when this pull
 * request is the bottom of the stack, or the live base is no longer the parent's head.
 */
export function stackMergeNote(baseRef: string, place: StackPlace | null | undefined): string | null {
  if (!place || place.parent_number === null || !place.parent_head) {
    return null;
  }
  if (baseRef !== place.parent_head) {
    return null;
  }
  return `Merging now lands this on ${baseRef}, the branch of #${place.parent_number}. That pull request is still open.`;
}

function effectiveBase(pr: StackRef, openByHead: Map<string, StackRef>, mergedByHead: Map<string, StackRef>): string {
  let base = pr.base_ref;
  const seen = new Set<string>();
  for (;;) {
    if (base === "" || seen.has(base)) {
      break;
    }
    seen.add(base);
    if (openByHead.has(base)) {
      break;
    }
    const via = mergedByHead.get(base);
    if (!via || via.id === pr.id) {
      break;
    }
    base = via.base_ref;
  }
  return base;
}

function byHead(group: readonly StackRef[], state: StackState): Map<string, StackRef> {
  const out = new Map<string, StackRef>();
  for (const pr of group) {
    if (pr.state !== state || pr.head_ref === "") {
      continue;
    }
    const current = out.get(pr.head_ref);
    if (!current || pr.number < current.number || (pr.number === current.number && pr.id < current.id)) {
      out.set(pr.head_ref, pr);
    }
  }
  return out;
}

function breakCycles(parentOf: Map<number, number>, byId: Map<number, StackRef>): void {
  const limit = parentOf.size + 1;
  for (let step = 0; step < limit; step += 1) {
    const cycle = findCycle(parentOf);
    if (!cycle) {
      return;
    }
    const victim = cycle.reduce((best, id) => (cmpPr(id, best, byId) < 0 ? id : best));
    parentOf.delete(victim);
  }
}

function findCycle(parentOf: Map<number, number>): number[] | null {
  const starts = [...parentOf.keys()].sort((left, right) => left - right);
  for (const start of starts) {
    const path: number[] = [];
    const at = new Map<number, number>();
    let cursor: number | undefined = start;
    while (cursor !== undefined) {
      const seen = at.get(cursor);
      if (seen !== undefined) {
        return path.slice(seen);
      }
      at.set(cursor, path.length);
      path.push(cursor);
      cursor = parentOf.get(cursor);
    }
  }
  return null;
}

function walk(
  id: number,
  depth: number,
  parent: number | null,
  children: Map<number, number[]>,
  out: { id: number; depth: number; parent: number | null }[],
): void {
  out.push({ id, depth, parent });
  for (const child of children.get(id) ?? []) {
    walk(child, depth + 1, id, children, out);
  }
}

function cmpPr(left: number, right: number, byId: Map<number, StackRef>): number {
  const ln = byId.get(left)?.number ?? 0;
  const rn = byId.get(right)?.number ?? 0;
  return ln - rn || left - right;
}
