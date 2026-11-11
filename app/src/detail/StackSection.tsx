import { useEffect, useState } from "react";
import type { Api, PrStack, StackMemberDto } from "../api";

interface Props {
  api: Api;
  prId: number;
  /** Opens that pull request in this panel. */
  onOpen: (id: number) => void;
}

/** The stack this pull request belongs to, bottom to top. Hidden when it is not stacked. */
export function StackSection({ api, prId, onOpen }: Props) {
  const [stack, setStack] = useState<PrStack | null>(null);

  useEffect(() => {
    if (!api.prStack) {
      return;
    }
    let cancel = false;
    api
      .prStack(prId)
      .then((next) => {
        if (!cancel) {
          setStack(next);
        }
      })
      .catch(() => {
        if (!cancel) {
          setStack(null);
        }
      });
    return () => {
      cancel = true;
    };
  }, [api, prId]);

  if (!stack || stack.members.length < 2) {
    return null;
  }
  const blocking = stack.members.find((member) => member.blocking) ?? stack.members[0];
  return (
    <section aria-label="Stack" className="rounded-lg border border-slate-800 bg-slate-950/40 p-3" data-testid="stack-section">
      <h3 className="text-xs font-medium text-slate-300">Stack</h3>
      <p className="mt-0.5 text-xs text-slate-500">Merge from the bottom. #{blocking?.number} is next.</p>
      <ol className="mt-2 flex flex-col gap-0.5">
        {stack.members.map((member) => (
          <li key={member.id}>
            <StackRow member={member} current={member.id === prId} onOpen={() => onOpen(member.id)} />
          </li>
        ))}
      </ol>
    </section>
  );
}

function StackRow({ member, current, onOpen }: { member: StackMemberDto; current: boolean; onOpen: () => void }) {
  const tone = current ? "bg-violet-500/10" : "hover:bg-slate-800/80";
  return (
    <button
      type="button"
      aria-current={current ? "true" : undefined}
      onClick={onOpen}
      className={`flex w-full flex-col gap-0.5 rounded px-1.5 py-1 text-left ${tone}`}
    >
      <span className="flex min-w-0 items-center gap-2 text-xs">
        <span className="w-4 shrink-0 text-slate-500">{member.position}</span>
        <span className="min-w-0 flex-1 truncate text-slate-200" style={{ paddingLeft: member.depth * 12 }}>
          <span className="text-slate-500">#{member.number}</span> {member.title}
        </span>
        {member.blocking && <span className="shrink-0 rounded bg-slate-800 px-1 text-[11px] text-slate-400">Merge first</span>}
      </span>
      <span className="pl-6 text-[11px] text-slate-500" style={{ paddingLeft: 24 + member.depth * 12 }}>
        {statusLine(member)}
      </span>
    </button>
  );
}

function statusLine(member: StackMemberDto): string {
  return `${ciText(member.checks)} · ${reviewText(member.review_decision)} · ${mergeText(member)}`;
}

function ciText(checks: string): string {
  switch (checks) {
    case "success":
      return "Passing";
    case "failure":
    case "error":
      return "Failing";
    case "pending":
    case "expected":
      return "Pending";
    case "none":
      return "No CI";
    default:
      return "No CI";
  }
}

function reviewText(decision: string | null): string {
  switch (decision) {
    case "approved":
      return "Approved";
    case "changes_requested":
      return "Changes";
    case "review_required":
      return "Review";
    case null:
      return "No review";
    default:
      return "No review";
  }
}

function mergeText(member: StackMemberDto): string {
  if (member.is_draft) {
    return "Draft";
  }
  switch (member.mergeable) {
    case "mergeable":
      return "Mergeable";
    case "conflicting":
      return "Conflicts";
    case "unknown":
      return "Unknown";
    default: {
      const unreachable: never = member.mergeable;
      return unreachable;
    }
  }
}
