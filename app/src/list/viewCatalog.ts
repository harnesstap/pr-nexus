/**
 * The sidebar's saved views. Built-in keys stay `mine`, `to_review`, and
 * `bumps_due` so a view saved before custom views still loads. Custom keys
 * are `custom:<n>`.
 */
import { BUMP_QUERY, MINE_QUERY, REVIEW_QUERY, VIEW_NAME_MAX, VIEW_QUERY_MAX, analyzeViewRule } from "./viewRule";

/** My PRs and To review can parent a custom view. Bumps due cannot. */
export type ParentKey = "mine" | "to_review";

export interface SidebarView {
  key: string;
  name: string;
  query: string;
  builtin: boolean;
  sort_order: number;
  /** Set only on a custom view. One level: a child is never a parent. */
  parent?: ParentKey | null;
}

export function defaultCatalog(): SidebarView[] {
  return [
    { key: "mine", name: "My PRs", query: MINE_QUERY, builtin: true, sort_order: 0 },
    { key: "to_review", name: "To review", query: REVIEW_QUERY, builtin: true, sort_order: 1 },
    { key: "bumps_due", name: "Bumps due", query: BUMP_QUERY, builtin: true, sort_order: 2 },
  ];
}

const BUILTIN_KEYS = ["mine", "to_review", "bumps_due"] as const;

/** A stored catalog, with the three built-ins filled in when they are missing. */
export function normalizeCatalog(saved: readonly SidebarView[] | null | undefined): SidebarView[] {
  const input = Array.isArray(saved) ? saved : [];
  const out: SidebarView[] = [];
  for (const [index, fallback] of defaultCatalog().entries()) {
    const found = input.find((view) => view.key === fallback.key);
    const name = cleanName(found?.name ?? "") || fallback.name;
    const query = capQuery(typeof found?.query === "string" ? found.query : fallback.query);
    out.push({
      key: fallback.key,
      name: nameTaken(out, name) ? fallback.name : name,
      query: analyzeViewRule(query).invalid ? fallback.query : query,
      builtin: true,
      sort_order: index,
    });
  }
  const customs = input
    .filter((view) => validCustomKey(view.key))
    .filter((view, index, all) => all.findIndex((other) => other.key === view.key) === index)
    .slice()
    .sort((left, right) => left.sort_order - right.sort_order || (left.key < right.key ? -1 : 1));
  for (const view of customs) {
    let name = cleanName(view.name);
    if (name === "") {
      name = `View ${out.length}`;
    }
    name = uniqueName(out, name);
    const query = capQuery(view.query);
    out.push({
      key: view.key,
      name,
      query: analyzeViewRule(query).invalid ? "" : query,
      builtin: false,
      sort_order: out.length,
      parent: parentKey(view.parent),
    });
  }
  return out;
}

export function viewNameError(views: readonly SidebarView[], name: string, exceptKey?: string): string | null {
  const cleaned = cleanName(name);
  if (cleaned === "") {
    return "Enter a name.";
  }
  if ([...cleaned].length > VIEW_NAME_MAX) {
    return "That name is too long.";
  }
  const folded = cleaned.toLowerCase();
  if (views.some((view) => view.key !== exceptKey && view.name.toLowerCase() === folded)) {
    return "A view already has that name.";
  }
  return null;
}

export function viewQueryError(query: string): string | null {
  if ([...query.trim()].length > VIEW_QUERY_MAX) {
    return "That query is too long.";
  }
  if (analyzeViewRule(query).invalid) {
    return "The query has an unfinished quote.";
  }
  return null;
}

export function nextCustomKey(views: readonly SidebarView[]): string {
  const max = views.reduce((highest, view) => {
    const rest = view.key.startsWith("custom:") ? Number(view.key.slice("custom:".length)) : 0;
    return Number.isInteger(rest) && rest > highest ? rest : highest;
  }, 0);
  return `custom:${max + 1}`;
}

export function withView(views: readonly SidebarView[], next: SidebarView): SidebarView[] {
  const without = views.filter((view) => view.key !== next.key);
  return normalizeCatalog([...without, next]);
}

export function duplicateOf(views: readonly SidebarView[], source: SidebarView): SidebarView {
  const key = nextCustomKey(views);
  return {
    key,
    name: uniqueName(views, fitName(source.name, " copy")),
    query: source.query,
    builtin: false,
    sort_order: views.length,
    parent: parentKey(source.parent),
  };
}

/** `mine` or `to_review`, or null when the stored parent is missing or not allowed. */
export function parentKey(value: string | null | undefined): ParentKey | null {
  return value === "mine" || value === "to_review" ? value : null;
}

/** Parent rule and the view's own rule, ANDed. A view with no parent is its own query. */
export function effectiveQuery(view: SidebarView, views: readonly SidebarView[]): string {
  const parent = view.builtin ? null : parentKey(view.parent);
  if (!parent) {
    return view.query.trim();
  }
  const base = views.find((item) => item.key === parent)?.query.trim() ?? "";
  return joinRules(base, view.query);
}

/**
 * Sidebar order: each child sits under its parent. Top-level custom views
 * stay after the three defaults, in their saved order.
 */
export function orderedViews(views: readonly SidebarView[]): SidebarView[] {
  const children = (key: ParentKey) => views.filter((view) => !view.builtin && parentKey(view.parent) === key);
  const top = views.filter((view) => !view.builtin && parentKey(view.parent) === null);
  const byKey = (key: string) => views.find((view) => view.key === key) ?? null;
  return [byKey("mine"), ...children("mine"), byKey("to_review"), ...children("to_review"), byKey("bumps_due"), ...top].filter(
    (view): view is SidebarView => view !== null,
  );
}

/**
 * What New view starts from. A view opened under My PRs or To review (or one
 * of their children) keeps that parent and stores only the extra rule. Anywhere
 * else, the field is the whole effective filter. Duplicate does not use this.
 */
export function newViewDraft(views: readonly SidebarView[], activeKey: string, search: string): { parent: ParentKey | null; query: string } {
  const active = views.find((view) => view.key === activeKey) ?? null;
  const parent = active ? (active.builtin ? parentKey(active.key) : parentKey(active.parent)) : null;
  const base = active ? effectiveQuery(active, views) : "";
  const extra = withoutTokens(search, base);
  if (!parent) {
    return { parent: null, query: joinRules(base, extra) };
  }
  const own = active && !active.builtin ? active.query.trim() : "";
  return { parent, query: joinRules(own, extra) };
}

/** Search text that is not already part of the view rule. */
function withoutTokens(search: string, base: string): string {
  const skip = new Set(base.trim().toLowerCase().split(/\s+/u).filter((part) => part !== ""));
  return search
    .trim()
    .split(/\s+/u)
    .filter((part) => part !== "" && !skip.has(part.toLowerCase()))
    .join(" ");
}

/** Joins two rules, dropping tokens the base already has. */
export function joinRules(base: string, extra: string): string {
  const have = new Set(base.trim().toLowerCase().split(/\s+/u).filter((part) => part !== ""));
  let out = base.trim();
  for (const part of extra.trim().split(/\s+/u)) {
    if (part === "" || have.has(part.toLowerCase())) {
      continue;
    }
    out = out === "" ? part : `${out} ${part}`;
    have.add(part.toLowerCase());
  }
  return out;
}

function cleanName(raw: string): string {
  return raw.trim().split(/\s+/u).filter((part) => part !== "").join(" ");
}

function capQuery(raw: string): string {
  const trimmed = raw.trim();
  return [...trimmed].slice(0, VIEW_QUERY_MAX).join("");
}

function nameTaken(views: readonly SidebarView[], name: string): boolean {
  const folded = name.toLowerCase();
  return views.some((view) => view.name.toLowerCase() === folded);
}

/** `base` cut so that `base + suffix` fits VIEW_NAME_MAX characters, which the backend enforces. */
function fitName(base: string, suffix = ""): string {
  return [...base].slice(0, VIEW_NAME_MAX - [...suffix].length).join("").trimEnd() + suffix;
}

function uniqueName(views: readonly SidebarView[], name: string): string {
  const fitted = fitName(name);
  if (!nameTaken(views, fitted)) {
    return fitted;
  }
  for (let index = 2; index < 1000; index += 1) {
    const next = fitName(name, ` ${index}`);
    if (!nameTaken(views, next)) {
      return next;
    }
  }
  return fitted;
}

function validCustomKey(key: string): boolean {
  return /^custom:[1-9][0-9]{0,7}$/u.test(key);
}

export function builtinKey(key: string): boolean {
  return (BUILTIN_KEYS as readonly string[]).includes(key);
}
