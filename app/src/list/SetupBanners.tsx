import { useState } from "react";
import { CloseButton } from "../CloseButton";
import { dismissSetupNotice, reconcileDismissed, type SetupProblem } from "./setupNotices";

const BANNER = "flex flex-wrap items-center gap-3 border-b border-amber-500/40 bg-amber-500/10 px-4 py-2 text-sm text-amber-100";

/** Amber notices in the same stack as the keychain and sync banners. One per problem. */
export function SetupBanners({ problems, onOpen }: { problems: readonly SetupProblem[]; onOpen: (problem: SetupProblem) => void }) {
  const active = problems.map((problem) => problem.id).join("\n");
  const [seen, setSeen] = useState(active);
  const [hidden, setHidden] = useState<ReadonlySet<string>>(() => reconcileDismissed(problems.map((problem) => problem.id)));
  if (seen !== active) {
    setSeen(active);
    setHidden(reconcileDismissed(active.length === 0 ? [] : active.split("\n")));
  }

  const visible = problems.filter((problem) => !hidden.has(problem.id));
  if (visible.length === 0) {
    return null;
  }
  return (
    <>
      {visible.map((problem) => (
        <div key={problem.id} role="alert" data-setup-problem={problem.id} className={BANNER}>
          <span>{problem.message}</span>
          <button type="button" className="underline" onClick={() => onOpen(problem)}>
            {problem.action}
          </button>
          <CloseButton
            className="ml-auto"
            label={`Dismiss: ${problem.message}`}
            shortcut={null}
            onClick={() => {
              dismissSetupNotice(problem.id);
              setHidden((current) => new Set(current).add(problem.id));
            }}
          />
        </div>
      ))}
    </>
  );
}

/** A small warning dot. The text is for assistive tech; the dot is the visual. */
export function AttentionDot() {
  return (
    <span className="inline-flex h-1.5 w-1.5 shrink-0 rounded-full bg-amber-400">
      <span className="sr-only">Needs attention</span>
    </span>
  );
}
