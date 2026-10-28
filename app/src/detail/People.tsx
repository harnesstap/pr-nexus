import type { OverviewReviewer, Person } from "../api";
import { reviewerLine } from "../list/ReviewerAvatars";
import { Tooltip } from "../Tooltip";
import { Avatar } from "./Avatar";

export const REVIEW_STATE: Record<OverviewReviewer["state"], { label: string; ring: string; text: string }> = {
  approved: { label: "Approved", ring: "ring-2 ring-teal-400", text: "text-teal-300" },
  changes_requested: { label: "Changes requested", ring: "ring-2 ring-rose-400", text: "text-rose-300" },
  commented: { label: "Commented", ring: "ring-2 ring-violet-400", text: "text-violet-300" },
  pending: { label: "Waiting", ring: "ring-2 ring-amber-400/80", text: "text-amber-300" },
  dismissed: { label: "Dismissed", ring: "ring-2 ring-slate-500", text: "text-slate-400" },
};

/** A pending seat GitHub didn't list as a request: a code-owner team the rules require. */
export interface RequiredSeat extends OverviewReviewer {
  required: true;
}

/**
 * Code-owner teams still owed a review, as pending seats, unless GitHub already lists
 * them as reviewers or they approved. Team names come as `@org/slug` or `org/slug`.
 */
export function requiredSeats(reviewers: OverviewReviewer[], mandatory: string[], approved: string[]): RequiredSeat[] {
  const bare = (team: string) => team.replace(/^@/, "").toLowerCase();
  const known = new Set([...reviewers.filter((r) => r.kind === "team").map((r) => bare(r.login)), ...approved.map(bare)]);
  const seen = new Set<string>();
  return mandatory
    .map((team) => team.replace(/^@/, ""))
    .filter((team) => !known.has(bare(team)) && !seen.has(bare(team)) && Boolean(seen.add(bare(team))))
    .map((login) => ({ kind: "team", login, name: null, avatar_url: null, state: "pending", required: true }));
}

interface ReviewersProps {
  reviewers: OverviewReviewer[];
  /** Code-owner teams that must still review, shown as pending. */
  required?: RequiredSeat[];
  /** Approvals still missing under the branch rules, when known. */
  missing?: number;
}

/** Who reviewed or still has to: one chip each, with the state as a coloured ring. */
export function Reviewers({ reviewers, required = [], missing = 0 }: ReviewersProps) {
  const seats: OverviewReviewer[] = [...reviewers, ...required];
  if (seats.length === 0) {
    return (
      <p className="text-xs text-slate-500">
        No reviewers yet.
        {missing > 0 && <span className="text-amber-300"> {missing} approval{missing === 1 ? "" : "s"} needed: request someone.</span>}
      </p>
    );
  }
  return (
    <ul className="flex flex-wrap gap-2" aria-label="Reviewers">
      {seats.map((reviewer) => {
        const owner = "required" in reviewer;
        const state = owner ? { ...REVIEW_STATE.pending, label: "Code owner · waiting" } : REVIEW_STATE[reviewer.state];
        const shown = reviewer.kind === "team" ? (reviewer.name ?? reviewer.login.split("/").pop() ?? reviewer.login) : reviewer.login;
        return (
          <li key={`${reviewer.kind}:${reviewer.login}`}>
            <Tooltip label={reviewerLine(reviewer)} detail={owner ? "Required by CODEOWNERS; not requested on GitHub yet" : state.label}>
              <span
                tabIndex={0}
                data-state={reviewer.state}
                className="flex items-center gap-2 rounded-full border border-slate-800 bg-slate-900/60 py-0.5 pl-0.5 pr-2.5 text-xs outline-none focus-visible:ring-2 focus-visible:ring-violet-400"
              >
                <span className="p-0.5">
                  <Avatar person={reviewer} size={20} ring={`${state.ring} ring-offset-1 ring-offset-slate-900`} />
                </span>
                <span className="max-w-[9rem] truncate text-slate-200">{shown}</span>
                {reviewer.kind === "bot" && <BotTag />}
                <span className={state.text}>{state.label}</span>
              </span>
            </Tooltip>
          </li>
        );
      })}
    </ul>
  );
}

/** Avatars drawn before the rest fold into "+N". */
export const MAX_PARTICIPANTS = 6;

/** The avatars to draw and the "+N" count, which includes people GitHub did not list. */
export function participantStack(participants: Person[], total: number): { shown: Person[]; more: number } {
  const shown = participants.slice(0, MAX_PARTICIPANTS);
  return { shown, more: Math.max(total, participants.length) - shown.length };
}

/** Everyone who took part, as overlapping avatars with every name in the tooltip. */
export function Participants({ participants, total }: { participants: Person[]; total: number }) {
  if (participants.length === 0) {
    return <p className="text-xs text-slate-500">No one yet.</p>;
  }
  const { shown, more } = participantStack(participants, total);
  const count = Math.max(total, participants.length);
  const hiddenByGithub = count - participants.length;
  const lines = participants.map((person) => (person.kind === "bot" ? `${reviewerLine(person)} · bot` : reviewerLine(person)));
  if (hiddenByGithub > 0) {
    lines.push(`and ${hiddenByGithub} more`);
  }
  const label = `${count} participant${count === 1 ? "" : "s"}`;
  return (
    <Tooltip label={label} lines={lines} align="start">
      <span role="group" tabIndex={0} aria-label={label} className="flex w-fit items-center -space-x-1.5 rounded-full outline-none focus-visible:ring-2 focus-visible:ring-violet-400">
        {shown.map((person) => (
          <Avatar key={`${person.kind}:${person.login}`} person={person} size={24} />
        ))}
        {more > 0 && (
          <span
            data-testid="participants-more"
            className="relative inline-flex h-6 min-w-6 items-center justify-center rounded-full bg-slate-700 px-1.5 text-[11px] font-medium text-slate-200 ring-2 ring-slate-900"
          >
            +{more}
          </span>
        )}
      </span>
    </Tooltip>
  );
}

function BotTag() {
  return <span className="rounded bg-slate-700/70 px-1 text-[10px] uppercase tracking-wide text-slate-300">bot</span>;
}
