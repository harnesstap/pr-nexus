import type { MergeMethod, OverviewCheck, PrOverview } from "../api";

/** Colour family for a card or line: green, red, amber, or neutral. */
export type Tone = "good" | "bad" | "warn" | "muted";

export interface Summary {
  tone: Tone;
  /** The big value on the card, e.g. "11/12". */
  value: string;
  /** The small line under it, e.g. "1 failing". */
  caption: string;
}

export interface ChecksSummary extends Summary {
  /** Passed or skipped: skipped checks do not block a merge. */
  ok: number;
  total: number;
  /** Failed checks first, then the ones still running. */
  attention: OverviewCheck[];
}

export function checksSummary(overview: Pick<PrOverview, "checks" | "checks_total">): ChecksSummary {
  const count = (state: OverviewCheck["state"]) => overview.checks.filter((check) => check.state === state).length;
  const failed = count("failed");
  const pending = count("pending");
  const ok = count("passed") + count("skipped");
  // GitHub caps the list; the total still counts every check.
  const total = Math.max(overview.checks_total, overview.checks.length);
  const attention = [...overview.checks.filter((check) => check.state === "failed"), ...overview.checks.filter((check) => check.state === "pending")];
  if (total === 0) {
    return {
      tone: "muted",
      value: "0",
      caption: "No checks",
      ok,
      total,
      attention,
    };
  }
  const value = `${ok}/${total}`;
  if (failed > 0) {
    return {
      tone: "bad",
      value,
      caption: `${failed} failing`,
      ok,
      total,
      attention,
    };
  }
  if (pending > 0) {
    return {
      tone: "warn",
      value,
      caption: `${pending} running`,
      ok,
      total,
      attention,
    };
  }
  return { tone: "good", value, caption: "All passing", ok, total, attention };
}

export function commentsSummary(overview: Pick<PrOverview, "issue_comments" | "review_comments">): Summary {
  const total = overview.issue_comments + overview.review_comments;
  const caption = total === 0 ? "No comments" : `${overview.issue_comments} on the PR · ${overview.review_comments} in code`;
  return { tone: "muted", value: String(total), caption };
}

export function approvalsSummary(overview: Pick<PrOverview, "approvals" | "required_approvals" | "review_decision">): Summary & { requiredKnown: boolean } {
  const { approvals, required_approvals: required, review_decision: decision } = overview;
  const requiredKnown = required !== null;
  const value = requiredKnown ? `${approvals}/${required}` : `${approvals}/?`;
  if (decision === "changes_requested") {
    return { tone: "bad", value, caption: "Changes requested", requiredKnown };
  }
  if (decision === "approved" || (requiredKnown && approvals >= required && (required > 0 || approvals > 0))) {
    return { tone: "good", value, caption: "Approved", requiredKnown };
  }
  if (requiredKnown && required > approvals) {
    const missing = required - approvals;
    return {
      tone: "warn",
      value,
      caption: `${missing} more needed`,
      requiredKnown,
    };
  }
  if (decision === "review_required") {
    return { tone: "warn", value, caption: "Review required", requiredKnown };
  }
  return {
    tone: approvals > 0 ? "good" : "muted",
    value,
    caption: approvals > 0 ? "Approved" : "No approvals",
    requiredKnown,
  };
}

/**
 * The Conflicts card: only whether the branch merges cleanly into its base.
 * Checks, reviews and drafts live in the verdict, so "None" here never reads
 * as "ready to merge".
 */
export function conflictsSummary(overview: Pick<PrOverview, "state" | "mergeable" | "merge_state">): Summary {
  if (overview.state === "merged") {
    return {
      tone: "good",
      value: "Merged",
      caption: "Already in the base branch",
    };
  }
  if (overview.state === "closed") {
    return { tone: "muted", value: "Closed", caption: "Not merged" };
  }
  if (overview.mergeable === "conflicting" || overview.merge_state === "dirty") {
    return { tone: "bad", value: "Yes", caption: "Resolve before merging" };
  }
  if (overview.mergeable === "unknown" || overview.merge_state === "unknown") {
    return {
      tone: "muted",
      value: "Checking",
      caption: "GitHub is computing it",
    };
  }
  if (overview.merge_state === "behind") {
    return {
      tone: "warn",
      value: "Behind",
      caption: "Update from the base branch",
    };
  }
  return {
    tone: "good",
    value: "None",
    caption: "Merges cleanly into the base",
  };
}

const WRITE = new Set(["admin", "maintain", "write"]);

export interface MergeVerdict {
  can: boolean;
  tone: Tone;
  /** "You can merge" or "You can't merge". */
  title: string;
  /** Why, in one sentence. */
  reason: string;
  /** Everything in the way, when it can't merge; empty otherwise. */
  blockers: string[];
}

const plural = (count: number, word: string) => `${count} ${word}${count === 1 ? "" : "s"}`;

/** The draft blocker line; the merge card puts "Mark ready for review" on it. */
export const DRAFT_BLOCKER = "It is still a draft.";

/**
 * Every reason an open pull request can't merge yet, in a fixed order: draft,
 * access, conflicts, behind, failing checks, required checks still running,
 * changes requested, and missing approvals.
 */
export function mergeBlockers(overview: PrOverview): string[] {
  if (overview.state !== "open") {
    return [];
  }
  const out: string[] = [];
  if (overview.is_draft || overview.merge_state === "draft") {
    out.push(DRAFT_BLOCKER);
  }
  if (overview.viewer.permission !== null && !WRITE.has(overview.viewer.permission)) {
    out.push("You don't have write access to this repository.");
  }
  if (overview.mergeable === "conflicting" || overview.merge_state === "dirty") {
    out.push("It has merge conflicts with the base branch.");
  }
  if (overview.merge_state === "behind") {
    out.push("The branch is behind its base.");
  }
  const failed = overview.checks.filter((check) => check.state === "failed");
  if (failed.length > 0) {
    const required = failed.filter((check) => check.required).length;
    const names = failed.map((check) => check.name).join(", ");
    out.push(
      `${plural(failed.length, "check")} failing${required > 0 && required < failed.length ? ` (${required} required)` : required > 0 ? "" : " (not required)"}: ${names}.`,
    );
  }
  const running = overview.checks.filter((check) => check.required && check.state === "pending");
  if (running.length > 0) {
    out.push(`${plural(running.length, "required check")} still running: ${running.map((check) => check.name).join(", ")}.`);
  }
  if (overview.review_decision === "changes_requested") {
    out.push("A reviewer requested changes.");
  }
  const required = overview.required_approvals;
  if (required !== null && overview.approvals < required) {
    out.push(`It needs ${plural(required - overview.approvals, "more approval")}.`);
  } else if (overview.review_decision === "review_required") {
    out.push("It is waiting for a required review.");
  }
  return out;
}

/**
 * Whether the signed-in user could press Merge on GitHub right now. GitHub has no exact
 * flag for this, so it is derived from the merge state, the token's permission, and
 * what the branch rules let it see; see the panel tooltip.
 */
export function mergeVerdict(overview: PrOverview): MergeVerdict {
  const no = (reason: string, tone: Tone = "bad"): MergeVerdict => ({
    can: false,
    tone,
    title: "You can't merge",
    reason,
    blockers: mergeBlockers(overview),
  });
  const yes = (reason: string, tone: Tone = "good"): MergeVerdict => ({
    can: true,
    tone,
    title: "You can merge",
    reason,
    blockers: [],
  });
  if (overview.state === "merged") {
    return no("It is already merged.", "muted");
  }
  if (overview.state === "closed") {
    return no("The pull request is closed.", "muted");
  }
  if (overview.is_draft || overview.merge_state === "draft") {
    return no(DRAFT_BLOCKER, "warn");
  }
  if (overview.viewer.permission !== null && !WRITE.has(overview.viewer.permission)) {
    return no("You don't have write access to this repository.", "muted");
  }
  if (overview.mergeable === "conflicting" || overview.merge_state === "dirty") {
    return no("It has merge conflicts with the base branch.");
  }
  switch (overview.merge_state) {
    case "clean":
      return yes("Every branch rule is met.");
    case "has_hooks":
      return yes("Every branch rule is met; GitHub will run the merge hooks.");
    case "unstable":
      return yes("Some checks that are not required are failing.", "warn");
    case "behind":
      return no("The branch is behind its base. Update it first.", "warn");
    case "blocked":
      return overview.viewer.can_merge_as_admin
        ? yes("Branch rules aren't met yet, but you can bypass them as an admin.", "warn")
        : no(blockedReason(overview));
    default:
      return no("GitHub is still working out whether it can merge.", "muted");
  }
}

function blockedReason(overview: PrOverview): string {
  if (overview.review_decision === "changes_requested") {
    return "A reviewer requested changes.";
  }
  const required = overview.required_approvals;
  if (required !== null && overview.approvals < required) {
    const missing = required - overview.approvals;
    return `It needs ${missing} more approval${missing === 1 ? "" : "s"}.`;
  }
  if (overview.review_decision === "review_required") {
    return "It is waiting for a required review.";
  }
  const requiredChecks = overview.checks.filter((check) => check.required);
  if (requiredChecks.some((check) => check.state === "failed")) {
    return "A required check is failing.";
  }
  if (requiredChecks.some((check) => check.state === "pending")) {
    return "A required check is still running.";
  }
  return "A branch rule isn't met yet.";
}

export const METHOD_LABEL: Record<MergeMethod, string> = {
  squash: "Squash and merge",
  merge: "Merge commit",
  rebase: "Rebase and merge",
};

export interface AutoMergeView {
  enabled: boolean;
  /** The control can be used: turn on when off, turn off when on. */
  available: boolean;
  /** Why it can't be used, for the tooltip. */
  reason: string | null;
  /** The methods the repository allows, squash first. */
  methods: MergeMethod[];
  defaultMethod: MergeMethod | null;
}

export function autoMergeView(overview: PrOverview): AutoMergeView {
  const settings = overview.merge_settings;
  const methods = (["squash", "merge", "rebase"] as const).filter((method) => settings[method]);
  const defaultMethod = methods[0] ?? null;
  const base = { methods, defaultMethod };
  if (overview.auto_merge) {
    return overview.viewer.can_disable_auto_merge
      ? { ...base, enabled: true, available: true, reason: null }
      : {
          ...base,
          enabled: true,
          available: false,
          reason: "You can't turn off auto-merge on this pull request.",
        };
  }
  const off = (reason: string): AutoMergeView => ({
    ...base,
    enabled: false,
    available: false,
    reason,
  });
  if (overview.state !== "open") {
    return off("The pull request is not open.");
  }
  if (!settings.auto_merge_allowed) {
    return off("Auto-merge is turned off in this repository's settings.");
  }
  if (overview.is_draft) {
    return off("Drafts can't auto-merge. Mark it ready for review first.");
  }
  if (overview.viewer.permission !== null && !WRITE.has(overview.viewer.permission)) {
    return off("You need write access to enable auto-merge.");
  }
  if (methods.length === 0) {
    return off("The repository allows no merge method.");
  }
  if (!overview.viewer.can_enable_auto_merge) {
    return off(
      overview.merge_state === "clean" || overview.merge_state === "has_hooks"
        ? "Nothing is left to wait for, so GitHub only offers a direct merge."
        : "GitHub doesn't let you enable auto-merge here.",
    );
  }
  return { ...base, enabled: false, available: true, reason: null };
}

/** Whether to offer a direct Merge button: open, mergeable now, and a method to use. */
export function canMergeNow(overview: PrOverview): boolean {
  return overview.state === "open" && !overview.auto_merge && mergeVerdict(overview).can && autoMergeView(overview).methods.length > 0;
}

/** Text and background classes for a tone. */
export const TONE_TEXT: Record<Tone, string> = {
  good: "text-teal-300",
  bad: "text-rose-300",
  warn: "text-amber-300",
  muted: "text-slate-300",
};

export const TONE_BAR: Record<Tone, string> = {
  good: "bg-teal-400",
  bad: "bg-rose-400",
  warn: "bg-amber-400",
  muted: "bg-slate-500",
};
