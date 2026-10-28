import type { Api, LinkOutcomeDto, ProjectDto, ProjectLinkDto, ProjectRuleDto, ReplacedLinkDto } from "./api";
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
  | "undoLinkPrs"
  | "createProjectWithPrs"
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
  const link = (projectId: number, prIds: number[]): LinkOutcomeDto => {
    if (prIds.some((prId) => !rows.some((row) => row.id === prId))) {
      throw new Error("Pull request not found");
    }
    const undo: ReplacedLinkDto[] = [];
    for (const prId of prIds) {
      const index = find(projectId, prId);
      if (index >= 0 && links[index].source === "manual") {
        continue;
      }
      if (index >= 0) {
        const [before] = links.splice(index, 1);
        undo.push({ pr_id: prId, before: before.source === "rule" ? "rule" : "excluded", rule_id: before.rule_id });
      } else {
        undo.push({ pr_id: prId, before: null, rule_id: null });
      }
      links.push({ project_id: projectId, pr_id: prId, source: "manual", rule_id: null });
    }
    sync();
    return { changed: undo.length, undo };
  };
  const undoLink = (projectId: number, undo: ReplacedLinkDto[]) => {
    let reverted = 0;
    for (const item of undo) {
      const index = find(projectId, item.pr_id);
      if (index < 0 || links[index].source !== "manual") {
        continue;
      }
      const ruleKept = item.before === "rule" && rules.some((rule) => rule.id === item.rule_id);
      if (item.before === "excluded" || ruleKept) {
        links[index] = { ...links[index], source: item.before!, rule_id: ruleKept ? item.rule_id : null };
      } else {
        links.splice(index, 1);
      }
      reverted += 1;
    }
    sync();
    return reverted;
  };
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
      try {
        return Promise.resolve(link(projectId, prIds));
      } catch (err) {
        return Promise.reject(err instanceof Error ? err : new Error(String(err)));
      }
    },
    undoLinkPrs: (projectId, undo) => Promise.resolve(undoLink(projectId, undo)),
    createProjectWithPrs: (name, prIds) => {
      const trimmed = name.trim();
      if (!trimmed) {
        return Promise.reject(new Error("Name cannot be empty"));
      }
      if (projects.some((project) => project.name === trimmed)) {
        return Promise.reject(new Error("A project with that name already exists"));
      }
      const project: ProjectDto = {
        id: Math.max(0, ...projects.map((item) => item.id)) + 1,
        name: trimmed,
        color: "#7c5cbf",
        archived: false,
        description: "",
        sort_order: projects.length, jira_links: [] };
      projects.push(project);
      link(project.id, prIds);
      return Promise.resolve({ ...project });
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
