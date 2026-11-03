import type { ListRow } from "./api";
import type { SearchRow } from "./list/paging";

const HOUR = 60 * 60 * 1000;

function slack(requestedHoursAgo: number, bumpedHoursAgo: number | null, id: number): ListRow["slack"] {
  return {
    requested_at: new Date(Date.now() - requestedHoursAgo * HOUR).toISOString(),
    last_bumped_at: bumpedHoursAgo === null ? null : new Date(Date.now() - bumpedHoursAgo * HOUR).toISOString(),
    bump_count: bumpedHoursAgo === null ? 0 : 1,
    workspace: "acme",
    channel_id: "C0ACME0001",
    channel_name: "team-reviews",
    permalink: `https://acme.slack.com/archives/C0ACME0001/p17000000000${String(id).padStart(5, "0")}`,
    root_ts: `1700000000.${String(id).padStart(6, "0")}`,
    source: "app",
  };
}

const dana = { kind: "user" as const, login: "dana", name: "Dana Example", avatar_url: null };
const lee = { kind: "user" as const, login: "lee", name: "Lee Sample", avatar_url: null };

/**
 * `?rows=varied` in the preview: one PR per next action (§8.2), mine and to
 * review, so the badges and the "Needs attention" sort can be checked by eye.
 */
export function variedRows(row: (partial: Partial<SearchRow> & Pick<SearchRow, "id" | "title" | "number">) => SearchRow): SearchRow[] {
  const ago = (hours: number) => new Date(Date.now() - hours * HOUR).toISOString();
  const asked = { reviewers_requested: true, pending_reviewers: [dana], review_decision: "review_required" };
  return [
    row({ id: 11, number: 201, title: "Rework session storage", ...asked, conflicts: true, readiness: "blocked", created_at: ago(90) }),
    row({ id: 12, number: 202, title: "Upgrade the payment client", ...asked, checks: "failure", readiness: "blocked", created_at: ago(40), project_ids: [2], repo: "acme/billing" }),
    row({ id: 13, number: 203, title: "Split invoice exporter", review_decision: "changes_requested", checks: "success", unresolved_threads: 3, total_threads: 5, created_at: ago(70), project_ids: [2], repo: "acme/billing" }),
    row({ id: 14, number: 204, title: "Add audit log retention", review_decision: "approved", created_at: ago(30) }),
    row({ id: 15, number: 205, title: "Prototype magic links", is_draft: true, review_decision: null, created_at: ago(10) }),
    row({ id: 16, number: 206, title: "Tidy feature flag names", review_decision: null, created_at: ago(5), project_ids: [] }),
    row({ id: 17, number: 207, title: "Cache org settings", ...asked, pending_reviewers: [dana, lee], slack: slack(26 * 24, 30, 17), created_at: ago(28 * 24) }),
    row({ id: 18, number: 208, title: "Retry webhook deliveries", ...asked, slack: slack(3, null, 18), created_at: ago(6) }),
    row({ id: 19, number: 209, title: "Bump the date library", review_decision: "approved", checks: "pending", created_at: ago(4) }),
    row({ id: 21, number: 311, title: "Speed up search indexing", ...asked, involvement: 2, author_login: "lee", created_at: ago(80) }),
    row({ id: 22, number: 312, title: "Add CSV import", ...asked, involvement: 2, author_login: "dana", created_at: ago(8) }),
    row({ id: 23, number: 313, title: "Drop the legacy flag", ...asked, involvement: 2, author_login: "lee", review_decision: "changes_requested", created_at: ago(30) }),
    row({ id: 24, number: 314, title: "Rotate signing keys", involvement: 2, author_login: "dana", review_decision: "approved", created_at: ago(20) }),
  ];
}
