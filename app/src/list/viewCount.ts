/**
 * How many pull requests match a sidebar view.
 *
 * Fully local queries are counted from the synced list, the same way
 * `list_page` builds that view. A query GitHub has to answer uses the last
 * search for that exact query, and only when every reported hit came back.
 * Anything else is unknown: the badge stays blank rather than showing a
 * short page or a zero that means "not loaded".
 */
import { defaultView } from "./model";
import { dayBounds, type PageRequest } from "./paging";
import { analyzeViewRule, isBumpQueueQuery } from "./viewRule";

/** Above this, the badge reads `99+`. */
export const VIEW_COUNT_CAP = 99;

/** The last GitHub search stored for one view. `query` is the view rule. */
export interface RemoteFetch {
  query: string;
  ids: readonly number[];
  total: number;
  ok: boolean;
  openOnly: boolean;
}

export type CountPlan =
  | { kind: "queue" }
  | { kind: "local" }
  | { kind: "remote"; ids: number[]; openOnly: boolean }
  | { kind: "unknown" };

export interface CountPage {
  summary: { open: number; finished: number };
}

/** What to count. The bump queue keeps its own due list. */
export function countPlan(key: string, query: string, remote: RemoteFetch | null): CountPlan {
  if (key === "bumps_due" && isBumpQueueQuery(query)) {
    return { kind: "queue" };
  }
  const rule = analyzeViewRule(query);
  if (rule.invalid || rule.fullyLocal) {
    return { kind: "local" };
  }
  if (!remote || !remote.ok || remote.query.trim() !== query.trim()) {
    return { kind: "unknown" };
  }
  // A search that reported more hits than it returned is a short page.
  if (remote.ids.length < remote.total) {
    return { kind: "unknown" };
  }
  return { kind: "remote", ids: [...remote.ids], openOnly: remote.openOnly };
}

/**
 * A one-row page request whose summary is the view's match count.
 * Remote ids are the membership; a local rule leaves them unset.
 */
export function countRequest(
  viewKey: string,
  query: string,
  dueIds: readonly number[],
  remoteIds: readonly number[] | null,
  openOnly: boolean,
  now: Date,
): PageRequest {
  const view = defaultView();
  return {
    view_key: viewKey,
    grouping: view.grouping,
    ordering: view.ordering,
    direction: view.direction,
    filters: view.filters,
    query: "",
    collapsed: [],
    cursor: null,
    limit: 1,
    now: now.toISOString(),
    ...dayBounds(now),
    view_rule: query,
    due_ids: [...dueIds],
    ...(remoteIds
      ? {
          github_ids: [...remoteIds],
          github_any_state: openOnly === false,
        }
      : {}),
  };
}

/** Rows the list would show: open, plus merged or closed when the query asks. */
export function listedCount(open: number, finished: number): number {
  return open + finished;
}

/** Blank at zero. `99+` above {@link VIEW_COUNT_CAP}. */
export function formatViewCount(count: number): string | null {
  if (count <= 0) {
    return null;
  }
  if (count > VIEW_COUNT_CAP) {
    return `${VIEW_COUNT_CAP}+`;
  }
  return String(count);
}

/**
 * One count per view key. `null` is unknown (or a failed read) and is not
 * the same as zero. The queue view returns `dueCount` and does not list.
 */
export async function loadViewCounts(
  views: readonly { key: string; query: string }[],
  dueCount: number,
  dueIds: readonly number[],
  remotes: Readonly<Record<string, RemoteFetch | undefined>>,
  listPage: (request: PageRequest) => Promise<CountPage>,
  now: Date,
): Promise<Record<string, number | null>> {
  async function readCount(view: { key: string; query: string }, remoteIds: readonly number[] | null, openOnly: boolean): Promise<number | null> {
    try {
      const page = await listPage(countRequest(view.key, view.query, dueIds, remoteIds, openOnly, now));
      return listedCount(page.summary.open, page.summary.finished);
    } catch {
      return null;
    }
  }

  const entries = await Promise.all(
    views.map(async (view) => {
      const plan = countPlan(view.key, view.query, remotes[view.key] ?? null);
      switch (plan.kind) {
        case "queue":
          return [view.key, dueCount] as const;
        case "unknown":
          return [view.key, null] as const;
        case "local":
          return [view.key, await readCount(view, null, true)] as const;
        case "remote":
          return [view.key, await readCount(view, plan.ids, plan.openOnly)] as const;
        default: {
          const unexpected: never = plan;
          void unexpected;
          return [view.key, null] as const;
        }
      }
    }),
  );
  return Object.fromEntries(entries);
}
