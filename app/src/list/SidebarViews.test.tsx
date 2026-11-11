import { fireEvent, render, screen, within } from "@testing-library/react";
import { useState } from "react";
import { expect, test } from "vitest";
import { SidebarViews } from "./SidebarViews";
import { defaultCatalog, type SidebarView } from "./viewCatalog";
import { MINE_QUERY } from "./viewRule";

function Harness({ initial = defaultCatalog() }: { initial?: SidebarView[] }) {
  const [views, setViews] = useState(initial);
  const [current, setCurrent] = useState("mine");
  return <SidebarViews views={views} current={current} dueCount={2} onSelect={setCurrent} onChange={setViews} />;
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
