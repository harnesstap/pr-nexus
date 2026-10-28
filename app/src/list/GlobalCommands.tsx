import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Api, ListRow, ProjectDto } from "../api";
import { isTypingTarget } from "./keys";
import { defaultView } from "./model";
import { dayBounds, PAGE_SIZE, type PageRequest } from "./paging";
import type { ListActions } from "./PrList";
import { CommandPalette, type PaletteCommand } from "./CommandPalette";
import { ShortcutSheet } from "./ShortcutSheet";
import { hint, matchShortcut } from "./shortcuts";

/** Nav views the palette searches for PRs, in sidebar order. */
const SEARCH_VIEWS = ["mine", "to_review"] as const;

export function searchRequest(viewKey: string, query: string, now: Date): PageRequest {
  const view = defaultView();
  return {
    view_key: viewKey,
    grouping: view.grouping,
    ordering: view.ordering,
    direction: view.direction,
    filters: view.filters,
    query,
    collapsed: [],
    cursor: null,
    limit: PAGE_SIZE,
    now: now.toISOString(),
    ...dayBounds(now),
  };
}

interface Props {
  api: Pick<Api, "listPage">;
  projects: readonly ProjectDto[];
  actions: { current: ListActions | null };
  /** True while the list (not Settings) is on screen, so row actions apply. */
  listShown: boolean;
  onView: (viewKey: string) => void;
  onDetails: (row: ListRow) => void;
  onSync: (full: boolean) => void;
  syncing: boolean;
}

/** The `?` cheat sheet and the ⌘K palette, over the list screen. */
export function GlobalCommands({ api, projects, actions, listShown, onView, onDetails, onSync, syncing }: Props) {
  const [open, setOpen] = useState<"palette" | "help" | null>(null);
  const found = useRef(new Map<number, string>());

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      const id = matchShortcut(event);
      if (id === "palette") {
        event.preventDefault();
        setOpen((current) => (current === "palette" ? null : "palette"));
        return;
      }
      if (id === "help" && open === null && !isTypingTarget(event.target instanceof HTMLElement ? event.target : null)) {
        if (document.querySelector('[role="dialog"][aria-modal="true"]')) {
          return;
        }
        event.preventDefault();
        setOpen("help");
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  const searchPrs = useCallback(
    async (query: string) => {
      const now = new Date();
      const pages = await Promise.all(SEARCH_VIEWS.map((view) => api.listPage(searchRequest(view, query, now)).then((page) => ({ view, page }))));
      const rows: ListRow[] = [];
      const seen = new Set<number>();
      for (const { view, page } of pages) {
        for (const entry of page.entries) {
          if (entry.row && !seen.has(entry.row.id)) {
            seen.add(entry.row.id);
            found.current.set(entry.row.id, view);
            rows.push(entry.row);
          }
        }
      }
      return rows;
    },
    [api],
  );

  const commands = useMemo<PaletteCommand[]>(() => {
    const list = actions.current;
    const target = listShown && list !== null && (list.selectedCount > 0 || list.focused() !== null);
    const focusedUrl = listShown ? list?.focused()?.url : undefined;
    const run = (id: "request" | "github" | "slack") => () => actions.current?.run(id);
    return [
      { id: "request", label: "Request review / bump on Slack", hint: hint("request"), keywords: "bump slack reviewers", disabled: !target, run: run("request") },
      { id: "github", label: "Open in GitHub", hint: hint("github"), keywords: "browser pull request", disabled: !focusedUrl, run: run("github") },
      { id: "mine", label: "Go to My PRs", keywords: "view mine", run: () => onView("mine") },
      { id: "to_review", label: "Go to To review", keywords: "view review queue", run: () => onView("to_review") },
      ...projects
        .filter((project) => !project.archived)
        .map((project) => ({ id: `project:${project.id}`, label: `Go to project ${project.name}`, keywords: project.name, run: () => onView(`project:${project.id}`) })),
      { id: "sync", label: "Sync now", keywords: "refresh github", disabled: syncing, run: () => onSync(false) },
      { id: "resync", label: "Full resync", keywords: "refresh reload github", disabled: syncing, run: () => onSync(true) },
      { id: "settings", label: "Open settings", keywords: "preferences accounts", run: () => onView("settings") },
      { id: "help", label: "Keyboard shortcuts", hint: "?", keywords: "help cheat sheet keys", run: () => setOpen("help") },
    ];
    // `actions` is a ref filled by the list; recompute whenever the palette opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, projects, listShown, syncing, onView, onSync]);

  if (open === "help") {
    return <ShortcutSheet onClose={() => setOpen(null)} />;
  }
  if (open === "palette") {
    return (
      <CommandPalette
        commands={commands}
        searchPrs={searchPrs}
        onClose={() => setOpen((current) => (current === "palette" ? null : current))}
        onPickPr={(row) => {
          const view = found.current.get(row.id);
          if (view) {
            onView(view);
          }
          onDetails(row);
        }}
      />
    );
  }
  return null;
}
