/** Which sidebar sections and parent views are folded. Stored as `sidebar.collapse`. */

export interface SidebarCollapse {
  views: boolean;
  projects: boolean;
  /** `mine` and `to_review` whose children are hidden. */
  parents: string[];
}

export const OPEN_SIDEBAR: SidebarCollapse = { views: false, projects: false, parents: [] };

/** A missing or broken value leaves every section open. Unknown parent keys are dropped. */
export function normalizeCollapse(raw: unknown): SidebarCollapse {
  if (!raw || typeof raw !== "object") {
    return { ...OPEN_SIDEBAR, parents: [] };
  }
  const value = raw as { views?: unknown; projects?: unknown; parents?: unknown };
  const parents: string[] = [];
  if (Array.isArray(value.parents)) {
    for (const key of value.parents) {
      if ((key === "mine" || key === "to_review") && !parents.includes(key)) {
        parents.push(key);
      }
    }
  }
  return { views: value.views === true, projects: value.projects === true, parents };
}
