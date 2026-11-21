import { fireEvent, render, screen, within } from "@testing-library/react";
import { useState } from "react";
import { expect, test } from "vitest";
import { SidebarViews } from "./SidebarViews";
import { defaultCatalog, type SidebarView } from "./viewCatalog";
import { MINE_QUERY, REVIEW_QUERY } from "./viewRule";

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

test("a new view is created from the header button and selected", () => {
  render(<Harness />);
  const heading = screen.getByRole("heading", { name: "Views" });
  expect(heading).toHaveClass("uppercase", "tracking-wide", "text-slate-500");
  expect(heading.querySelector("svg")).toHaveAttribute("width", "14");
  const create = screen.getByRole("button", { name: "New view" });
  expect(create.textContent?.trim()).toBe("");
  expect(create.tagName).toBe("BUTTON");
  expect(within(screen.getByRole("navigation", { name: "Views" })).queryByRole("button", { name: "New view" })).toBeNull();
  fireEvent.click(create);
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

const tree: SidebarView[] = [
  ...defaultCatalog(),
  { key: "custom:1", name: "Drafts", query: "is:draft", builtin: false, sort_order: 3, parent: "mine" },
  { key: "custom:2", name: "Web platform", query: "team-review-requested:acme/web-platform", builtin: false, sort_order: 4, parent: "to_review" },
  { key: "custom:3", name: "Payments", query: "team-review-requested:acme/payments", builtin: false, sort_order: 5, parent: "to_review" },
];

function viewNames(): string[] {
  return within(screen.getByRole("navigation", { name: "Views" }))
    .getAllByRole("button")
    .map((button) => button.textContent?.trim() ?? "")
    .filter((name) => name !== "");
}

test("children sit under their parent, and the parent chevron hides them", () => {
  function Tree() {
    const [parents, setParents] = useState<string[]>([]);
    return (
      <SidebarViews
        views={tree}
        current="to_review"
        counts={{}}
        collapsedParents={parents}
        onToggleParent={(key) => setParents((current) => (current.includes(key) ? current.filter((item) => item !== key) : [...current, key]))}
        onSelect={() => undefined}
        onChange={() => undefined}
      />
    );
  }
  render(<Tree />);
  expect(viewNames()).toEqual(["My PRs", "Drafts", "To review", "Web platform", "Payments", "Bumps due"]);
  expect(document.querySelectorAll("[data-connector]")).toHaveLength(2);
  const fold = screen.getByRole("button", { name: "Collapse To review" });
  expect(fold).toHaveAttribute("aria-expanded", "true");
  fireEvent.click(fold);
  expect(screen.getByRole("button", { name: "Expand To review" })).toHaveAttribute("aria-expanded", "false");
  expect(viewNames()).toEqual(["My PRs", "Drafts", "To review", "Bumps due"]);
  expect(screen.getByRole("button", { name: "To review" })).toBeInTheDocument();
});

test("the section header toggles without the new-view button, and the choice is kept", () => {
  function Sections() {
    const [collapsed, setCollapsed] = useState(false);
    const stored = JSON.stringify({ views: collapsed, projects: false, parents: [] });
    return (
      <>
        <SidebarViews
          views={defaultCatalog()}
          current="mine"
          counts={{ mine: 2, to_review: 1, bumps_due: 0 }}
          collapsed={collapsed}
          onToggleCollapsed={() => setCollapsed((current) => !current)}
          onSelect={() => undefined}
          onChange={() => undefined}
        />
        <button type="button" onClick={() => setCollapsed(JSON.parse(stored).views)}>
          Reload
        </button>
      </>
    );
  }
  render(<Sections />);
  const header = screen.getByRole("button", { name: "Views" });
  expect(header).toHaveAttribute("aria-expanded", "true");
  fireEvent.click(screen.getByRole("button", { name: "New view" }));
  expect(header).toHaveAttribute("aria-expanded", "true");
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  fireEvent.keyDown(header, { key: "Enter" });
  expect(header).toHaveAttribute("aria-expanded", "false");
  expect(screen.queryByRole("navigation", { name: "Views" })).not.toBeInTheDocument();
  expect(header).toHaveTextContent("3");
  fireEvent.click(screen.getByRole("button", { name: "Reload" }));
  expect(screen.getByRole("button", { name: "Views" })).toHaveAttribute("aria-expanded", "false");
  fireEvent.keyDown(screen.getByRole("button", { name: "Views" }), { key: " " });
  expect(screen.getByRole("navigation", { name: "Views" })).toBeInTheDocument();
});

test("new view is prefilled from the open parent, and edit shows the parent", () => {
  render(
    <SidebarViews
      views={tree}
      current="to_review"
      counts={{}}
      createDraft={{ parent: "to_review", query: "is:draft" }}
      onSelect={() => undefined}
      onChange={() => undefined}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "New view" }));
  expect(screen.getByRole("combobox", { name: "Parent" })).toHaveValue("to_review");
  expect(screen.getByRole("textbox", { name: "Query" })).toHaveValue("is:draft");
  expect(screen.getByText(`Shows ${REVIEW_QUERY} is:draft`)).toBeInTheDocument();
  fireEvent.change(screen.getByRole("combobox", { name: "Parent" }), { target: { value: "mine" } });
  expect(screen.getByText(`Shows ${MINE_QUERY} is:draft`)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

  openMenu("Web platform");
  fireEvent.click(screen.getByRole("menuitem", { name: "Edit filters" }));
  expect(screen.getByRole("combobox", { name: "Parent" })).toHaveValue("to_review");
  expect(screen.getByText(`Shows ${REVIEW_QUERY} team-review-requested:acme/web-platform`)).toBeInTheDocument();
});

function dragTransfer() {
  const store: Record<string, string> = {};
  return {
    setData(type: string, value: string) {
      store[type] = value;
    },
    getData(type: string) {
      return store[type] ?? "";
    },
    setDragImage() {
      return undefined;
    },
    effectAllowed: "move",
    dropEffect: "move",
    types: ["application/x-pr-nexus-view"],
  };
}

test("defaults are not draggable and Bumps due is not a drop target", () => {
  render(<Harness initial={tree} />);
  expect(screen.getByRole("group", { name: "My PRs" })).toHaveAttribute("draggable", "false");
  expect(screen.getByRole("group", { name: "To review" })).toHaveAttribute("draggable", "false");
  expect(screen.getByRole("group", { name: "Bumps due, 2 due" })).toHaveAttribute("draggable", "false");
  expect(screen.getByRole("group", { name: "Drafts" })).toHaveAttribute("draggable", "true");
  openMenu("My PRs");
  expect(screen.queryByRole("menuitem", { name: "Move to" })).not.toBeInTheDocument();
});

test("dragging a child onto the other parent highlights it and confirms only when the rule changes", async () => {
  render(<Harness initial={tree} />);
  const transfer = dragTransfer();
  fireEvent.dragStart(screen.getByRole("group", { name: "Web platform" }), { dataTransfer: transfer, clientX: 30, clientY: 90 });
  const target = screen.getByRole("group", { name: "My PRs" });
  fireEvent.dragOver(target, { dataTransfer: transfer, clientX: 40, clientY: 48 });
  expect(target).toHaveAttribute("data-drop-target", "active");
  expect(screen.getByTestId("view-drag-ghost")).toHaveTextContent("Web platform");
  fireEvent.dragOver(screen.getByRole("group", { name: "Bumps due, 2 due" }), { dataTransfer: transfer, clientX: 40, clientY: 200 });
  expect(screen.getByRole("group", { name: "Bumps due, 2 due" })).not.toHaveAttribute("data-drop-target");
  fireEvent.drop(target, { dataTransfer: transfer, clientX: 40, clientY: 48 });
  const dialog = await screen.findByRole("dialog", { name: "Move view" });
  expect(dialog).toHaveTextContent("Under My PRs it will also need author:@me. Move anyway?");
  expect(dialog).not.toHaveTextContent("→");
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(screen.queryByRole("dialog", { name: "Move view" })).not.toBeInTheDocument();
  expect(viewNames()).toEqual(["My PRs", "Drafts", "To review", "Web platform", "Payments", "Bumps due"]);

  fireEvent.dragStart(screen.getByRole("group", { name: "Web platform" }), { dataTransfer: transfer, clientX: 30, clientY: 90 });
  fireEvent.drop(screen.getByRole("group", { name: "My PRs" }), { dataTransfer: transfer, clientX: 40, clientY: 48 });
  fireEvent.click(within(await screen.findByRole("dialog", { name: "Move view" })).getByRole("button", { name: "Move" }));
  expect(viewNames()).toEqual(["My PRs", "Drafts", "Web platform", "To review", "Payments", "Bumps due"]);
  openMenu("Web platform");
  fireEvent.click(screen.getByRole("menuitem", { name: "Edit filters" }));
  expect(screen.getByRole("combobox", { name: "Parent" })).toHaveValue("mine");
  expect(screen.getByRole("textbox", { name: "Query" })).toHaveValue("review-requested:@me team-review-requested:acme/web-platform");
});

test("a drag that keeps the same pull requests moves without asking", () => {
  const initial = [
    ...tree,
    { key: "custom:4", name: "Bugs", query: `${REVIEW_QUERY} label:bug`, builtin: false, sort_order: 6 },
  ];
  render(<Harness initial={initial} />);
  const transfer = dragTransfer();
  const bugs = screen.getByRole("group", { name: "Bugs" });
  fireEvent.dragStart(bugs, { dataTransfer: transfer, clientX: 20, clientY: 220 });
  const target = screen.getByRole("group", { name: "To review" });
  fireEvent.dragOver(target, { dataTransfer: transfer, clientX: 40, clientY: 120 });
  fireEvent.drop(target, { dataTransfer: transfer, clientX: 40, clientY: 120 });
  expect(screen.queryByRole("dialog", { name: "Move view" })).not.toBeInTheDocument();
  expect(viewNames()).toEqual(["My PRs", "Drafts", "To review", "Web platform", "Payments", "Bugs", "Bumps due"]);
  openMenu("Bugs");
  fireEvent.click(screen.getByRole("menuitem", { name: "Edit filters" }));
  expect(screen.getByRole("combobox", { name: "Parent" })).toHaveValue("to_review");
  expect(screen.getByRole("textbox", { name: "Query" })).toHaveValue("label:bug");
});

test("Move to uses the same confirm, and counts appear when a local count is returned", async () => {
  render(
    <SidebarViews
      views={tree}
      current="mine"
      counts={{}}
      onSelect={() => undefined}
      onChange={() => undefined}
      countMove={() => Promise.resolve({ before: 4, after: 1 })}
    />,
  );
  openMenu("Drafts");
  fireEvent.click(screen.getByRole("menuitem", { name: "Move to" }));
  const submenu = screen.getByRole("menu", { name: "Move to" });
  expect(within(submenu).getByRole("menuitem", { name: "My PRs" })).toBeDisabled();
  fireEvent.click(within(submenu).getByRole("menuitem", { name: "To review" }));
  const dialog = await screen.findByRole("dialog", { name: "Move view" });
  expect(dialog).toHaveTextContent("Under To review it will also need review-requested:@me (4 → 1 PRs). Move anyway?");
});

test("Move to top level stores the full rule and the keyboard opens the submenu", () => {
  function Tree() {
    const [views, setViews] = useState(tree);
    return <SidebarViews views={views} current="mine" counts={{}} onSelect={() => undefined} onChange={setViews} />;
  }
  render(<Tree />);
  openMenu("Drafts");
  const menu = screen.getByRole("menu", { name: "Actions for Drafts" });
  fireEvent.keyDown(menu, { key: "ArrowDown" });
  fireEvent.keyDown(menu, { key: "ArrowDown" });
  fireEvent.keyDown(menu, { key: "ArrowDown" });
  expect(within(menu).getByRole("menuitem", { name: "Move to" })).toHaveFocus();
  fireEvent.keyDown(menu, { key: "ArrowRight" });
  const submenu = screen.getByRole("menu", { name: "Move to" });
  expect(within(submenu).getByRole("menuitem", { name: "To review" })).toHaveFocus();
  fireEvent.click(within(submenu).getByRole("menuitem", { name: "Top level" }));
  expect(screen.queryByRole("dialog", { name: "Move view" })).not.toBeInTheDocument();
  expect(viewNames()).toEqual(["My PRs", "To review", "Web platform", "Payments", "Bumps due", "Drafts"]);
  openMenu("Drafts");
  fireEvent.click(screen.getByRole("menuitem", { name: "Edit filters" }));
  expect(screen.getByRole("combobox", { name: "Parent" })).toHaveValue("");
  expect(screen.getByRole("textbox", { name: "Query" })).toHaveValue(`${MINE_QUERY} is:draft`);
});

test("dropping a child on the top-level zone clears its parent", () => {
  function Tree() {
    const [views, setViews] = useState(tree);
    return <SidebarViews views={views} current="mine" counts={{}} onSelect={() => undefined} onChange={setViews} />;
  }
  render(<Tree />);
  const transfer = dragTransfer();
  fireEvent.dragStart(screen.getByRole("group", { name: "Drafts" }), { dataTransfer: transfer });
  const zone = document.querySelector('[data-drop-zone="top"]');
  if (!(zone instanceof HTMLElement)) {
    throw new Error("no top-level drop zone");
  }
  fireEvent.dragOver(zone, { dataTransfer: transfer });
  expect(zone).toHaveAttribute("data-drop-target", "active");
  fireEvent.drop(zone, { dataTransfer: transfer });
  expect(screen.queryByRole("dialog", { name: "Move view" })).not.toBeInTheDocument();
  expect(viewNames()).toEqual(["My PRs", "To review", "Web platform", "Payments", "Bumps due", "Drafts"]);
});

test("dropping a sibling reorders that level", () => {
  function Tree() {
    const [views, setViews] = useState(tree);
    return <SidebarViews views={views} current="to_review" counts={{}} onSelect={() => undefined} onChange={setViews} />;
  }
  render(<Tree />);
  const transfer = dragTransfer();
  fireEvent.dragStart(screen.getByRole("group", { name: "Payments" }), { dataTransfer: transfer });
  const before = screen.getByRole("group", { name: "Web platform" });
  fireEvent.dragOver(before, { dataTransfer: transfer });
  fireEvent.drop(before, { dataTransfer: transfer });
  expect(screen.queryByRole("dialog", { name: "Move view" })).not.toBeInTheDocument();
  expect(viewNames()).toEqual(["My PRs", "Drafts", "To review", "Payments", "Web platform", "Bumps due"]);
});
