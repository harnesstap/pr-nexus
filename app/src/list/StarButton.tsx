import { StarIcon } from "../icons";
import { Tooltip } from "../Tooltip";
import { hint } from "./shortcuts";

/** The row's star: toggles without opening the panel, shown in full and compact rows. */
export function StarButton({ starred, title, onToggle, size = 14 }: { starred: boolean; title: string; onToggle: () => void; size?: number }) {
  return (
    <Tooltip label={starred ? "Unstar" : "Star"} shortcut={hint("star")} detail="Starred PRs stay at the top of the list">
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
