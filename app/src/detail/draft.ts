import type { ListRow, PrOverview } from "../api";

const WRITE = new Set(["admin", "maintain", "write"]);

export interface DraftView {
  /** True when the PR is a draft now. */
  draft: boolean;
  available: boolean;
  /** Why the toggle is off; null when it is available. */
  reason: string | null;
  /** What a click does, for the button and tooltip. */
  action: string;
}

/**
 * Whether the viewer can flip draft ↔ ready. GitHub lets the author and
 * anyone with write access do it, only while the PR is open.
 */
export function draftView(row: ListRow, overview: PrOverview | null): DraftView {
  const draft = overview ? overview.is_draft : row.is_draft;
  const action = draft ? "Mark ready for review" : "Convert to draft";
  const state = overview?.state ?? row.state;
  const off = (reason: string): DraftView => ({ draft, available: false, reason, action });
  if (state === "merged") return off("It is already merged.");
  if (state === "closed") return off("The pull request is closed.");
  if (!overview) return off("Still loading the pull request.");
  const author = overview.viewer.login !== "" && overview.viewer.login.toLowerCase() === (row.author_login ?? "").toLowerCase();
  const permission = overview.viewer.permission;
  // A hidden permission (null) is not proof of no access: let GitHub decide.
  if (!author && permission !== null && !WRITE.has(permission)) {
    return off("Only the author or someone with write access can change this.");
  }
  return { draft, available: true, reason: null, action };
}
