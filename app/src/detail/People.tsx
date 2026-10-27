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

/** Who reviewed or still has to: one chip each, with the state as a coloured ring. */
export function Reviewers({ reviewers }: { reviewers: OverviewReviewer[] }) {
  if (reviewers.length === 0) {
    return <p className="text-xs text-slate-500">No reviewers yet.</p>;
  }
  return (
    <ul className="flex flex-wrap gap-2" aria-label="Reviewers">
      {reviewers.map((reviewer) => {
        const state = REVIEW_STATE[reviewer.state];
        const shown = reviewer.kind === "team" ? (reviewer.name ?? reviewer.login.split("/").pop() ?? reviewer.login) : reviewer.login;
        return (
          <li key={`${reviewer.kind}:${reviewer.login}`}>
            <Tooltip label={reviewerLine(reviewer)} detail={state.label}>
              <span
                tabIndex={0}
                data-state={reviewer.state}
                className="flex items-center gap-2 rounded-full border border-slate-800 bg-slate-900/60 py-0.5 pl-0.5 pr-2.5 text-xs outline-none focus-visible:ring-2 focus-visible:ring-violet-400"
              >
                <span className="p-0.5">
                  <Avatar person={reviewer} size={20} ring={`${state.ring} ring-offset-1 ring-offset-slate-900`} />
                </span>
                <span className="max-w-[9rem] truncate text-slate-200">{shown}</span>
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
  const lines = participants.map(reviewerLine);
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
