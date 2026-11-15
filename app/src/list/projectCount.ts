/**
 * How many pull requests belong to each sidebar project.
 *
 * One local `list_page` of the open list, grouped by project, is the same
 * set the list shows when that project is selected, before the toolbar search
 * and the menu filters. It does not call GitHub. A failed read is unknown
 * (`null`) for every asked id, which is not the same as zero.
 */
import { defaultView } from "./model";
import { dayBounds, type PageRequest, type PageSection } from "./paging";

export interface ProjectCountPage {
  sections: readonly Pick<PageSection, "project_id" | "count">[];
}

/**
 * A one-row page whose project sections are the sidebar counts.
 * `sidebar-projects` is not Mine, To review, or `project:<id>`, so the open
 * list includes every synced row. Project grouping then splits them the way
 * a project view does.
 */
export function projectCountRequest(now: Date): PageRequest {
  const view = defaultView();
  return {
    view_key: "sidebar-projects",
    grouping: "project",
    ordering: view.ordering,
    direction: view.direction,
    filters: view.filters,
    query: "",
    collapsed: [],
    cursor: null,
    limit: 1,
    now: now.toISOString(),
    ...dayBounds(now),
    view_rule: "",
    due_ids: [],
  };
}

/**
 * One count per project id. Missing from the page means zero (an empty
 * project has no rows). `null` is a failed read.
 */
export async function loadProjectCounts(
  projectIds: readonly number[],
  listPage: (request: PageRequest) => Promise<ProjectCountPage>,
  now: Date,
): Promise<Record<number, number | null>> {
  if (projectIds.length === 0) {
    return {};
  }
  try {
    const page = await listPage(projectCountRequest(now));
    const found = new Map<number, number>();
    for (const section of page.sections) {
      if (section.project_id !== null) {
        found.set(section.project_id, section.count);
      }
    }
    return Object.fromEntries(projectIds.map((id) => [id, found.get(id) ?? 0]));
  } catch {
    return Object.fromEntries(projectIds.map((id) => [id, null]));
  }
}
