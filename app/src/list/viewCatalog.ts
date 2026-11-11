/**
 * The sidebar's saved views. Built-in keys stay `mine`, `to_review`, and
 * `bumps_due` so a view saved before custom views still loads. Custom keys
 * are `custom:<n>`.
 */
import { BUMP_QUERY, MINE_QUERY, REVIEW_QUERY, VIEW_NAME_MAX, VIEW_QUERY_MAX, analyzeViewRule } from "./viewRule";

export interface SidebarView {
  key: string;
  name: string;
  query: string;
  builtin: boolean;
  sort_order: number;
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
    name: uniqueName(views, `${source.name} copy`),
    query: source.query,
    builtin: false,
    sort_order: views.length,
  };
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

function uniqueName(views: readonly SidebarView[], name: string): string {
  if (!nameTaken(views, name)) {
    return name;
  }
  for (let index = 2; index < 1000; index += 1) {
    const next = `${name} ${index}`;
    if (!nameTaken(views, next)) {
      return next;
    }
  }
  return name;
}

function validCustomKey(key: string): boolean {
  return /^custom:[1-9][0-9]{0,7}$/u.test(key);
}

export function builtinKey(key: string): boolean {
  return (BUILTIN_KEYS as readonly string[]).includes(key);
}
