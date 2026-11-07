import { useEffect, useId, useRef, useState } from "react";
import { CloseIcon } from "../icons";
import { plainText } from "../textInput";

export interface RepoOption {
  name: string;
  count: number;
}

/**
 * Compact multi-select for `owner/name`. An empty selection means every repository.
 * The popover overlays the toolbar; it does not add a row.
 */
export function RepoPicker({
  options,
  selected,
  onChange,
}: {
  options: readonly RepoOption[];
  selected: readonly string[];
  onChange: (repos: string[]) => void;
}) {
  const [open, setOpen] = useState(false);
  const [find, setFind] = useState("");
  const root = useRef<HTMLDivElement>(null);
  const findRef = useRef<HTMLInputElement>(null);
  const listId = useId();
  const chosen = selected.length;
  const aria =
    chosen === 0 ? "Repository" : chosen === 1 ? `Repository, ${selected[0]}` : `Repository, ${chosen} repositories`;
  const short = chosen === 0 ? "Repos" : chosen === 1 ? selected[0] : String(chosen);
  const wide = chosen === 0 ? "Repository" : chosen === 1 ? selected[0] : `${chosen} repositories`;
  const query = find.trim().toLowerCase();
  const shown = options.filter((option) => option.name.toLowerCase().includes(query));

  useEffect(() => {
    if (!open) {
      return;
    }
    findRef.current?.focus();
    function onPointer(event: MouseEvent) {
      if (root.current && event.target instanceof Node && !root.current.contains(event.target)) {
        setOpen(false);
        setFind("");
      }
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.stopPropagation();
        setOpen(false);
        setFind("");
      }
    }
    document.addEventListener("mousedown", onPointer);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onPointer);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  function toggle(name: string) {
    const key = name.toLowerCase();
    const next = selected.some((item) => item.toLowerCase() === key)
      ? selected.filter((item) => item.toLowerCase() !== key)
      : [...selected, name];
    onChange(next);
  }

  return (
    <div ref={root} className="relative shrink-0">
      <div className={`flex h-8 items-center rounded-md border text-xs ${chosen > 0 ? "border-violet-400 bg-violet-500/10 text-violet-100" : "border-slate-700 bg-slate-900 text-slate-300"}`}>
        <button
          type="button"
          aria-label={aria}
          aria-haspopup="dialog"
          aria-expanded={open}
          aria-controls={listId}
          className="flex h-8 max-w-24 items-center gap-1 px-2 hover:text-white @3xl:max-w-56"
          onClick={() => {
            setFind("");
            setOpen((current) => !current);
          }}
        >
          <span className="truncate @3xl:hidden">{short}</span>
          <span className="hidden truncate @3xl:inline">{wide}</span>
        </button>
        {chosen > 0 && (
          <button
            type="button"
            aria-label="Clear repositories"
            className="mr-1 flex h-6 w-6 items-center justify-center rounded text-violet-200 hover:bg-slate-800"
            onClick={() => onChange([])}
          >
            <CloseIcon />
          </button>
        )}
      </div>
      {open && (
        <div id={listId} role="dialog" aria-label="Repositories" className="absolute left-0 z-30 mt-1 w-72 rounded border border-slate-700 bg-slate-900 p-2 text-sm shadow-xl">
          <input
            {...plainText}
            ref={findRef}
            aria-label="Find a repository"
            className="mb-2 w-full rounded border border-slate-700 bg-slate-950 px-2 py-1 text-sm outline-none placeholder:text-slate-500 focus:border-violet-400"
            placeholder="Find a repository"
            value={find}
            onChange={(event) => setFind(event.target.value)}
          />
          <label className="flex items-center gap-2 rounded px-1 py-1 hover:bg-slate-800">
            <input
              type="checkbox"
              checked={chosen === 0}
              onChange={() => onChange([])}
            />
            All repositories
          </label>
          <ul className="max-h-64 overflow-auto">
            {shown.length === 0 && <li className="px-1 py-2 text-xs text-slate-500">No repositories match.</li>}
            {shown.map((option) => {
              const checked = selected.some((item) => item.toLowerCase() === option.name.toLowerCase());
              return (
                <li key={option.name.toLowerCase()}>
                  <label className={`flex items-center gap-2 rounded px-1 py-1 hover:bg-slate-800 ${option.count === 0 ? "text-slate-500" : ""}`}>
                    <input
                      type="checkbox"
                      aria-label={option.name}
                      checked={checked}
                      onChange={() => toggle(option.name)}
                    />
                    <span className="min-w-0 flex-1 truncate">{option.name}</span>
                    <span className="shrink-0 text-xs text-slate-500">{option.count}</span>
                  </label>
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </div>
  );
}
