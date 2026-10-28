import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import type { ListRow } from "../api";
import { plainText } from "../textInput";

export interface PaletteCommand {
  id: string;
  label: string;
  /** Shown right-aligned, e.g. the shortcut "r". */
  hint?: string;
  /** Extra words that match, e.g. "slack" for Request review. */
  keywords?: string;
  disabled?: boolean;
  run: () => void;
}

export type PaletteItem = { kind: "command"; command: PaletteCommand } | { kind: "pr"; row: ListRow };

/**
 * Subsequence fuzzy score: every query character must appear in order. Higher is
 * better; consecutive runs and word starts score more. Null means no match.
 */
export function fuzzyScore(query: string, text: string): number | null {
  const q = query.trim().toLowerCase();
  if (q === "") {
    return 0;
  }
  const t = text.toLowerCase();
  let score = 0;
  let at = -1;
  let run = 0;
  for (const char of q) {
    if (char === " ") {
      run = 0;
      continue;
    }
    const next = t.indexOf(char, at + 1);
    if (next === -1) {
      return null;
    }
    run = next === at + 1 ? run + 1 : 1;
    const wordStart = next === 0 || /[\s/#:._-]/.test(t[next - 1]);
    score += run * 2 + (wordStart ? 3 : 0) - Math.min(next - at - 1, 5) * 0.1;
    at = next;
  }
  return t.startsWith(q) ? score + 10 : score;
}

/** Commands matching the query, best first; with no query, all in their given order. */
export function filterCommands(commands: readonly PaletteCommand[], query: string): PaletteCommand[] {
  if (query.trim() === "") {
    return commands.filter((command) => !command.disabled);
  }
  return commands
    .filter((command) => !command.disabled)
    .map((command, index) => ({ command, index, score: Math.max(fuzzyScore(query, command.label) ?? -1, (fuzzyScore(query, command.keywords ?? "") ?? -1) - 1) }))
    .filter((item) => item.score >= 0)
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .map((item) => item.command);
}

export function prText(row: ListRow): string {
  return `#${row.number} ${row.title} ${row.repo}`;
}

/** PRs whose number, title or repo fuzzy-match, best first, at most `limit`. */
export function filterPrs(rows: readonly ListRow[], query: string, limit = 8): ListRow[] {
  const q = query.trim().replace(/^#/, "");
  if (q === "") {
    return [];
  }
  return rows
    .map((row, index) => {
      const exact = String(row.number) === q ? 100 : String(row.number).startsWith(q) ? 20 : 0;
      const score = Math.max(fuzzyScore(q, row.title) ?? -1, fuzzyScore(q, row.repo) ?? -1);
      return { row, index, score: score < 0 && exact === 0 ? -1 : Math.max(score, 0) + exact };
    })
    .filter((item) => item.score >= 0)
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .slice(0, limit)
    .map((item) => item.row);
}

interface Props {
  commands: readonly PaletteCommand[];
  /** Loads PRs for the query (every nav view). */
  searchPrs: (query: string) => Promise<ListRow[]>;
  onPickPr: (row: ListRow) => void;
  onClose: () => void;
}

/** ⌘K / Ctrl+K: a combobox over commands and PRs. */
export function CommandPalette({ commands, searchPrs, onPickPr, onClose }: Props) {
  const id = useId();
  const input = useRef<HTMLInputElement>(null);
  const dialog = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState("");
  const [found, setFound] = useState<{ query: string; rows: ListRow[] }>({ query: "", rows: [] });
  const [active, setActive] = useState(0);

  useEffect(() => {
    input.current?.focus();
  }, []);

  useEffect(() => {
    let live = true;
    if (query.trim() === "") {
      return;
    }
    void searchPrs(query).then(
      (rows) => live && setFound({ query, rows: filterPrs(rows, query) }),
      () => live && setFound({ query, rows: [] }),
    );
    return () => {
      live = false;
    };
  }, [query, searchPrs]);

  const items: PaletteItem[] = useMemo(
    () => {
      const prs = query.trim() !== "" && found.query === query ? found.rows : [];
      return [...filterCommands(commands, query).map((command) => ({ kind: "command" as const, command })), ...prs.map((row) => ({ kind: "pr" as const, row }))];
    },
    [commands, query, found],
  );
  const current = Math.min(active, items.length - 1);

  function choose(item: PaletteItem | undefined) {
    if (!item) {
      return;
    }
    onClose();
    if (item.kind === "command") {
      item.command.run();
    } else {
      onPickPr(item.row);
    }
  }

  function onKeyDown(event: ReactKeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      onClose();
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (items.length > 0) {
        setActive((current + (event.key === "ArrowDown" ? 1 : items.length - 1)) % items.length);
      }
    } else if (event.key === "Enter") {
      event.preventDefault();
      choose(items[current]);
    } else if (event.key === "Tab") {
      // Focus trap: the input is the only tab stop; options are reached with the arrows.
      event.preventDefault();
      input.current?.focus();
    }
  }

  const optionId = (index: number) => `${id}-option-${index}`;
  const firstPr = items.findIndex((item) => item.kind === "pr");

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/50 p-8 pt-24" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <div
        ref={dialog}
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        className="w-full max-w-xl overflow-hidden rounded-lg border border-slate-700 bg-slate-900 text-sm text-slate-200 shadow-xl"
        onKeyDown={onKeyDown}
      >
        <input
          ref={input}
          {...plainText}
          role="combobox"
          aria-expanded="true"
          aria-controls={`${id}-list`}
          aria-autocomplete="list"
          aria-activedescendant={current >= 0 ? optionId(current) : undefined}
          aria-label="Search commands and PRs"
          placeholder="Type a command or search PRs…"
          className="w-full border-b border-slate-700 bg-transparent px-4 py-3 outline-none placeholder:text-slate-500"
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            setActive(0);
          }}
        />
        <ul id={`${id}-list`} role="listbox" aria-label="Results" className="max-h-96 overflow-auto py-1">
          {items.length === 0 && <li className="px-4 py-3 text-slate-400">No matches</li>}
          {items.map((item, index) => (
            <li
              key={item.kind === "command" ? `c-${item.command.id}` : `p-${item.row.id}`}
              id={optionId(index)}
              role="option"
              aria-selected={index === current}
              className={`flex cursor-pointer items-center justify-between gap-3 px-4 py-2 ${index === current ? "bg-slate-800 text-white" : ""} ${index === firstPr && index > 0 ? "border-t border-slate-800" : ""}`}
              onMouseMove={() => setActive(index)}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => choose(item)}
            >
              {item.kind === "command" ? (
                <>
                  <span>{item.command.label}</span>
                  {item.command.hint && <kbd className="rounded border border-slate-600 px-1.5 font-mono text-xs text-slate-300">{item.command.hint}</kbd>}
                </>
              ) : (
                <>
                  <span className="truncate">
                    <span className="text-slate-400">#{item.row.number}</span> {item.row.title}
                  </span>
                  <span className="shrink-0 text-xs text-slate-400">{item.row.repo}</span>
                </>
              )}
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
