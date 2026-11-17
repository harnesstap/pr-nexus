import { GitHubIcon, StarIcon } from "../icons";
import { Tooltip } from "../Tooltip";
import { hint } from "./shortcuts";

/**
 * The row's star: toggles without opening the panel, shown in full and compact rows.
 * `shortcut` is the focused row only: Shift+s stars that row, not the one under the pointer.
 */
export function StarButton({
  starred,
  title,
  onToggle,
  size = 14,
  shortcut = true,
}: {
  starred: boolean;
  title: string;
  onToggle: () => void;
  size?: number;
  /** Show the Shift+s chip. The detail header passes the default; a list row passes whether it is focused. */
  shortcut?: boolean;
}) {
  return (
    <Tooltip label={starred ? "Unstar" : "Star"} shortcut={shortcut ? hint("star") : undefined} detail="Starred PRs stay at the top of the list">
      <button
        type="button"
        aria-label={`${starred ? "Unstar" : "Star"} ${title}`}
        aria-pressed={starred}
        className={`flex shrink-0 p-0.5 items-center justify-center rounded hover:bg-slate-800 ${starred ? "text-amber-300" : "text-slate-600 hover:text-slate-300"}`}
        onClick={(event) => {
          event.stopPropagation();
          onToggle();
        }}
      >
        <StarIcon filled={starred} size={size} />
      </button>
    </Tooltip>
  );
}

/** Opens the PR on GitHub straight from its row, without opening the detail panel. */
export function GitHubRowButton({ title, onOpen, size = 14, shortcut = false }: { title: string; onOpen: () => void; size?: number; shortcut?: boolean }) {
  return (
    <Tooltip label="Open on GitHub" shortcut={shortcut ? hint("github") : undefined}>
      <button
        type="button"
        aria-label={`Open ${title} on GitHub`}
        className="flex shrink-0 items-center justify-center rounded p-0.5 text-slate-600 hover:bg-slate-800 hover:text-slate-200 group-hover:text-slate-400 group-focus-within:text-slate-400 focus-visible:text-slate-200"
        onClick={(event) => {
          event.stopPropagation();
          onOpen();
        }}
      >
        <GitHubIcon size={size} />
      </button>
    </Tooltip>
  );
}
