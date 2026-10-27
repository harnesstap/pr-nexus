import type { Api, ProjectDto, ProjectLinkDto, ProjectRuleDto } from "./api";
import { candidatesFromRows, pageFromRows, type SearchRow } from "./list/paging";

type ProjectApi = Pick<
  Api,
  | "setProjectColor"
  | "listProjectRules"
  | "saveProjectRule"
  | "deleteProjectRule"
  | "previewRule"
  | "listProjectLinks"
  | "linkPrs"
  | "unlinkPr"
  | "restorePr"
  | "listPage"
  | "rowsById"
  | "linkCandidates"
>;

/**
 * In-memory project links and rules for the browser preview and tests. Rows'
 * `project_ids` are kept in sync; rule matching is a plain substring check.
 * Paging and search run the TypeScript reference of the backend (`paging.ts`).
 */
export function memoryProjectApi(rows: SearchRow[], projects: ProjectDto[], rules: ProjectRuleDto[] = []): ProjectApi {
  const links: (ProjectLinkDto & { project_id: number })[] = rows.flatMap((row) =>
    row.project_ids.map((project_id) => ({ project_id, pr_id: row.id, source: "manual" as const, rule_id: null })),
  );
  let nextRule = Math.max(0, ...rules.map((rule) => rule.id ?? 0)) + 1;
  const sync = () => {
    for (const row of rows) {
      row.project_ids = links.filter((link) => link.pr_id === row.id && link.source !== "excluded").map((link) => link.project_id);
    }
  };
  const find = (projectId: number, prId: number) => links.findIndex((link) => link.project_id === projectId && link.pr_id === prId);
  const strip = ({ body: _body, ...row }: SearchRow) => (void _body, row);
  return {
    listPage: (request) => Promise.resolve(structuredClone(pageFromRows(rows, projects, request))),
    rowsById: (ids) => {
      const byId = new Map(rows.map((row) => [row.id, row]));
      return Promise.resolve(ids.flatMap((id) => (byId.has(id) ? [strip({ ...byId.get(id)! })] : [])));
    },
    linkCandidates: (projectId, query, limit) => {
      const linked = new Set(links.filter((link) => link.project_id === projectId && link.source !== "excluded").map((link) => link.pr_id));
      return Promise.resolve(candidatesFromRows(rows, projects, linked, query, limit));
    },
    setProjectColor: (id, color) => {
      const project = projects.find((item) => item.id === id);
      if (!project) {
        return Promise.reject(new Error("Project not found"));
      }
      project.color = color;
      return Promise.resolve({ ...project });
    },
    listProjectRules: (projectId) => Promise.resolve(rules.filter((rule) => rule.project_id === projectId).map((rule) => ({ ...rule }))),
    saveProjectRule: (rule) => {
      if (!rule.pattern.trim()) {
        return Promise.reject(new Error("Pattern cannot be empty"));
      }
      const saved = { ...rule, pattern: rule.pattern.trim(), id: rule.id ?? nextRule++ };
      const index = rules.findIndex((item) => item.id === saved.id);
      if (index >= 0) {
        rules[index] = saved;
      } else {
        rules.push(saved);
      }
      return Promise.resolve(saved);
    },
    deleteProjectRule: (id) => {
      const index = rules.findIndex((rule) => rule.id === id);
      if (index >= 0) {
        rules.splice(index, 1);
      }
      return Promise.resolve();
    },
    previewRule: (_kind, pattern) => {
      const needle = pattern.replaceAll("*", "").toLowerCase();
      return Promise.resolve(rows.filter((row) => `${row.title} ${row.repo}`.toLowerCase().includes(needle)).length);
    },
    listProjectLinks: (projectId) =>
      Promise.resolve(
        links.filter((link) => link.project_id === projectId).map(({ pr_id, source, rule_id }) => ({ pr_id, source, rule_id })),
      ),
    linkPrs: (projectId, prIds) => {
      let changed = 0;
      for (const prId of prIds) {
        const index = find(projectId, prId);
        if (index >= 0 && links[index].source === "manual") {
          continue;
        }
        if (index >= 0) {
          links.splice(index, 1);
        }
        links.push({ project_id: projectId, pr_id: prId, source: "manual", rule_id: null });
        changed += 1;
      }
      sync();
      return Promise.resolve(changed);
    },
    unlinkPr: (projectId, prId) => {
      const index = find(projectId, prId);
      if (index >= 0) {
        if (links[index].source === "rule") {
          links[index] = { ...links[index], source: "excluded", rule_id: null };
        } else {
          links.splice(index, 1);
        }
      }
      sync();
      return Promise.resolve();
    },
    restorePr: (projectId, prId) => {
      const index = find(projectId, prId);
      if (index >= 0 && links[index].source === "excluded") {
        links.splice(index, 1);
      }
      sync();
      return Promise.resolve();
    },
  };
}
