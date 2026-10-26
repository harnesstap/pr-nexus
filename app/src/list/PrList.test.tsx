import { act, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, expect, test, vi } from "vitest";
import type { ListRow, ProjectDto } from "../api";
import { defaultView, type SavedView } from "./model";
import { PrList } from "./PrList";

afterEach(() => {
  vi.useRealTimers();
});

const auth: ProjectDto = { id: 1, name: "Auth", color: "#7c5cbf", archived: false, description: "", sort_order: 0 };
const billing: ProjectDto = { id: 2, name: "Billing", color: "#3f6f8f", archived: false, description: "", sort_order: 1 };
const now = new Date("2026-10-26T12:00:00Z");

function row(partial: Partial<ListRow> & Pick<ListRow, "id" | "title">): ListRow {
  return {
    number: partial.number ?? partial.id,
    url: `https://github.com/acme/widgets/pull/${partial.id}`,
    body: "",
    owner: "acme",
    repo: "acme/widgets",
    author_login: "ada",
    is_draft: false,
    state: "open",
    checks: "success",
    review_decision: null,
    unresolved_threads: 0,
    total_threads: 2,
    involvement: 1,
    updated_at: "2026-10-26T11:00:00Z",
    created_at: "2026-10-20T11:00:00Z",
    hidden: false,
    project_ids: [1],
    readiness: "ready",
    readiness_notes: [],
    slack: null,
    ...partial,
  };
}

function Harness({
  rows,
  projects = [auth, billing],
  onRename = () => Promise.resolve(),
}: {
  rows: ListRow[];
  projects?: ProjectDto[];
  onRename?: (id: number, name: string) => Promise<void>;
}) {
  const [view, setView] = useState<SavedView>(defaultView());
  return (
    <PrList
      rows={rows}
      projects={projects}
      viewKey="mine"
      view={view}
      onViewChange={setView}
      now={now}
      onOpen={() => undefined}
      onRename={onRename}
      onArchive={() => undefined}
      onDelete={() => undefined}
    />
  );
}

const rows = [
  row({ id: 1, number: 142, title: "Migrate login", project_ids: [1, 2] }),
  row({ id: 2, number: 138, title: "Add passkeys", project_ids: [1] }),
  row({ id: 3, number: 23, title: "Fix tax rounding", project_ids: [] }),
];

test("section and global tri-state checkboxes select only visible rows", () => {
  render(<Harness rows={rows} />);
  const section = screen.getByRole("checkbox", { name: "Select Auth" });
  expect(section).not.toBeChecked();
  fireEvent.click(section);
  for (const box of screen.getAllByRole("checkbox", { name: "Select Migrate login" })) {
    expect(box).toBeChecked();
  }
  expect(screen.getByRole("checkbox", { name: "Select Add passkeys" })).toBeChecked();
  expect(screen.getByRole("checkbox", { name: "Select Fix tax rounding" })).not.toBeChecked();
  fireEvent.click(screen.getByRole("checkbox", { name: "Select Add passkeys" }));
  expect(screen.getByRole("checkbox", { name: "Select Auth" })).toBePartiallyChecked();
  expect(screen.getByRole("checkbox", { name: "Select all" })).toBePartiallyChecked();
  fireEvent.click(screen.getByRole("checkbox", { name: "Select Auth" }));
  expect(screen.getByRole("checkbox", { name: "Select Add passkeys" })).toBeChecked();
});

test("a pull request in two projects is selected in both and counted once", () => {
  render(<Harness rows={rows} />);
  const copies = screen.getAllByRole("checkbox", { name: "Select Migrate login" });
  expect(copies).toHaveLength(2);
  fireEvent.click(copies[0]!);
  expect(copies[1]).toBeChecked();
  expect(screen.getByRole("checkbox", { name: "Select Billing" })).toBeChecked();
  expect(screen.getByText("1 selected")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Request / bump on Slack" })).toBeDisabled();
});

test("search excludes hidden rows from the parent toggle and keeps a hidden selection", async () => {
  vi.useFakeTimers();
  render(<Harness rows={rows} />);
  fireEvent.click(screen.getByRole("checkbox", { name: "Select all" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Search" }), { target: { value: "passkeys" } });
  await act(async () => {
    vi.advanceTimersByTime(150);
  });
  expect(screen.queryByText("Migrate login")).not.toBeInTheDocument();
  expect(screen.getByText("3 selected (2 hidden)")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("checkbox", { name: "Select Auth" }));
  expect(screen.getByRole("checkbox", { name: "Select Add passkeys" })).not.toBeChecked();
  fireEvent.change(screen.getByRole("textbox", { name: "Search" }), { target: { value: "" } });
  await act(async () => {
    vi.advanceTimersByTime(150);
  });
  expect(screen.getAllByRole("checkbox", { name: "Select Migrate login" })[0]).toBeChecked();
  expect(screen.getByRole("checkbox", { name: "Select Add passkeys" })).not.toBeChecked();
});

test("the funnel resets filters and leaves grouping in place", () => {
  render(<Harness rows={rows} />);
  fireEvent.click(screen.getByRole("button", { name: "Filter menu" }));
  fireEvent.click(screen.getByRole("button", { name: /Grouping/ }));
  fireEvent.click(screen.getByRole("button", { name: "Org" }));
  fireEvent.click(screen.getByRole("button", { name: /Status \/ draft/ }));
  fireEvent.click(screen.getByRole("checkbox", { name: "ready" }));
  expect(screen.getByLabelText("Filters active")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Reset" }));
  expect(screen.queryByLabelText("Filters active")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: /Grouping/ })).toHaveTextContent("Org");
});

test("inline rename saves, cancels, and rejects an empty or duplicate name", async () => {
  const onRename = vi.fn(() => Promise.resolve());
  render(<Harness rows={rows} onRename={onRename} />);
  fireEvent.doubleClick(screen.getByText("Unassigned"));
  expect(screen.queryByRole("textbox", { name: "Rename Unassigned" })).not.toBeInTheDocument();
  fireEvent.doubleClick(screen.getByText("Auth"));
  const input = screen.getByRole("textbox", { name: "Rename Auth" });
  fireEvent.change(input, { target: { value: "   " } });
  fireEvent.keyDown(input, { key: "Enter" });
  expect(screen.getByText(/empty/)).toBeInTheDocument();
  expect(onRename).not.toHaveBeenCalled();
  fireEvent.change(input, { target: { value: "Billing" } });
  fireEvent.keyDown(input, { key: "Enter" });
  expect(screen.getByText(/already exists/)).toBeInTheDocument();
  fireEvent.change(input, { target: { value: "Auth v2" } });
  fireEvent.keyDown(input, { key: "Enter" });
  expect(onRename).toHaveBeenCalledWith(1, "Auth v2");
});
