import { useState, type ReactNode } from "react";
import type { OverviewCheck, PrOverview } from "../api";
import { AlertIcon, CheckIcon, ClockIcon, CommentIcon, DeclinedIcon, MergeIcon } from "../icons";
import { Tooltip } from "../Tooltip";
import { Avatar } from "./Avatar";
import { approvalsSummary, checksSummary, commentsSummary, conflictsSummary, TONE_BAR, TONE_TEXT, type Summary } from "./status";

interface Props {
  overview: PrOverview;
  onOpen: (url: string) => void;
}

/** The four at-a-glance cards; the checks card lists what is failing when clicked. */
export function StatCards({ overview, onOpen }: Props) {
  const [showChecks, setShowChecks] = useState(false);
  const checks = checksSummary(overview);
  const approvals = approvalsSummary(overview);
  const approvers = overview.reviewers.filter((reviewer) => reviewer.state === "approved");
  const percent = checks.total === 0 ? 0 : Math.round((checks.ok / checks.total) * 100);
  return (
    <div className="@container">
      <div className="grid auto-rows-fr grid-cols-2 gap-2 @lg:grid-cols-4" data-testid="stat-cards">
        <button
          type="button"
          aria-expanded={showChecks}
          aria-controls="detail-checks"
          aria-label={`Checks: ${checks.value} passed, ${checks.caption}`}
          disabled={checks.attention.length === 0}
          onClick={() => setShowChecks((open) => !open)}
          className={`${CARD} text-left enabled:hover:border-slate-600 disabled:cursor-default`}
        >
          <CardHead icon={<CheckIcon size={13} />} label="Checks" />
          <p className={`${VALUE} ${TONE_TEXT[checks.tone]}`}>{checks.value}</p>
          <div className={`${BAR_SLOT} overflow-hidden rounded-full bg-slate-800`} aria-hidden="true">
            <div className={`h-full rounded-full ${TONE_BAR[checks.tone]}`} style={{ width: `${percent}%` }} />
          </div>
          <p className={CAPTION}>{checks.caption}</p>
        </button>
        <Card icon={<CommentIcon size={13} />} label="Comments" summary={commentsSummary(overview)} />
        <Card
          icon={<CheckIcon size={13} />}
          label="Approvals"
          summary={approvals}
          hint={approvals.requiredKnown ? undefined : "GitHub only shows classic branch protection to admins, so the number of required approvals is unknown."}
        >
          {approvers.length > 0 && (
            <span className="ml-auto flex -space-x-1" aria-label={`Approved by ${approvers.map((item) => item.login).join(", ")}`}>
              {approvers.slice(0, 3).map((person) => (
                <Avatar key={person.login} person={person} size={16} />
              ))}
            </span>
          )}
        </Card>
        <Card icon={<MergeIcon size={13} />} label="Conflicts" summary={conflictsSummary(overview)} />
      </div>
      {showChecks && checks.attention.length > 0 && (
        <ul
          id="detail-checks"
          aria-label="Checks needing attention"
          className="mt-2 divide-y divide-slate-800 rounded-lg border border-slate-800 bg-slate-900/50"
        >
          {checks.attention.map((check) => (
            <CheckLine key={check.name} check={check} onOpen={onOpen} />
          ))}
        </ul>
      )}
    </div>
  );
}

/** Every card shares one frame so the grid stays uniform whatever the content. */
const CARD = "flex h-full w-full min-w-0 flex-col rounded-lg border border-slate-800 bg-slate-900/70 p-3";
const VALUE = "mt-1 h-7 truncate text-lg font-semibold leading-7 tabular-nums";
const BAR_SLOT = "mt-1 h-1 shrink-0";
/** Two caption lines are always reserved, so a short caption does not shrink a card. */
const CAPTION = "mt-1 line-clamp-2 min-h-8 text-xs leading-4 text-slate-500";

function CardHead({ icon, label }: { icon: ReactNode; label: string }) {
  return (
    <p className="flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-wide text-slate-500">
      {icon}
      {label}
    </p>
  );
}

function Card({ icon, label, summary, hint, children }: { icon: ReactNode; label: string; summary: Summary; hint?: string; children?: ReactNode }) {
  const body = (
    <div
      tabIndex={hint ? 0 : undefined}
      aria-label={`${label}: ${summary.value}, ${summary.caption}`}
      className={`${CARD} outline-none focus-visible:ring-2 focus-visible:ring-violet-400`}
    >
      <CardHead icon={icon} label={label} />
      <p className={`${VALUE} flex items-center gap-2 ${TONE_TEXT[summary.tone]}`}>
        {summary.value}
        {children}
      </p>
      {/* The empty bar slot keeps captions level with the Checks card. */}
      <div className={BAR_SLOT} aria-hidden="true" />
      <p className={CAPTION}>{summary.caption}</p>
    </div>
  );
  return hint ? (
    <Tooltip label={`${label}: ${summary.value}`} detail={hint} className="h-full w-full">
      {body}
    </Tooltip>
  ) : (
    body
  );
}

function CheckLine({ check, onOpen }: { check: OverviewCheck; onOpen: (url: string) => void }) {
  const failed = check.state === "failed";
  return (
    <li className="flex items-center gap-2 px-3 py-1.5 text-xs">
      <span className={failed ? "text-rose-300" : "text-amber-300"}>{failed ? <DeclinedIcon size={13} /> : <ClockIcon size={13} />}</span>
      <span className="min-w-0 flex-1 truncate text-slate-200">{check.name}</span>
      {check.required && <span className="rounded bg-slate-800 px-1 text-[10px] text-slate-400">Required</span>}
      <span className={failed ? "text-rose-300" : "text-amber-300"}>{failed ? "Failed" : "Running"}</span>
      {check.url && (
        <button type="button" className="text-violet-300 hover:underline" onClick={() => onOpen(check.url ?? "")}>
          Logs ↗
        </button>
      )}
    </li>
  );
}

export function VerdictIcon({ tone }: { tone: Summary["tone"] }) {
  if (tone === "good") {
    return <CheckIcon size={16} />;
  }
  if (tone === "bad") {
    return <DeclinedIcon size={16} />;
  }
  return <AlertIcon size={16} />;
}
