/**
 * Moving a custom view between My PRs, To review, and the top level.
 * The stored query is rewritten so the view keeps matching the same pull
 * requests whenever the new parent does not add a requirement.
 */
import { effectiveQuery, joinRules, normalizeCatalog, orderedViews, parentKey, type ParentKey, type SidebarView } from "./viewCatalog";
import { analyzeViewRule, unmetQualifiers, withoutSharedQualifiers } from "./viewRule";

/** MIME type carried by a dragged sidebar view. */
export const VIEW_DRAG_MIME = "application/x-pr-nexus-view";

export interface ViewMove {
  key: string;
  parent: ParentKey | null;
  /** Query stored on the view after the move. */
  query: string;
  /** Effective rule before the move. */
  beforeQuery: string;
  /** Effective rule after the move. */
  afterQuery: string;
  /** Parent qualifiers that would change which pull requests match. */
  added: string[];
  needsConfirm: boolean;
}

/**
 * `before` is the sibling to insert ahead of. Null appends to that level.
 * Dropping on a parent row passes null and does not reorder a view that is
 * already there. Returns null when the view cannot move or the drop is a no-op.
 */
export function planViewMove(views: readonly SidebarView[], key: string, parent: ParentKey | null, before: string | null): ViewMove | null {
  const view = views.find((item) => item.key === key);
  if (!view || view.builtin || before === key || hasChildren(views, key)) {
    return null;
  }
  if (parent !== null && parentKey(parent) !== parent) {
    return null;
  }
  const current = parentKey(view.parent);
  const sameParent = current === parent;
  // The parent row only assigns a parent. A view that is already there stays put.
  if (sameParent && before === null && parent !== null) {
    return null;
  }
  if (sameParent && sameSlot(views, key, parent, before)) {
    return null;
  }
  const beforeQuery = effectiveQuery(view, views);
  if (analyzeViewRule(beforeQuery).invalid || analyzeViewRule(view.query).invalid) {
    return null;
  }
  const parentQuery = parent === null ? "" : (views.find((item) => item.key === parent)?.query.trim() ?? "");
  const query = sameParent ? view.query : parent === null ? beforeQuery : withoutSharedQualifiers(beforeQuery, parentQuery);
  if (analyzeViewRule(query).invalid) {
    return null;
  }
  const afterQuery = sameParent ? beforeQuery : parent === null ? query : joinRules(parentQuery, query);
  const added = sameParent || parent === null ? [] : unmetQualifiers(beforeQuery, parentQuery);
  return { key, parent, query, beforeQuery, afterQuery, added, needsConfirm: added.length > 0 };
}

/** Inserts the view and reassigns custom `sort_order` from the resulting sidebar order. */
export function applyViewMove(views: readonly SidebarView[], key: string, parent: ParentKey | null, query: string, before: string | null): SidebarView[] {
  const moving = views.find((view) => view.key === key);
  if (!moving || moving.builtin || hasChildren(views, key)) {
    return [...views];
  }
  const updated: SidebarView = { ...moving, parent, query, builtin: false };
  const visual = orderedViews(views.filter((view) => view.key !== key));
  const index = insertionIndex(visual, parent, before);
  const next = [...visual.slice(0, index), updated, ...visual.slice(index)];
  let sort = 3;
  const ranked = next.map((view) => {
    if (view.builtin) {
      return view;
    }
    const placed = { ...view, sort_order: sort };
    sort += 1;
    return placed;
  });
  return normalizeCatalog(ranked);
}

export function moveConfirmMessage(parentName: string, added: readonly string[], counts: { before: number; after: number } | null): string {
  const need = added.join(" ");
  const change = counts ? ` (${counts.before} → ${counts.after} PRs)` : "";
  return `Under ${parentName} it will also need ${need}${change}. Move anyway?`;
}

/** A custom view whose key is stored as someone else's parent. One level only. */
function hasChildren(views: readonly SidebarView[], key: string): boolean {
  return views.some((view) => view.key !== key && view.parent === key);
}

function sameSlot(views: readonly SidebarView[], key: string, parent: ParentKey | null, before: string | null): boolean {
  const group = orderedViews(views).filter((view) => !view.builtin && parentKey(view.parent) === parent);
  const index = group.findIndex((view) => view.key === key);
  if (index < 0) {
    return false;
  }
  if (before === null) {
    return index === group.length - 1;
  }
  return group.findIndex((view) => view.key === before) === index + 1;
}

function insertionIndex(visual: readonly SidebarView[], parent: ParentKey | null, before: string | null): number {
  if (before) {
    const at = visual.findIndex((view) => view.key === before);
    if (at >= 0) {
      return at;
    }
  }
  if (parent === null) {
    return visual.length;
  }
  const parentAt = visual.findIndex((view) => view.key === parent);
  if (parentAt < 0) {
    return visual.length;
  }
  let index = parentAt + 1;
  while (index < visual.length && !visual[index]?.builtin && parentKey(visual[index]?.parent) === parent) {
    index += 1;
  }
  return index;
}
