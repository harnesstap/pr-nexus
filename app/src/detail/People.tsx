import type { OverviewReviewer, Person, PrOverview } from "../api";
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

/** Avatars drawn per group before the rest fold into "+N". */
export const MAX_STACK = 4;

export type GroupKey = "author" | "reviewers" | "assignees" | "commenters";

export interface PeopleGroup {
  key: GroupKey;
  label: string;
  people: (Person | OverviewReviewer)[];
}

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

function uniq<T extends Person>(people: T[], skip: (person: T) => boolean = () => false): T[] {
  const out: T[] = [];
  for (const person of people) {
    if (!skip(person) && !out.some((known) => known.kind === person.kind && same(known.login, person.login))) {
      out.push(person);
    }
  }
  return out;
}

/**
 * Who is on the pull request, as up to four groups in a fixed order: Author
 * (unless it's the viewer), Reviewers (with pending code-owner seats),
 * Assignees other than the viewer, and Commenters not already shown above nor the viewer.
 * Empty groups are left out.
 */
export function peopleGroups(overview: Pick<PrOverview, "author" | "reviewers" | "assignees" | "commenters" | "viewer">, required: RequiredSeat[] = []): PeopleGroup[] {
  const viewer = overview.viewer.login;
  const author = overview.author && !(viewer && same(overview.author.login, viewer)) ? [overview.author] : [];
  const reviewers = uniq<OverviewReviewer>([...overview.reviewers, ...required]);
  const shown = [...(overview.author ? [overview.author] : []), ...reviewers];
  const commenters = uniq(overview.commenters, (person) => (viewer !== "" && same(person.login, viewer)) || shown.some((known) => same(known.login, person.login)));
  const groups: PeopleGroup[] = [
    { key: "author", label: "Author", people: author },
    { key: "reviewers", label: "Reviewers", people: reviewers },
    { key: "assignees", label: "Assignees", people: uniq(overview.assignees, (person) => viewer !== "" && same(person.login, viewer)) },
    { key: "commenters", label: "Commenters", people: commenters },
  ];
  return groups.filter((group) => group.people.length > 0);
}

/** The status a seat shows: a reviewer's review state, or nothing for plain people. */
function seatState(person: Person | OverviewReviewer) {
  if (!("state" in person)) {
    return null;
  }
  return "required" in person ? { ...REVIEW_STATE.pending, label: "Code owner · waiting" } : REVIEW_STATE[person.state];
}

/** One tooltip line: name, bot marker, and review status for reviewers. */
export function personLine(person: Person | OverviewReviewer): string {
  const state = seatState(person);
  return [reviewerLine(person), person.kind === "bot" ? "bot" : null, state?.label].filter(Boolean).join(" · ");
}

/** The people row: one labelled avatar stack per non-empty group, inline, wrapping when narrow. */
export function PeopleRow({ groups, missing = 0 }: { groups: PeopleGroup[]; missing?: number }) {
  const noReviewers = !groups.some((group) => group.key === "reviewers");
  if (groups.length === 0 && !(noReviewers && missing > 0)) {
    return null;
  }
  return (
    <section aria-label="People" data-testid="people-row" className="space-y-1.5">
      {groups.length > 0 && (
        <div className="flex flex-wrap items-center gap-x-5 gap-y-2">
          {groups.map((group) => (
            <Group key={group.key} group={group} />
          ))}
        </div>
      )}
      {noReviewers && missing > 0 && (
        <p className="text-xs text-amber-300">
          No reviewers yet: {missing} approval{missing === 1 ? "" : "s"} needed.
        </p>
      )}
    </section>
  );
}

function Group({ group }: { group: PeopleGroup }) {
  const shown = group.people.slice(0, MAX_STACK);
  const more = group.people.length - shown.length;
  return (
    <div role="group" aria-label={group.label} data-group={group.key} className="flex items-center gap-2">
      <span className="text-[11px] font-medium uppercase tracking-wide text-slate-500">{group.label}</span>
      <Tooltip label={`${group.label} (${group.people.length})`} lines={group.people.map(personLine)} align="start">
        <span tabIndex={0} className="flex items-center -space-x-1.5 rounded-full outline-none focus-visible:ring-2 focus-visible:ring-violet-400">
          {shown.map((person) => {
            const state = seatState(person);
            return (
              <span key={`${person.kind}:${person.login}`} data-login={person.login} data-state={"state" in person ? person.state : undefined} className="relative inline-flex">
                <Avatar person={person} size={24} ring={state ? `${state.ring} ring-offset-1 ring-offset-slate-900` : undefined} />
              </span>
            );
          })}
          {more > 0 && (
            <span data-testid={`${group.key}-more`} className="relative inline-flex h-6 min-w-6 items-center justify-center rounded-full bg-slate-700 px-1.5 text-[11px] font-medium text-slate-200 ring-2 ring-slate-900">
              +{more}
            </span>
          )}
        </span>
      </Tooltip>
    </div>
  );
}
