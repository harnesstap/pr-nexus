import { act, fireEvent, render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { ListRow, ProjectDto } from "../api";
import { CommandPalette, filterCommands, filterPrs, fuzzyScore, type PaletteCommand } from "./CommandPalette";
import { GlobalCommands } from "./GlobalCommands";
import type { PageDto } from "./paging";

const pr = (id: number, title: string, repo = "acme/widgets"): ListRow => ({ id, number: id, title, repo, url: `https://github.com/${repo}/pull/${id}` }) as ListRow;

function commands(run = vi.fn()): PaletteCommand[] {
  return [
    { id: "request", label: "Request review / bump on Slack", keywords: "bump", run: () => run("request") },
    { id: "github", label: "Open in GitHub", run: () => run("github") },
    { id: "sync", label: "Sync now", run: () => run("sync") },
    { id: "resync", label: "Full resync", disabled: true, run: () => run("resync") },
    { id: "settings", label: "Open settings", run: () => run("settings") },
  ];
}

test("fuzzy matching is an in-order subsequence that prefers prefixes", () => {
  expect(fuzzyScore("osn", "Open settings")).not.toBeNull();
  expect(fuzzyScore("zz", "Open settings")).toBeNull();
  expect(fuzzyScore("open", "Open settings")!).toBeGreaterThan(fuzzyScore("open", "Reopen")!);
});

test("filterCommands ranks, uses keywords and hides disabled commands", () => {
  const list = commands();
  expect(filterCommands(list, "").map((item) => item.id)).toEqual(["request", "github", "sync", "settings"]);
  expect(filterCommands(list, "sync").map((item) => item.id)).toEqual(["sync"]);
  expect(filterCommands(list, "bump")[0].id).toBe("request");
  expect(filterCommands(list, "open")[0].id).toBe("github");
});

test("filterPrs matches number, title and repo", () => {
  const rows = [pr(479, "Add address autocomplete"), pr(12, "Fix login", "acme-labs/portal"), pr(4790, "Other")];
  expect(filterPrs(rows, "#479")[0].id).toBe(479);
  expect(filterPrs(rows, "login").map((row) => row.id)).toEqual([12]);
  expect(filterPrs(rows, "portal").map((row) => row.id)).toEqual([12]);
  expect(filterPrs(rows, "")).toEqual([]);
});

test("the palette is an accessible combobox; arrows and Enter run a command", () => {
  const run = vi.fn();
  const onClose = vi.fn();
  render(<CommandPalette commands={commands(run)} searchPrs={async () => []} onPickPr={() => undefined} onClose={onClose} />);
  const input = screen.getByRole("combobox", { name: "Search commands and PRs" });
  expect(input).toHaveFocus();
  expect(input).toHaveAttribute("autocorrect", "off");
  expect(input).toHaveAttribute("autocapitalize", "off");
  expect(input).toHaveAttribute("spellcheck", "false");
  expect(screen.getAllByRole("option")).toHaveLength(4);
  fireEvent.keyDown(input, { key: "ArrowDown" });
  expect(input.getAttribute("aria-activedescendant")).toBe(screen.getAllByRole("option")[1].id);
  fireEvent.keyDown(input, { key: "Enter" });
  expect(run).toHaveBeenCalledWith("github");
  expect(onClose).toHaveBeenCalled();
});

test("Esc closes and Tab stays in the palette", () => {
  const onClose = vi.fn();
  render(<CommandPalette commands={commands()} searchPrs={async () => []} onPickPr={() => undefined} onClose={onClose} />);
  const input = screen.getByRole("combobox");
  fireEvent.keyDown(input, { key: "Tab" });
  expect(input).toHaveFocus();
  fireEvent.keyDown(input, { key: "Escape" });
  expect(onClose).toHaveBeenCalled();
});

test("typing searches PRs and picking one opens it", async () => {
  const onPickPr = vi.fn();
  render(<CommandPalette commands={commands()} searchPrs={async () => [pr(479, "Add address autocomplete")]} onPickPr={onPickPr} onClose={() => undefined} />);
  fireEvent.change(screen.getByRole("combobox"), { target: { value: "autocomplete" } });
  fireEvent.click(await screen.findByRole("option", { name: /#479 Add address autocomplete/ }));
  expect(onPickPr).toHaveBeenCalledWith(expect.objectContaining({ id: 479 }));
});

const page = (rows: ListRow[]): PageDto => ({ entries: rows.map((row) => ({ section: "s", row })), sections: [], next_cursor: null, summary: { open: 0, finished: 0, unassigned: 0, live_projects: 0 }, in_view: 0, orgs: [], quick: {} as PageDto["quick"], elsewhere: null });

test("Mod+K opens the palette; commands reuse the list's actions and navigation", async () => {
  const onView = vi.fn();
  const onSync = vi.fn();
  const onDetails = vi.fn();
  const run = vi.fn(() => true);
  const project: ProjectDto = { id: 3, name: "Checkout", color: "#000000", archived: false, description: "", sort_order: 0 };
  const listPage = vi.fn(async (request: { view_key: string }) => page(request.view_key === "to_review" ? [pr(9, "Review me")] : []));
  render(
    <GlobalCommands
      api={{ listPage }}
      projects={[project]}
      actions={{ current: { run, focused: () => pr(1, "Focused"), selectedCount: 0 } }}
      listShown
      onView={onView}
      onDetails={onDetails}
      onSync={onSync}
      syncing={false}
    />,
  );
  const open = (query: string) => {
    fireEvent.keyDown(window, { key: "k", metaKey: true });
    fireEvent.change(screen.getByRole("combobox"), { target: { value: query } });
  };
  open("request");
  fireEvent.keyDown(screen.getByRole("combobox"), { key: "Enter" });
  expect(run).toHaveBeenCalledWith("request");

  open("checkout");
  fireEvent.keyDown(screen.getByRole("combobox"), { key: "Enter" });
  expect(onView).toHaveBeenCalledWith("project:3");

  open("full resync");
  fireEvent.keyDown(screen.getByRole("combobox"), { key: "Enter" });
  expect(onSync).toHaveBeenCalledWith(true);

  open("review me");
  await act(async () => undefined);
  fireEvent.click(await screen.findByRole("option", { name: /#9 Review me/ }));
  expect(onView).toHaveBeenLastCalledWith("to_review");
  expect(onDetails).toHaveBeenCalledWith(expect.objectContaining({ id: 9 }));
});

test("? opens the cheat sheet outside text fields", () => {
  render(<GlobalCommands api={{ listPage: vi.fn() }} projects={[]} actions={{ current: null }} listShown onView={vi.fn()} onDetails={vi.fn()} onSync={vi.fn()} syncing={false} />);
  fireEvent.keyDown(window, { key: "?", shiftKey: true });
  expect(screen.getByRole("dialog", { name: "Keyboard shortcuts" })).toBeInTheDocument();
});
