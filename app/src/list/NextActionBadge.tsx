import type { ReactElement } from "react";
import type { ListRow } from "../api";
import { AlertIcon, CheckIcon, ClockIcon, CommentIcon, DeclinedIcon, DraftIcon, EyeOffIcon, MergeIcon, SlackLogo } from "../icons";
import { Tooltip } from "../Tooltip";
import { nextAction, type NextAction } from "./nextAction";
import { DEFAULT_REVIEW_SLA, type ReviewSla } from "./sla";

/** What clicking the badge does; null when there is nothing to click. */
export type BadgeClick = "slack" | "checks" | "pr" | "files" | "details" | null;

interface BadgeSpec {
  label: string;
  /** Border, background and text classes for the dark theme. */
  className: string;
  icon: (size: number) => ReactElement;
  click: BadgeClick;
  /** Shown after the reason in the tooltip when the badge is clickable. */
  hint?: string;
}

const URGENT = "border-rose-500/40 bg-rose-500/10 text-rose-300";
const WARN = "border-amber-500/40 bg-amber-500/10 text-amber-300";
const GOOD = "border-emerald-500/40 bg-emerald-500/10 text-emerald-300";
const ASK = "border-sky-500/40 bg-sky-500/10 text-sky-300";
const REVIEW = "border-violet-400/50 bg-violet-500/15 text-violet-200";
const QUIET = "border-slate-700 bg-slate-900 text-slate-400";

export const BADGE_SPECS: Record<NextAction, BadgeSpec> = {
  conflicts: { label: "Resolve conflicts", className: URGENT, icon: (size) => <AlertIcon size={size} />, click: "pr", hint: "Click to open the PR on GitHub" },
  ci_failing: { label: "Fix CI", className: URGENT, icon: (size) => <AlertIcon size={size} />, click: "checks", hint: "Click to open the failing checks" },
  changes_requested: { label: "Address review", className: WARN, icon: (size) => <DeclinedIcon size={size} />, click: "pr", hint: "Click to open the PR on GitHub" },
  ready_to_merge: { label: "Ready to merge", className: GOOD, icon: (size) => <MergeIcon size={size} />, click: "details", hint: "Click to open the details panel to merge" },
  review_overdue: { label: "Review now", className: URGENT, icon: (size) => <CommentIcon size={size} />, click: "files", hint: "Click to open the changes on GitHub" },
  draft: { label: "Mark ready", className: QUIET, icon: (size) => <DraftIcon size={size} />, click: "details", hint: "Click to open the details panel" },
  request_review: { label: "Request review", className: ASK, icon: (size) => <EyeOffIcon size={size} />, click: "slack", hint: "Click to request a review on Slack" },
  nudge: { label: "Follow up", className: WARN, icon: (size) => <SlackLogo variant="mono" size={size} />, click: "slack", hint: "Click to send a Slack reminder" },
  review: { label: "Review", className: REVIEW, icon: (size) => <CommentIcon size={size} />, click: "files", hint: "Click to open the changes on GitHub" },
  checks_running: { label: "Checks running", className: QUIET, icon: (size) => <ClockIcon size={size} />, click: "checks", hint: "Click to open the checks" },
  waiting_review: { label: "Waiting for review", className: QUIET, icon: (size) => <ClockIcon size={size} />, click: null },
  waiting_author: { label: "Waiting on author", className: QUIET, icon: (size) => <ClockIcon size={size} />, click: null },
  reviewed: { label: "Approved", className: "border-slate-700 bg-slate-900 text-emerald-400/80", icon: (size) => <CheckIcon size={size} />, click: null },
  author_draft: { label: "Draft", className: QUIET, icon: (size) => <DraftIcon size={size} />, click: null },
  done: { label: "Done", className: QUIET, icon: (size) => <CheckIcon size={size} />, click: null },
};

/** The URL a badge click opens, for the clicks that go to GitHub. */
export function badgeUrl(row: Pick<ListRow, "url">, click: BadgeClick): string | null {
  switch (click) {
    case "checks":
      return `${row.url}/checks`;
    case "files":
      return `${row.url}/files`;
    case "pr":
      return row.url;
    case "slack":
    case "details":
    case null:
      return null;
    default: {
      const unreachable: never = click;
      return unreachable;
    }
  }
}

interface Props {
  row: ListRow;
  viewKey: string;
  now: Date;
  /** Icon only, for the compact row. */
  compact?: boolean;
  /** Review wait from Settings. Defaults to 48 hours, every day. */
  sla?: ReviewSla;
  onOpen: (url: string) => void;
  onRequestReview: (row: ListRow) => void;
  onDetails: (row: ListRow) => void;
}

/** The row's next-action badge (§8.2): label, colour, icon and a tooltip with the reason. */
export function NextActionBadge({ row, viewKey, now, compact = false, sla = DEFAULT_REVIEW_SLA, onOpen, onRequestReview, onDetails }: Props) {
  const { action, reason } = nextAction(row, viewKey, now, sla);
  const spec = BADGE_SPECS[action];
  const label = action === "done" ? reason : spec.label;
  const look = `inline-flex max-w-full items-center gap-1 rounded-full border text-xs font-medium whitespace-nowrap ${compact ? "h-5 w-5 justify-center" : "px-2 py-0.5"} ${spec.className}`;
  const content = (
    <>
      <span aria-hidden="true" className="shrink-0">
        {spec.icon(12)}
      </span>
      {!compact && <span className="truncate">{label}</span>}
    </>
  );
  const click = spec.click;
  const detail = click && spec.hint ? `${reason}. ${spec.hint}` : reason;
  const run = () => {
    if (click === "slack") {
      onRequestReview(row);
    } else if (click === "details") {
      onDetails(row);
    } else {
      const url = badgeUrl(row, click);
      if (url) {
        onOpen(url);
      }
    }
  };
  return (
    <Tooltip label={label} detail={detail} align="end" className="min-w-0">
      {click ? (
        <button type="button" data-action={action} aria-label={`${label}: ${reason}`} className={`${look} cursor-pointer transition hover:brightness-125 focus-visible:outline focus-visible:outline-violet-400`} onClick={run}>
          {content}
        </button>
      ) : (
        <span data-action={action} role="img" aria-label={`${label}: ${reason}`} className={look}>
          {content}
        </span>
      )}
    </Tooltip>
  );
}
