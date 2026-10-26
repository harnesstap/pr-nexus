import type { ListRow, ProjectLinkDto, RuleKind } from "../api";
import { matchesSearch } from "./model";

export const PROJECT_COLORS = ["#7c5cbf", "#3f6f8f", "#2f855a", "#b7791f", "#c05621", "#c53030", "#b83280", "#4a5568"];

export const RULE_KINDS: { id: RuleKind; label: string; placeholder: string; hint: string }[] = [
  { id: "ticket_key_in_branch", label: "Ticket key", placeholder: "PROJ-* or PROJ-123", hint: "Ticket key in the branch or title, * matches anything" },
  { id: "branch_regex", label: "Branch regex", placeholder: "^billing/", hint: "Regex on the head branch" },
  { id: "label", label: "Label", placeholder: "billing", hint: "Exact GitHub label" },
  { id: "repo", label: "Repository", placeholder: "acme/billing-*", hint: "owner/name, * matches anything" },
  { id: "title_regex", label: "Title regex", placeholder: "(?i)refund", hint: "Regex on the PR title" },
];

export function ruleKindLabel(kind: RuleKind): string {
  return RULE_KINDS.find((item) => item.id === kind)?.label ?? kind;
}

/** PRs that can be added by hand: not already linked (excluded ones can be re-added). */
export function linkCandidates(rows: readonly ListRow[], links: readonly ProjectLinkDto[], query: string, limit = 8): ListRow[] {
  if (!query.trim()) {
    return [];
  }
  const linked = new Set(links.filter((link) => link.source !== "excluded").map((link) => link.pr_id));
  const needle = query.trim().replace(/^#/, "");
  return rows
    .filter((row) => !linked.has(row.id) && !row.hidden)
    .filter((row) => String(row.number) === needle || matchesSearch(row, [], query))
    .slice(0, limit);
}
