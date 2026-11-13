import { fireEvent, render, screen, within } from "@testing-library/react";
import { useState } from "react";
import { expect, test } from "vitest";
import { SidebarViews } from "./SidebarViews";
import { defaultCatalog, type SidebarView } from "./viewCatalog";
import { MINE_QUERY } from "./viewRule";

function Harness({ initial = defaultCatalog() }: { initial?: SidebarView[] }) {
  const [views, setViews] = useState(initial);
  const [current, setCurrent] = useState("mine");
  return <SidebarViews views={views} current={current} counts={{ bumps_due: 2 }} onSelect={setCurrent} onChange={setViews} />;
}

function openMenu(name: string) {
  const row = screen.getByRole("button", { name: (value) => value === name });
  const menu = row.parentElement?.querySelector('button[aria-label="View menu"]');
  if (!(menu instanceof HTMLButtonElement)) {
    throw new Error(`no menu for ${name}`);
  }
  fireEvent.click(menu);
}

test("a new view is created from the dialog and selected", () => {
  render(<Harness />);
  fireEvent.click(screen.getByRole("button", { name: "New view" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Name" }), { target: { value: "Needs me" } });
  fireEvent.change(screen.getByRole("textbox", { name: "Query" }), { target: { value: "is:open review-requested:@me" } });
  fireEvent.click(screen.getByRole("button", { name: "Create view" }));
  const nav = screen.getByRole("navigation", { name: "Views" });
  const created = within(nav).getByRole("button", { name: "Needs me" });
  expect(created).toHaveAttribute("aria-current", "page");
  expect(within(nav).getAllByRole("button", { name: /Needs me|My PRs|To review|Bumps due/ }).length).toBeGreaterThan(0);
});

test("the menu renames, edits, duplicates, and confirms a delete", () => {
  render(
    <Harness
      initial={[
        ...defaultCatalog(),
        { key: "custom:1", name: "Needs me", query: "is:open review-requested:@me", builtin: false, sort_order: 3 },
      ]}
    />,
  );
  openMenu("Needs me");
  fireEvent.click(screen.getByRole("menuitem", { name: "Rename" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Name" }), { target: { value: "Waiting" } });
  fireEvent.click(screen.getByRole("button", { name: "Rename" }));
  expect(screen.getByRole("button", { name: "Waiting" })).toBeInTheDocument();

  openMenu("Waiting");
  fireEvent.click(screen.getByRole("menuitem", { name: "Edit filters" }));
  expect(screen.getByRole("dialog", { name: "Edit filters" })).toBeInTheDocument();
  fireEvent.change(screen.getByRole("textbox", { name: "Query" }), { target: { value: "is:open is:draft" } });
  fireEvent.click(screen.getByRole("button", { name: "Save" }));

  openMenu("Waiting");
  fireEvent.click(screen.getByRole("menuitem", { name: "Duplicate" }));
  expect(screen.getByRole("button", { name: "Waiting copy" })).toBeInTheDocument();

  openMenu("Waiting");
  fireEvent.click(screen.getByRole("menuitem", { name: "Delete" }));
  expect(screen.getByRole("dialog", { name: "Delete view" })).toHaveTextContent("Delete “Waiting”?");
  fireEvent.click(screen.getByRole("button", { name: "Delete" }));
  expect(screen.queryByRole("button", { name: "Waiting" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Waiting copy" })).toBeInTheDocument();
});

test("a default view resets and cannot be deleted", () => {
  render(<Harness />);
  openMenu("My PRs");
  expect(screen.queryByRole("menuitem", { name: "Delete" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("menuitem", { name: "Rename" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Name" }), { target: { value: "Authored" } });
  fireEvent.click(screen.getByRole("button", { name: "Rename" }));
  openMenu("Authored");
  fireEvent.click(screen.getByRole("menuitem", { name: "Reset to default" }));
  expect(screen.getByRole("button", { name: "My PRs" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Authored" })).not.toBeInTheDocument();
});

test("the view menu stays in the row and shows on hover, focus, or while open", () => {
  render(<Harness />);
  const selected = screen.getByRole("button", { name: (value) => value === "My PRs" });
  expect(selected).toHaveAttribute("aria-current", "page");
  const quiet = menuButton(selected);
  expect(quiet).toHaveClass("opacity-0");
  expect(quiet).toHaveClass("group-hover:opacity-100");
  expect(quiet).toHaveClass("focus-visible:opacity-100");
  expect(quiet).not.toHaveClass("opacity-100");

  fireEvent.click(quiet);
  expect(menuButton(selected)).toHaveClass("opacity-100");
  expect(menuButton(selected)).not.toHaveClass("opacity-0");
  fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
  expect(menuButton(selected)).toHaveClass("opacity-0");
});

test("counts sit in the menu slot, stay blank at zero, and cap at 99+", () => {
  render(
    <SidebarViews
      views={defaultCatalog()}
      current="mine"
      counts={{ mine: 4, to_review: 0, bumps_due: 120 }}
      onSelect={() => undefined}
      onChange={() => undefined}
    />,
  );
  const mine = screen.getByRole("group", { name: "My PRs, 4" });
  const mineCount = within(mine).getByText("4");
  const menu = within(mine).getByRole("button", { name: "View menu" });
  expect(mineCount.parentElement).toBe(menu.parentElement);
  expect(mineCount.parentElement).toHaveClass("absolute");
  expect(mineCount).toHaveClass("text-slate-400");
  expect(mineCount).toHaveClass("pointer-events-none");
  expect(mineCount).toHaveClass("group-hover:invisible");
  expect(mineCount).not.toHaveClass("invisible");
  expect(menu).toHaveClass("opacity-0");
  expect(menu).toHaveClass("pointer-events-none");
  expect(menu).toHaveClass("group-hover:opacity-100");
  expect(menu).toHaveClass("group-hover:pointer-events-auto");

  fireEvent.click(menu);
  expect(within(mine).getByText("4")).toHaveClass("invisible");
  const openMenu = within(mine).getByRole("button", { name: "View menu" });
  expect(openMenu).toHaveClass("opacity-100");
  expect(openMenu).toHaveClass("pointer-events-auto");

  expect(screen.getByRole("group", { name: "To review" }).querySelector(".tabular-nums")).toBeNull();
  const bumps = within(screen.getByRole("group", { name: "Bumps due, 120 due" })).getByText("99+");
  expect(bumps).toHaveClass("bg-slate-700");
  expect(bumps).toHaveClass("text-slate-200");
  expect(screen.getByRole("button", { name: "My PRs" })).toHaveAccessibleDescription("4");
});

function menuButton(row: HTMLElement): HTMLButtonElement {
  const menu = row.parentElement?.querySelector('button[aria-label="View menu"]');
  if (!(menu instanceof HTMLButtonElement)) {
    throw new Error("no menu");
  }
  return menu;
}

test("right-click and the keyboard open the same menu", () => {
  render(<Harness />);
  fireEvent.contextMenu(screen.getByRole("button", { name: "Bumps due" }));
  expect(screen.getByRole("menu", { name: `Actions for Bumps due` })).toBeInTheDocument();
  fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  const actions = screen.getByRole("button", { name: (value) => value === "To review" }).parentElement?.querySelector('button[aria-label="View menu"]');
  if (!(actions instanceof HTMLButtonElement)) {
    throw new Error("no menu");
  }
  fireEvent.keyDown(actions, { key: "ArrowDown" });
  const menu = screen.getByRole("menu");
  expect(within(menu).getByRole("menuitem", { name: "Edit filters" })).toBeInTheDocument();
  fireEvent.keyDown(menu, { key: "ArrowDown" });
  expect(within(menu).getByRole("menuitem", { name: "Edit filters" })).toHaveFocus();
});

test("reset restores the default query", () => {
  const edited = defaultCatalog().map((view) => (view.key === "mine" ? { ...view, name: "Authored", query: "is:closed author:@me" } : view));
  render(<Harness initial={edited} />);
  openMenu("Authored");
  fireEvent.click(screen.getByRole("menuitem", { name: "Edit filters" }));
  expect(screen.getByRole("textbox", { name: "Query" })).toHaveValue("is:closed author:@me");
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  openMenu("Authored");
  fireEvent.click(screen.getByRole("menuitem", { name: "Reset to default" }));
  openMenu("My PRs");
  fireEvent.click(screen.getByRole("menuitem", { name: "Edit filters" }));
  expect(screen.getByRole("textbox", { name: "Query" })).toHaveValue(MINE_QUERY);
});
