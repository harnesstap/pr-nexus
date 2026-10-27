import { useState } from "react";
import type { PendingReviewer } from "../api";
import { Tooltip } from "../Tooltip";

/** Avatars drawn before the rest fold into a "+N" chip. */
export const MAX_AVATARS = 3;
/** Pixel size of each avatar; the stack never grows taller than the row's text. */
const SIZE = 20;
/** Requested from GitHub at 2x for sharp avatars on Retina screens. */
const FETCH_SIZE = 64;

/** The avatars to draw and how many fold into the "+N" chip. */
export function stackLayout(reviewers: PendingReviewer[]): { shown: PendingReviewer[]; more: number } {
  if (reviewers.length <= MAX_AVATARS) {
    return { shown: reviewers, more: 0 };
  }
  return { shown: reviewers.slice(0, MAX_AVATARS), more: reviewers.length - MAX_AVATARS };
}

/** One tooltip line: "Name (@login)" for a user, "Team name (org/slug)" for a team. */
export function reviewerLine(reviewer: PendingReviewer): string {
  if (reviewer.kind === "team") {
    return reviewer.name ? `${reviewer.name} (${reviewer.login})` : reviewer.login;
  }
  return reviewer.name ? `${reviewer.name} (@${reviewer.login})` : `@${reviewer.login}`;
}

/** Up to two letters for the fallback badge. */
export function initials(reviewer: PendingReviewer): string {
  const source = reviewer.name ?? (reviewer.kind === "team" ? (reviewer.login.split("/").pop() ?? "") : reviewer.login);
  const words = source.split(/[\s_-]+/).filter(Boolean);
  const letters = words.length > 1 ? words[0][0] + words[1][0] : source.slice(0, 2);
  return letters.toUpperCase() || "?";
}

/** Keeps the avatar request small even if a stored URL lost its size parameter. */
export function sizedAvatar(url: string): string {
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== "https:" && parsed.protocol !== "http:") {
      return url;
    }
    if (!parsed.searchParams.has("s") && !parsed.searchParams.has("size")) {
      parsed.searchParams.set("s", String(FETCH_SIZE));
    }
    return parsed.toString();
  } catch {
    return url;
  }
}

/** Overlapping avatars of who still has to review, with every name in the tooltip. */
export function ReviewerAvatars({ reviewers }: { reviewers: PendingReviewer[] }) {
  if (reviewers.length === 0) {
    return null;
  }
  const { shown, more } = stackLayout(reviewers);
  const label = `Waiting on ${reviewers.length} reviewer${reviewers.length === 1 ? "" : "s"}`;
  return (
    <Tooltip label={label} lines={reviewers.map(reviewerLine)} side="bottom" align="end">
      <span
        role="group"
        tabIndex={0}
        aria-label={label}
        data-testid="reviewer-avatars"
        className="flex h-5 items-center -space-x-1 rounded-full outline-none focus-visible:ring-2 focus-visible:ring-violet-400"
      >
        {shown.map((reviewer) => (
          <Avatar key={`${reviewer.kind}:${reviewer.login}`} reviewer={reviewer} />
        ))}
        {more > 0 && (
          <span
            data-testid="reviewer-more"
            className="relative inline-flex h-5 min-w-5 items-center justify-center rounded-full bg-slate-700 px-1 text-[10px] font-medium leading-none text-slate-200 ring-2 ring-slate-950"
          >
            +{more}
          </span>
        )}
      </span>
    </Tooltip>
  );
}

function Avatar({ reviewer }: { reviewer: PendingReviewer }) {
  const [failed, setFailed] = useState(false);
  // Teams are squares, users circles, so a fallback still says which is which.
  const shape = reviewer.kind === "team" ? "rounded-md" : "rounded-full";
  const ring = `relative inline-flex h-5 w-5 shrink-0 overflow-hidden ring-2 ring-slate-950 ${shape}`;
  if (reviewer.avatar_url && !failed) {
    return (
      <img
        src={sizedAvatar(reviewer.avatar_url)}
        alt=""
        width={SIZE}
        height={SIZE}
        loading="lazy"
        draggable={false}
        onError={() => setFailed(true)}
        data-reviewer={reviewer.login}
        className={`${ring} bg-slate-800 object-cover`}
      />
    );
  }
  return (
    <span
      aria-hidden="true"
      data-reviewer={reviewer.login}
      className={`${ring} items-center justify-center text-[9px] font-semibold leading-none ${reviewer.kind === "team" ? "bg-indigo-900 text-indigo-200" : "bg-slate-700 text-slate-200"}`}
    >
      {initials(reviewer)}
    </span>
  );
}
