import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import type { ProjectDto } from "../api";
import { PlusIcon } from "../icons";
import { plainText } from "../textInput";
import { renameError } from "./model";

interface Props {
  /** Every project; archived ones are left out of the menu. */
  projects: ProjectDto[];
  /** How many PRs the action covers, for the labels. */
  count: number;
  onPick: (projectId: number) => void;
  /** Creates the project with the PRs; rejects with the reason it could not. */
  onCreate: (name: string) => Promise<void>;
  onClose: () => void;
}

/**
 * "Add to project" (§8.2): the non-archived projects with their colors, then
 * "New project from selection…", which turns into an inline name field.
 */
export function AddToProjectMenu({ projects, count, onPick, onCreate, onClose }: Props) {
  const active = projects.filter((project) => !project.archived);
  const [draft, setDraft] = useState<{ name: string; error: string | null; busy: boolean } | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!draft) {
      menuRef.current?.querySelector<HTMLElement>('[role="menuitem"]')?.focus();
    }
  }, [draft]);

  useEffect(() => {
    function onDown(event: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(event.target as Node)) {
        onClose();
      }
    }
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [onClose]);

  function onMenuKey(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      if (draft) {
        setDraft(null);
      } else {
        onClose();
      }
      return;
    }
    if (draft || (event.key !== "ArrowDown" && event.key !== "ArrowUp")) {
      return;
    }
    event.preventDefault();
    const items = [...(menuRef.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [])];
    const index = items.indexOf(document.activeElement as HTMLElement);
    const next = event.key === "ArrowDown" ? index + 1 : index - 1;
    items[(next + items.length) % items.length]?.focus();
  }

  async function create() {
    if (!draft) {
      return;
    }
    const problem = renameError(draft.name, projects, null);
    if (problem) {
      setDraft({ ...draft, error: problem });
      return;
    }
    setDraft({ ...draft, busy: true, error: null });
    try {
      await onCreate(draft.name.trim());
    } catch (err) {
      setDraft({ ...draft, busy: false, error: err instanceof Error ? err.message : String(err) });
    }
  }

  const prs = `${count} ${count === 1 ? "PR" : "PRs"}`;
  return (
    <div
      ref={menuRef}
      role="menu"
      aria-label={`Add ${prs} to project`}
      className="absolute bottom-full left-0 z-40 mb-2 w-64 rounded-md border border-slate-700 bg-slate-900 py-1 text-sm shadow-xl"
      onKeyDown={onMenuKey}
    >
      {active.length > 0 && (
        <div className="max-h-64 overflow-auto">
          {active.map((project) => (
            <button
              key={project.id}
              type="button"
              role="menuitem"
              className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-slate-200 hover:bg-slate-800 focus:bg-slate-800 focus:outline-none"
              onClick={() => onPick(project.id)}
            >
              <span aria-hidden="true" className="h-2 w-2 shrink-0 rounded-sm" style={{ background: project.color }} />
              <span className="truncate">{project.name}</span>
            </button>
          ))}
        </div>
      )}
      {active.length > 0 && <div role="separator" className="my-1 border-t border-slate-800" />}
      {draft ? (
        <form
          className="px-3 py-1.5"
          onSubmit={(event) => {
            event.preventDefault();
            void create();
          }}
        >
          <input
            {...plainText}
            aria-label="New project name"
            autoFocus
            disabled={draft.busy}
            className="w-full rounded border border-slate-600 bg-slate-950 px-2 py-1 text-sm"
            placeholder="Project name"
            value={draft.name}
            onChange={(event) => setDraft({ ...draft, name: event.target.value, error: null })}
          />
          {draft.error ? (
            <p className="mt-1 text-xs text-rose-300">{draft.error}</p>
          ) : (
            <p className="mt-1 text-xs text-slate-500">Enter to create with {prs} · Esc to go back</p>
          )}
        </form>
      ) : (
        <button
          type="button"
          role="menuitem"
          className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-slate-200 hover:bg-slate-800 focus:bg-slate-800 focus:outline-none"
          onClick={() => setDraft({ name: "", error: null, busy: false })}
        >
          <span className="flex w-2 justify-center text-slate-400">
            <PlusIcon size={14} />
          </span>
          New project from selection…
        </button>
      )}
    </div>
  );
}

