import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { useState } from "react";
import { afterEach, expect, test, vi } from "vitest";
import type { ListRow, ProjectDto } from "../api";
import { PAGE_SIZE, pageFromRows, type PageDto, type PageRequest, type SearchRow } from "./paging";
import { defaultView, type SavedView } from "./model";
import { PrList, SEARCH_DEBOUNCE_MS } from "./PrList";

afterEach(() => {
  vi.useRealTimers();
});

const auth: ProjectDto = { id: 1, name: "Auth", color: "#7c5cbf", archived: false, description: "", sort_order: 0 };
const billing: ProjectDto = { id: 2, name: "Billing", color: "#3f6f8f", archived: false, description: "", sort_order: 1 };
const now = new Date("2026-10-26T12:00:00Z");

function row(partial: Partial<SearchRow> & Pick<SearchRow, "id" | "title">): SearchRow {
  return {
    number: partial.number ?? partial.id,
    url: `https://github.com/acme/widgets/pull/${partial.id}`,
    body: "",
    owner: "acme",
    repo: "acme/widgets",
    author_login: "ada",
    head_ref: `feature/pr-${partial.id}`,
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
    conflicts: false,
    reviewers_requested: false,
    pending_reviewers: [],
    slack: null,
    ...partial,
  };
}

function Harness({
  rows,
  projects = [auth, billing],
  loadPage,
  version = 0,
  onRename = () => Promise.resolve(),
  viewKey = "mine",
  onLinkPrs = () => undefined,
  onCreateProjectWithPrs = () => Promise.resolve(),
  onEditProject = () => undefined,
  onDetails = () => undefined,
  detailId = null,
  onRequestReview = () => undefined,
  onBulkReview = () => undefined,
  onOpen = () => undefined,
}: {
  rows: SearchRow[];
  projects?: ProjectDto[];
  /** Defaults to the TypeScript reference of `list_page` over `rows`. */
  loadPage?: (request: PageRequest) => Promise<PageDto>;
  version?: number;
  onRename?: (id: number, name: string) => Promise<void>;
  viewKey?: string;
  onLinkPrs?: (projectId: number, ids: number[]) => void;
  onCreateProjectWithPrs?: (name: string, ids: number[]) => Promise<void>;
  onEditProject?: (id: number) => void;
  onDetails?: (row: ListRow) => void;
  detailId?: number | null;
  onRequestReview?: (row: ListRow) => void;
  onBulkReview?: (ids: number[]) => void;
  onOpen?: (url: string) => void;
}) {
  const [view, setView] = useState<SavedView>(defaultView());
  const [currentView, setCurrentView] = useState(viewKey);
  const [reference] = useState(() => (request: PageRequest) => Promise.resolve(pageFromRows(rows, projects, request)));
  return (
    <PrList
      loadPage={loadPage ?? reference}
      version={version}
      projects={projects}
      viewKey={currentView}
      view={view}
      onViewChange={setView}
      onShowView={setCurrentView}
      now={now}
      onOpen={onOpen}
      onRename={onRename}
      onArchive={() => undefined}
      onDelete={() => undefined}
      onEditProject={onEditProject}
      onLinkPrs={onLinkPrs}
      onCreateProjectWithPrs={onCreateProjectWithPrs}
      onDetails={onDetails}
      detailId={detailId}
      onRequestReview={onRequestReview}
      onBulkReview={onBulkReview}
    />
  );
}

/** Lets pending `list_page` promises resolve and React commit. */
async function settle() {
  await act(async () => {
    for (let index = 0; index < 5; index += 1) {
      await Promise.resolve();
    }
  });
}

async function show(ui: React.ReactElement) {
  const result = render(ui);
  await settle();
  return result;
}

const rows = [
  row({ id: 1, number: 142, title: "Migrate login", project_ids: [1, 2] }),
  row({ id: 2, number: 138, title: "Add passkeys", project_ids: [1] }),
  row({ id: 3, number: 23, title: "Fix tax rounding", project_ids: [] }),
];

test("section and global tri-state checkboxes select only visible rows", async () => {
  await show(<Harness rows={rows} />);
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

test("a pull request in two projects is selected in both and counted once", async () => {
  const requested: number[] = [];
  const bulk: number[][] = [];
  await show(<Harness rows={rows} onRequestReview={(row) => requested.push(row.id)} onBulkReview={(ids) => bulk.push(ids)} />);
  const copies = screen.getAllByRole("checkbox", { name: "Select Migrate login" });
  expect(copies).toHaveLength(2);
  fireEvent.click(copies[0]!);
  expect(copies[1]).toBeChecked();
  expect(screen.getByRole("checkbox", { name: "Select Billing" })).toBeChecked();
  expect(screen.getByText("1 selected")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Request / bump on Slack" }));
  expect(requested).toEqual([1]);
  fireEvent.click(screen.getByRole("checkbox", { name: "Select Add passkeys" }));
  fireEvent.click(screen.getByRole("button", { name: "Request / bump on Slack" }));
  expect(requested).toEqual([1]);
  expect(bulk.map((ids) => [...ids].sort())).toEqual([[1, 2]]);
});

test("search excludes hidden rows from the parent toggle and keeps a hidden selection", async () => {
  vi.useFakeTimers();
  await show(<Harness rows={rows} />);
  fireEvent.click(screen.getByRole("checkbox", { name: "Select all" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Search" }), { target: { value: "passkeys" } });
  await act(async () => {
    vi.advanceTimersByTime(200);
  });
  await settle();
  expect(screen.queryByText("Migrate login")).not.toBeInTheDocument();
  expect(screen.getByText("3 selected (2 hidden)")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("checkbox", { name: "Select Auth" }));
  expect(screen.getByRole("checkbox", { name: "Select Add passkeys" })).not.toBeChecked();
  fireEvent.change(screen.getByRole("textbox", { name: "Search" }), { target: { value: "" } });
  await act(async () => {
    vi.advanceTimersByTime(200);
  });
  await settle();
  expect(screen.getAllByRole("checkbox", { name: "Select Migrate login" })[0]).toBeChecked();
  expect(screen.getByRole("checkbox", { name: "Select Add passkeys" })).not.toBeChecked();
});

test("the funnel resets filters and leaves grouping in place", async () => {
  await show(<Harness rows={rows} />);
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
  await show(<Harness rows={rows} onRename={onRename} />);
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

test("the funnel button sits right of search and its menu closes on Escape and outside click", async () => {
  await show(<Harness rows={rows} />);
  const search = screen.getByRole("textbox", { name: "Search" });
  const funnel = screen.getByRole("button", { name: "Filter menu" });
  expect(search.compareDocumentPosition(funnel) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(funnel.querySelector("svg")).not.toBeNull();
  expect(funnel).not.toHaveTextContent("▾");
  fireEvent.click(funnel);
  expect(screen.getByRole("menu")).toBeInTheDocument();
  fireEvent.keyDown(document, { key: "Escape" });
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  fireEvent.click(funnel);
  fireEvent.mouseDown(screen.getByRole("menu"));
  expect(screen.getByRole("menu")).toBeInTheDocument();
  fireEvent.mouseDown(document.body);
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
});

test("rows show labelled threads, a review pill, and the Slack request status", async () => {
  const slack = {
    workspace: "acme-team",
    channel_id: "C1",
    channel_name: "reviews",
    permalink: "https://acme-team.slack.com/archives/C1/p1",
    root_ts: "1.0",
    source: "app",
    bump_count: 0,
    last_bumped_at: null,
    requested_at: "2026-10-24T12:00:00Z",
  };
  const onOpen = vi.fn();
  await show(
    <Harness
      onOpen={onOpen}
      rows={[
        row({ id: 1, title: "Requested one", review_decision: "approved", unresolved_threads: 1, total_threads: 3, slack }),
        row({
          id: 2,
          title: "Bumped one",
          review_decision: "changes_requested",
          slack: { ...slack, bump_count: 2, last_bumped_at: "2026-10-26T07:00:00Z" },
        }),
        row({ id: 3, title: "Quiet one", total_threads: 0 }),
        row({
          id: 4,
          title: "Labs one",
          slack: { ...slack, team_id: "T0ACME0002", workspace_name: "Acme Labs", workspace_disabled: true, requested_at: "2026-10-25T12:00:00Z" },
        }),
      ]}
    />,
  );
  expect(screen.getByText("Requested · 2d ago")).toBeInTheDocument();
  expect(screen.getByText("Bumped · 5h ago")).toBeInTheDocument();
  expect(screen.getByText("· Acme Labs")).toBeInTheDocument();
  expect(screen.getByText("· workspace disabled")).toBeInTheDocument();
  expect(screen.getAllByText("· workspace disabled")).toHaveLength(1);
  expect(screen.getByText("Not requested")).toBeInTheDocument();
  expect(screen.getByLabelText("1 unresolved of 3 review threads")).toBeInTheDocument();
  expect(screen.queryByLabelText("0 unresolved of 0 review threads")).not.toBeInTheDocument();
  expect(screen.queryByText("0/0")).not.toBeInTheDocument();
  expect(screen.getByText("Approved")).toBeInTheDocument();
  expect(screen.getByText("Changes requested")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Open Slack request for Requested one" }));
  expect(onOpen).toHaveBeenCalledWith(slack.permalink);
  expect(screen.queryByRole("button", { name: "Open Slack request for Quiet one" })).not.toBeInTheDocument();
});

function dataTransfer() {
  const store = new Map<string, string>();
  return {
    get types() {
      return [...store.keys()];
    },
    setData: (type: string, value: string) => store.set(type, value),
    getData: (type: string) => store.get(type) ?? "",
    dropEffect: "none",
    effectAllowed: "all",
    setDragImage: vi.fn((image: Element) => void badges.push(image.textContent ?? "")),
  };
}

const badges: string[] = [];

test("the header counts existing projects, and a project view counts its own PRs", async () => {
  const { unmount } = await show(<Harness rows={rows} />);
  expect(screen.getByText("3 open PRs across 2 projects · 1 unassigned")).toBeInTheDocument();
  unmount();
  await show(<Harness rows={rows.filter((item) => item.project_ids.includes(2))} viewKey="project:2" />);
  expect(screen.getByRole("heading", { level: 1, name: "Billing" })).toBeInTheDocument();
  expect(screen.getByText("1 open PR in this project")).toBeInTheDocument();
});

test("an empty project keeps its section, renames on double-click, and opens its dialog", async () => {
  const empty: ProjectDto = { id: 3, name: "Search", color: "#2f855a", archived: false, description: "", sort_order: 2 };
  const onRename = vi.fn(() => Promise.resolve());
  const onEditProject = vi.fn();
  await show(<Harness rows={rows} projects={[auth, billing, empty]} onRename={onRename} onEditProject={onEditProject} />);
  const section = screen.getByRole("region", { name: "Search" });
  expect(within(section).getByText(/Drag PRs onto this project/)).toBeInTheDocument();
  fireEvent.doubleClick(within(section).getByText("Search"));
  const input = screen.getByRole("textbox", { name: "Rename Search" });
  fireEvent.change(input, { target: { value: "Discovery" } });
  fireEvent.keyDown(input, { key: "Enter" });
  await act(async () => undefined);
  expect(onRename).toHaveBeenCalledWith(3, "Discovery");

  fireEvent.change(screen.getByPlaceholderText(/Search projects/), { target: { value: "passkeys" } });
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 250));
  });
  expect(screen.queryByRole("region", { name: "Search" })).not.toBeInTheDocument();
  fireEvent.change(screen.getByPlaceholderText(/Search projects/), { target: { value: "" } });
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 250));
  });
  fireEvent.click(within(screen.getByRole("region", { name: "Search" })).getByRole("button", { name: "Actions for Search" }));
  fireEvent.click(screen.getByRole("button", { name: "Edit auto-link rules" }));
  expect(onEditProject).toHaveBeenCalledWith(3);
});

test("the project view title renames on double-click", async () => {
  const onRename = vi.fn(() => Promise.resolve());
  await show(<Harness rows={rows.filter((item) => item.project_ids.includes(2))} viewKey="project:2" onRename={onRename} />);
  fireEvent.doubleClick(screen.getByRole("heading", { level: 1, name: "Billing" }));
  const input = screen.getByRole("textbox", { name: "Rename Billing" });
  fireEvent.change(input, { target: { value: "Payments" } });
  fireEvent.keyDown(input, { key: "Enter" });
  await act(async () => undefined);
  expect(onRename).toHaveBeenCalledWith(2, "Payments");
});

test("dragging a selected row onto a project section links the whole selection", async () => {
  const onLinkPrs = vi.fn();
  await show(<Harness rows={rows} onLinkPrs={onLinkPrs} />);
  fireEvent.click(screen.getByRole("checkbox", { name: /Select Add passkeys/ }));
  fireEvent.click(screen.getByRole("checkbox", { name: /Select Fix tax rounding/ }));
  const transfer = dataTransfer();
  const dragged = screen.getAllByText("Fix tax rounding")[0].closest("article")!;
  fireEvent.dragStart(dragged, { dataTransfer: transfer });
  expect(badges.at(-1)).toBe("2 PRs");
  const billingSection = screen.getByRole("region", { name: "Billing" });
  fireEvent.dragOver(billingSection, { dataTransfer: transfer });
  fireEvent.drop(billingSection, { dataTransfer: transfer });
  expect(onLinkPrs).toHaveBeenCalledWith(2, [2, 3]);

  const single = dataTransfer();
  fireEvent.dragStart(screen.getAllByText("Migrate login")[0].closest("article")!, { dataTransfer: single });
  expect(badges.at(-1)).toBe("1 PR");
  fireEvent.drop(billingSection, { dataTransfer: single });
  expect(onLinkPrs).toHaveBeenLastCalledWith(2, [1]);
});

test("the title button and Enter open a row's details", async () => {
  const onDetails = vi.fn();
  await show(<Harness rows={rows} onDetails={onDetails} />);
  fireEvent.click(screen.getByRole("button", { name: /Fix tax rounding/ }));
  expect(onDetails).toHaveBeenLastCalledWith(expect.objectContaining({ id: 3 }));
  fireEvent.keyDown(window, { key: "Enter" });
  expect(onDetails).toHaveBeenCalledTimes(2);
});

test("clicking anywhere on a row opens it, but not its controls or a text selection", async () => {
  const onDetails = vi.fn();
  await show(<Harness rows={rows} onDetails={onDetails} />);
  const article = screen.getByRole("article", { name: /Fix tax rounding/ });
  fireEvent.click(article);
  expect(onDetails).toHaveBeenLastCalledWith(expect.objectContaining({ id: 3 }));
  expect(onDetails).toHaveBeenCalledTimes(1);
  fireEvent.click(within(article).getByRole("checkbox"));
  expect(onDetails).toHaveBeenCalledTimes(1);
  for (const link of within(article).queryAllByRole("link")) {
    link.addEventListener("click", (event) => event.preventDefault());
    fireEvent.click(link);
  }
  expect(onDetails).toHaveBeenCalledTimes(1);
  const text = within(article).getByText(/acme\//);
  const selecting = vi.spyOn(window, "getSelection").mockReturnValue({ isCollapsed: false } as Selection);
  fireEvent.click(text);
  expect(onDetails).toHaveBeenCalledTimes(1);
  selecting.mockRestore();
  fireEvent.click(text);
  expect(onDetails).toHaveBeenCalledTimes(2);
});

test("with the detail panel open, arrows and clicks switch the PR it shows", async () => {
  const onDetails = vi.fn();
  const { rerender } = await show(<Harness rows={rows} onDetails={onDetails} />);
  const order = Array.from(document.querySelectorAll("article"));
  const idOf = (element: Element) => rows.find((item) => element.getAttribute("aria-label") === `#${item.number} ${item.title}`)?.id;
  // Closed panel: the arrows are left to the page.
  fireEvent.keyDown(window, { key: "ArrowDown" });
  expect(focused()).toBe(order[0].getAttribute("aria-label"));
  expect(onDetails).not.toHaveBeenCalled();

  rerender(<Harness rows={rows} onDetails={onDetails} detailId={999} />);
  fireEvent.keyDown(window, { key: "ArrowDown" });
  expect(focused()).toBe(order[1].getAttribute("aria-label"));
  expect(onDetails).toHaveBeenLastCalledWith(expect.objectContaining({ id: idOf(order[1]) }));
  fireEvent.keyDown(window, { key: "k" });
  expect(focused()).toBe(order[0].getAttribute("aria-label"));
  fireEvent.mouseDown(order[2]);
  fireEvent.click(order[2]);
  expect(onDetails).toHaveBeenLastCalledWith(expect.objectContaining({ id: idOf(order[2]) }));
  expect(onDetails).toHaveBeenCalledTimes(3);
});

function focused(): string | null {
  return document.querySelector('article[aria-current="true"]')?.getAttribute("aria-label") ?? null;
}

test("j and k move the focused row, x and Shift+x select, Esc clears", async () => {
  await show(<Harness rows={rows} />);
  const order = Array.from(document.querySelectorAll("article")).map((element) => element.getAttribute("aria-label"));
  expect(focused()).toBe(order[0]);
  fireEvent.keyDown(window, { key: "j" });
  expect(focused()).toBe(order[1]);
  fireEvent.keyDown(window, { key: "k" });
  fireEvent.keyDown(window, { key: "k" });
  expect(focused()).toBe(order[0]);
  fireEvent.keyDown(window, { key: "x" });
  expect(screen.getByText("1 selected")).toBeInTheDocument();
  fireEvent.keyDown(window, { key: "Escape" });
  expect(screen.queryByRole("region", { name: "Selection actions" })).not.toBeInTheDocument();
  // A browser reports Shift+x as "X": it toggles the focused row's section.
  fireEvent.keyDown(window, { key: "X", shiftKey: true });
  const auth = screen.getByRole("region", { name: "Auth" });
  expect(within(auth).getByRole("checkbox", { name: "Select Auth" })).toBeChecked();
  fireEvent.keyDown(window, { key: "a", metaKey: true });
  expect(screen.getByRole("checkbox", { name: "Select all" })).toBeChecked();
});

test("Esc clears the selection from a focused row checkbox, but not from the search field", async () => {
  await show(<Harness rows={rows} />);
  const first = screen.getAllByRole("checkbox", { name: "Select Migrate login" })[0];
  const second = screen.getByRole("checkbox", { name: "Select Add passkeys" });
  fireEvent.click(first);
  fireEvent.click(second);
  second.focus();
  expect(screen.getByRole("region", { name: "Selection actions" })).toBeInTheDocument();
  fireEvent.keyDown(second, { key: "Escape" });
  expect(screen.queryByRole("region", { name: "Selection actions" })).not.toBeInTheDocument();

  fireEvent.click(second);
  const search = screen.getByRole("textbox", { name: "Search" });
  fireEvent.change(search, { target: { value: "passkeys" } });
  fireEvent.keyDown(search, { key: "Escape" });
  expect(search).toHaveValue("");
  expect(screen.getByRole("region", { name: "Selection actions" })).toBeInTheDocument();
});

test("a merged PR is not listed as open and shows its state under the Archived filter", async () => {
  await show(<Harness rows={[...rows, row({ id: 4, number: 19, title: "Ship sync", state: "merged", project_ids: [1] })]} />);
  expect(screen.queryByRole("article", { name: /Ship sync/ })).not.toBeInTheDocument();
  expect(screen.getAllByText(/^3 open PRs across/)[0]).not.toHaveTextContent("merged");
  fireEvent.click(screen.getByRole("button", { name: "Filter menu" }));
  fireEvent.click(screen.getByRole("checkbox", { name: "Archived and closed" }));
  await settle();
  const merged = screen.getByRole("article", { name: /Ship sync/ });
  expect(within(merged).getByText("Merged")).toBeInTheDocument();
  expect(screen.getAllByText(/3 open PRs .*· 1 merged or closed/).length).toBeGreaterThan(0);
});

test("r requests the selection or the focused row, and system shortcuts are left alone", async () => {
  const onRequestReview = vi.fn();
  const onBulkReview = vi.fn();
  const onOpen = vi.fn();
  await show(<Harness rows={rows} onRequestReview={onRequestReview} onBulkReview={onBulkReview} onOpen={onOpen} />);
  fireEvent.keyDown(window, { key: "r", metaKey: true });
  fireEvent.keyDown(window, { key: "r", ctrlKey: true });
  fireEvent.keyDown(window, { key: "o", ctrlKey: true });
  expect(onRequestReview).not.toHaveBeenCalled();
  expect(onOpen).not.toHaveBeenCalled();
  fireEvent.keyDown(window, { key: "r" });
  expect(onRequestReview).toHaveBeenCalledTimes(1);
  fireEvent.keyDown(window, { key: "o" });
  expect(onOpen).toHaveBeenCalledWith(expect.stringMatching(/^https:\/\/github.com\/acme\/widgets\/pull\//));
  fireEvent.click(screen.getByRole("checkbox", { name: "Select Add passkeys" }));
  fireEvent.click(screen.getByRole("checkbox", { name: "Select Fix tax rounding" }));
  fireEvent.keyDown(window, { key: "r" });
  expect(onBulkReview).toHaveBeenCalledWith(expect.arrayContaining([2, 3]));
  expect(onRequestReview).toHaveBeenCalledTimes(1);
});

test("Enter on a button keeps its own action instead of opening the focused row", async () => {
  const onDetails = vi.fn();
  await show(<Harness rows={rows} onDetails={onDetails} />);
  fireEvent.click(screen.getByRole("checkbox", { name: "Select Add passkeys" }));
  const clear = screen.getByRole("button", { name: "Clear" });
  fireEvent.keyDown(clear, { key: "Enter" });
  expect(onDetails).not.toHaveBeenCalled();
});

test("the empty list says why and offers the way out", async () => {
  vi.useFakeTimers();
  const { unmount } = await show(<Harness rows={[]} projects={[]} />);
  expect(screen.getByText("No open PRs in your tracked orgs")).toBeInTheDocument();
  unmount();
  await show(<Harness rows={rows} />);
  fireEvent.change(screen.getByRole("textbox", { name: "Search" }), { target: { value: "zzz-nothing" } });
  await act(async () => {
    vi.advanceTimersByTime(200);
  });
  await settle();
  expect(screen.getByText("No pull requests in My PRs match your search.")).toBeInTheDocument();
  expect(screen.queryByText(/Matches elsewhere/)).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Clear search" }));
  await act(async () => {
    vi.advanceTimersByTime(200);
  });
  await settle();
  expect(screen.getAllByRole("article")).toHaveLength(4);
});

async function search(value: string) {
  fireEvent.change(screen.getByRole("textbox", { name: "Search" }), { target: { value } });
  await act(async () => {
    vi.advanceTimersByTime(SEARCH_DEBOUNCE_MS);
  });
  await settle();
}

const sspRows = [
  row({ id: 11, number: 123, title: "fix(bids): v2.1 time-outs", repo: "acme/service-ssp", url: "https://github.com/acme/service-ssp/pull/123", project_ids: [] }),
  row({ id: 12, number: 124, title: "Tune the floor", repo: "acme/service-ssp", involvement: 2, project_ids: [] }),
  row({ id: 13, number: 125, title: "Drop the old floor", repo: "acme/service-ssp", state: "merged", project_ids: [] }),
  ...rows,
];

test("searching a hyphenated repository name finds its PRs and select-all follows", async () => {
  vi.useFakeTimers();
  await show(<Harness rows={sspRows} />);
  for (const query of ["service-ssp", "acme/service-ssp", "ssp", "SERVICE-S", "#123", "v2.1"]) {
    await search(query);
    const shown = screen.getAllByRole("article");
    expect(shown, query).toHaveLength(1);
    expect(shown[0]).toHaveTextContent("fix(bids): v2.1 time-outs");
  }
  fireEvent.click(screen.getByRole("checkbox", { name: "Select all" }));
  expect(screen.getByRole("region", { name: "Selection actions" })).toHaveTextContent("1 selected");
});

test("an empty search points to the view and the closed rows that match", async () => {
  vi.useFakeTimers();
  await show(<Harness rows={sspRows} />);
  await search("floor");
  expect(screen.getByText("No pull requests in My PRs match your search.")).toBeInTheDocument();
  expect(screen.getByText("Matches elsewhere: 1 in To review, 1 merged, closed or archived.")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Include archived and closed (1)" }));
  await settle();
  expect(screen.getAllByRole("article")).toHaveLength(1);
  expect(screen.getByText("Drop the old floor")).toBeInTheDocument();
  await search("tune floor");
  fireEvent.click(screen.getByRole("button", { name: "Show in To review (1)" }));
  await settle();
  expect(screen.getByText("Tune the floor")).toBeInTheDocument();
  expect(screen.getByRole("textbox", { name: "Search" })).toHaveValue("tune floor");
});

test("the header and the bulk bar follow the mockup wording", async () => {
  await show(<Harness rows={rows} />);
  expect(screen.getByText("3 open PRs")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("checkbox", { name: "Select Add passkeys" }));
  const bar = screen.getByRole("region", { name: "Selection actions" });
  expect(within(bar).getByText("1 selected")).toBeInTheDocument();
  expect(within(bar).getByRole("button", { name: "Request / bump on Slack" })).toBeEnabled();
});

/** 130 open PRs in Auth: more than two pages of `PAGE_SIZE`. */
function many(count = 130): SearchRow[] {
  return Array.from({ length: count }, (_, index) =>
    row({ id: index + 1, number: 1000 + index, title: `Change ${index + 1}`, project_ids: [1], updated_at: new Date(Date.UTC(2026, 9, 26, 11, 0, 0) - index * 60_000).toISOString() }),
  );
}

// Billing first, so its empty section header is in the first page and can take a drop.
const billingFirst = { ...billing, sort_order: -1 };

test("header counts, select-all and drag cover the full result, not just the loaded page", async () => {
  const onLinkPrs = vi.fn();
  const all = many();
  await show(<Harness rows={all} projects={[auth, billingFirst]} onLinkPrs={onLinkPrs} />);
  expect(screen.getAllByRole("article").length).toBeLessThan(PAGE_SIZE + 1);
  expect(screen.getByText("130 open PRs")).toBeInTheDocument();
  const authSection = screen.getByRole("region", { name: "Auth" });
  expect(within(authSection).getByText("(130)")).toBeInTheDocument();

  fireEvent.click(screen.getByRole("checkbox", { name: "Select all" }));
  const bar = screen.getByRole("region", { name: "Selection actions" });
  expect(within(bar).getByText(/^130 selected/)).toBeInTheDocument();
  expect(screen.getByRole("checkbox", { name: "Select all" })).toBeChecked();

  const transfer = dataTransfer();
  fireEvent.dragStart(screen.getByText("Change 1").closest("article")!, { dataTransfer: transfer });
  const target = screen.getByRole("region", { name: "Billing" });
  fireEvent.dragOver(target, { dataTransfer: transfer });
  fireEvent.drop(target, { dataTransfer: transfer });
  expect(onLinkPrs).toHaveBeenCalledTimes(1);
  const [projectId, ids] = onLinkPrs.mock.calls[0] as [number, number[]];
  expect(projectId).toBe(2);
  expect([...ids].sort((a, b) => a - b)).toEqual(all.map((item) => item.id));
});

test("the section checkbox selects every PR in the section, loaded or not", async () => {
  await show(<Harness rows={many()} />);
  fireEvent.click(screen.getByRole("checkbox", { name: "Select Auth" }));
  expect(within(screen.getByRole("region", { name: "Selection actions" })).getByText(/^130 selected/)).toBeInTheDocument();
});

test("infinite scroll loads the next page when the sentinel comes into view", async () => {
  const observers: Array<(items: Array<{ isIntersecting: boolean }>) => void> = [];
  vi.stubGlobal(
    "IntersectionObserver",
    class {
      constructor(callback: (items: Array<{ isIntersecting: boolean }>) => void) {
        observers.push(callback);
      }
      observe() {}
      disconnect() {}
    },
  );
  try {
    const all = many();
    const requests: PageRequest[] = [];
    let release: (() => void) | null = null;
    const loadPage = (request: PageRequest) => {
      requests.push(request);
      const page = pageFromRows(all, [auth, billing], request);
      if (request.cursor === null) {
        return Promise.resolve(page);
      }
      return new Promise<PageDto>((resolve) => {
        release = () => resolve(page);
      });
    };
    await show(<Harness rows={all} loadPage={loadPage} />);
    const first = screen.getAllByRole("article").length;
    expect(first).toBeLessThan(130);
    expect(screen.queryByText("Change 130")).not.toBeInTheDocument();

    await act(async () => observers.at(-1)!([{ isIntersecting: true }]));
    expect(screen.getByText("Loading more…")).toBeInTheDocument();
    expect(requests.at(-1)!.cursor).not.toBeNull();
    expect(requests.at(-1)!.limit).toBe(PAGE_SIZE);
    // The sentinel scrolls away, so no read-ahead follows this page.
    observers.at(-1)!([{ isIntersecting: false }]);
    await act(async () => release!());
    await settle();
    expect(requests).toHaveLength(2);
    expect(screen.getAllByRole("article").length).toBeGreaterThan(first);
    expect(screen.queryByText("Change 130")).not.toBeInTheDocument();

    // The last page removes the loading row.
    await act(async () => observers.at(-1)!([{ isIntersecting: true }]));
    observers.at(-1)!([{ isIntersecting: false }]);
    await act(async () => release!());
    await settle();
    expect(requests).toHaveLength(3);
    expect(screen.getByText("Change 130")).toBeInTheDocument();
    expect(screen.getAllByRole("article")).toHaveLength(130);
    expect(screen.queryByText("Loading more…")).not.toBeInTheDocument();
    // Each page reports counts over the full set, so the header never jumps.
    expect(screen.getByText("130 open PRs")).toBeInTheDocument();
  } finally {
    vi.unstubAllGlobals();
  }
});

test("a scroll near the bottom also loads more, as a fallback for the observer", async () => {
  await show(<Harness rows={many()} />);
  const region = screen.getByText("Change 1").closest("[aria-busy]") as HTMLElement;
  const before = screen.getAllByRole("article").length;
  fireEvent.scroll(region);
  await settle();
  expect(screen.getAllByRole("article").length).toBeGreaterThan(before);
});

test("a slow search response that lands after a newer one is dropped", async () => {
  vi.useFakeTimers();
  const pending = new Map<string, () => void>();
  const loadPage = (request: PageRequest) => {
    const page = pageFromRows(rows, [auth, billing], request);
    if (!request.query) {
      return Promise.resolve(page);
    }
    return new Promise<PageDto>((resolve) => pending.set(request.query, () => resolve(page)));
  };
  await show(<Harness rows={rows} loadPage={loadPage} />);
  const search = screen.getByRole("textbox", { name: "Search" });

  fireEvent.change(search, { target: { value: "login" } });
  await act(async () => {
    vi.advanceTimersByTime(SEARCH_DEBOUNCE_MS);
  });
  expect(screen.getByText("Searching…")).toBeInTheDocument();
  fireEvent.change(search, { target: { value: "tax" } });
  await act(async () => {
    vi.advanceTimersByTime(SEARCH_DEBOUNCE_MS);
  });
  expect([...pending.keys()]).toEqual(["login", "tax"]);

  await act(async () => pending.get("tax")!());
  await settle();
  expect(screen.getByText("Fix tax rounding")).toBeInTheDocument();
  expect(screen.queryByText("Searching…")).not.toBeInTheDocument();

  // The older "login" answer arrives last and must not replace the list.
  await act(async () => pending.get("login")!());
  await settle();
  expect(screen.getByText("Fix tax rounding")).toBeInTheDocument();
  expect(screen.queryByText("Migrate login")).not.toBeInTheDocument();
});

test("typing waits for the debounce before it asks the backend", async () => {
  vi.useFakeTimers();
  const loadPage = vi.fn((request: PageRequest) => Promise.resolve(pageFromRows(rows, [auth, billing], request)));
  await show(<Harness rows={rows} loadPage={loadPage} />);
  const calls = loadPage.mock.calls.length;
  const search = screen.getByRole("textbox", { name: "Search" });
  for (const value of ["p", "pa", "pas", "pass"]) {
    fireEvent.change(search, { target: { value } });
    await act(async () => {
      vi.advanceTimersByTime(SEARCH_DEBOUNCE_MS - 50);
    });
  }
  expect(loadPage.mock.calls.length).toBe(calls);
  await act(async () => {
    vi.advanceTimersByTime(SEARCH_DEBOUNCE_MS);
  });
  await settle();
  expect(loadPage.mock.calls.length).toBe(calls + 1);
  expect(loadPage.mock.calls.at(-1)![0].query).toBe("pass");
});

test("a sync refresh re-reads what is loaded in place, without a loading state", async () => {
  const all = many();
  const loadPage = vi.fn((request: PageRequest) => Promise.resolve(pageFromRows(all, [auth, billing], request)));
  const { rerender } = await show(<Harness rows={all} loadPage={loadPage} version={0} />);
  fireEvent.scroll(screen.getByText("Change 1").closest("[aria-busy]") as HTMLElement);
  await settle();
  const loaded = screen.getAllByRole("article").length;
  rerender(<Harness rows={all} loadPage={loadPage} version={1} />);
  expect(screen.queryByText("Loading pull requests…")).not.toBeInTheDocument();
  expect(screen.getAllByRole("article").length).toBe(loaded);
  await settle();
  const refresh = loadPage.mock.calls.at(-1)![0];
  expect(refresh.cursor).toBeNull();
  expect(refresh.limit).toBeGreaterThan(PAGE_SIZE);
  expect(screen.getAllByRole("article").length).toBe(loaded);
});

test("Add to project links the selection to a picked project", async () => {
  const onLinkPrs = vi.fn();
  const archived: ProjectDto = { ...billing, id: 4, name: "Old work", archived: true };
  await show(<Harness rows={rows} projects={[auth, billing, archived]} onLinkPrs={onLinkPrs} />);
  fireEvent.click(screen.getByRole("checkbox", { name: /Select Fix tax rounding/ }));
  fireEvent.click(screen.getByRole("button", { name: "Add to project" }));
  const menu = screen.getByRole("menu", { name: "Add 1 PR to project" });
  expect(within(menu).queryByRole("menuitem", { name: /Old work/ })).not.toBeInTheDocument();
  fireEvent.click(within(menu).getByRole("menuitem", { name: /Billing/ }));
  expect(onLinkPrs).toHaveBeenCalledWith(2, [3]);
  expect(screen.queryByRole("menu", { name: /to project/ })).not.toBeInTheDocument();
});

test("New project from selection creates the project with the selected PRs", async () => {
  const onCreate = vi.fn((name: string) =>
    name === "Auth" ? Promise.reject(new Error("A project with that name already exists")) : Promise.resolve(),
  );
  await show(<Harness rows={rows} onCreateProjectWithPrs={onCreate} />);
  fireEvent.click(screen.getByRole("checkbox", { name: /Select Add passkeys/ }));
  fireEvent.click(screen.getByRole("checkbox", { name: /Select Fix tax rounding/ }));
  fireEvent.click(screen.getByRole("button", { name: "Add to project" }));
  fireEvent.click(screen.getByRole("menuitem", { name: /New project from selection/ }));
  const input = screen.getByRole("textbox", { name: "New project name" });
  expect(screen.getByText(/Enter to create with 2 PRs/)).toBeInTheDocument();

  fireEvent.change(input, { target: { value: "Auth" } });
  fireEvent.submit(input);
  expect(screen.getByText("A project with that name already exists")).toBeInTheDocument();
  expect(onCreate).not.toHaveBeenCalled();

  fireEvent.change(input, { target: { value: "Checkout" } });
  fireEvent.submit(input);
  await settle();
  expect(onCreate).toHaveBeenCalledWith("Checkout", [2, 3]);
  expect(screen.queryByRole("menu", { name: /to project/ })).not.toBeInTheDocument();
});

test("p opens Add to project for the focused row when nothing is selected", async () => {
  const onLinkPrs = vi.fn();
  await show(<Harness rows={rows} onLinkPrs={onLinkPrs} />);
  fireEvent.keyDown(window, { key: "p" });
  await settle();
  const menu = screen.getByRole("menu", { name: "Add 1 PR to project" });
  expect(within(menu).getAllByRole("menuitem")[0]).toHaveFocus();
  fireEvent.keyDown(within(menu).getAllByRole("menuitem")[0], { key: "ArrowDown" });
  expect(within(menu).getAllByRole("menuitem")[1]).toHaveFocus();
  fireEvent.keyDown(within(menu).getAllByRole("menuitem")[1], { key: "Escape" });
  expect(screen.queryByRole("menu", { name: /to project/ })).not.toBeInTheDocument();
  fireEvent.keyDown(window, { key: "p" });
  fireEvent.click(within(screen.getByRole("menu", { name: /to project/ })).getByRole("menuitem", { name: /Auth/ }));
  expect(onLinkPrs).toHaveBeenCalledTimes(1);
  expect(onLinkPrs.mock.calls[0][1]).toHaveLength(1);
});

test("quick filters show counts, filter one at a time and clear on a second click", async () => {
  const quickRows = [
    row({ id: 1, number: 1, title: "Draft work", is_draft: true, project_ids: [1] }),
    row({ id: 2, number: 2, title: "Ship it", review_decision: "approved", project_ids: [1] }),
    row({ id: 3, number: 3, title: "Broken build", checks: "failure", project_ids: [2] }),
  ];
  await show(<Harness rows={quickRows} />);
  const group = screen.getByRole("group", { name: "Quick filters" });
  const ready = within(group).getByRole("button", { name: "Ready to merge (1)" });
  expect(within(group).getByRole("button", { name: "Draft (1)" })).toHaveAttribute("aria-pressed", "false");
  expect(within(group).getByRole("button", { name: "Review declined (0)" })).toBeInTheDocument();

  fireEvent.click(ready);
  await settle();
  expect(ready).toHaveAttribute("aria-pressed", "true");
  expect(screen.getByRole("checkbox", { name: "Select Ship it" })).toBeInTheDocument();
  expect(screen.queryByRole("checkbox", { name: "Select Draft work" })).not.toBeInTheDocument();
  expect(screen.queryByRole("checkbox", { name: "Select Broken build" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("checkbox", { name: "Select all" }));
  expect(screen.getByText("1 selected")).toBeInTheDocument();

  fireEvent.click(within(group).getByRole("button", { name: "CI errors or conflicts (1)" }));
  await settle();
  expect(ready).toHaveAttribute("aria-pressed", "false");
  expect(screen.getByRole("checkbox", { name: "Select Broken build" })).toBeInTheDocument();
  expect(screen.queryByRole("checkbox", { name: "Select Ship it" })).not.toBeInTheDocument();

  fireEvent.click(within(group).getByRole("button", { name: "CI errors or conflicts (1)" }));
  await settle();
  expect(screen.getByRole("checkbox", { name: "Select Draft work" })).toBeInTheDocument();
  expect(screen.getByRole("checkbox", { name: "Select Ship it" })).toBeInTheDocument();
});

test("a row shows who still has to review, and a row without requests shows no stack", async () => {
  const reviewers = [
    { kind: "user" as const, login: "ada", name: "Ada Example", avatar_url: null },
    { kind: "user" as const, login: "bo", name: null, avatar_url: null },
    { kind: "team" as const, login: "acme/web", name: "Web Team", avatar_url: null },
    { kind: "team" as const, login: "acme/api", name: null, avatar_url: null },
  ];
  await show(
    <Harness
      rows={[
        row({ id: 1, title: "Needs eyes", project_ids: [1], reviewers_requested: true, pending_reviewers: reviewers }),
        row({ id: 2, title: "Nobody asked", project_ids: [1] }),
      ]}
    />,
  );
  const asked = screen.getByRole("article", { name: "#1 Needs eyes" });
  const stack = within(asked).getByRole("group", { name: "Waiting on 4 reviewers" });
  expect(within(stack).getByTestId("reviewer-more")).toHaveTextContent("+1");
  expect(within(screen.getByRole("article", { name: "#2 Nobody asked" })).queryByTestId("reviewer-avatars")).toBeNull();
});

test("with the panel open, rows go compact: wrapped title, status dot and Slack icon with tooltips, no avatars", async () => {
  await show(<Harness rows={rows} detailId={999} />);
  const article = screen.getAllByRole("article")[0];
  const title = within(article).getAllByRole("button").find((button) => button.className.includes("line-clamp-2"));
  expect(title).toBeDefined();
  expect(title?.getAttribute("title")).toMatch(/^#\d+ /);
  for (const pill of ["Review pending", "Approved", "Changes requested"]) {
    expect(screen.queryByText(pill)).not.toBeInTheDocument();
  }
  const reviewedRow = rows.find((item) => item.review_decision !== null);
  if (reviewedRow) {
    const article = screen.getByRole("article", { name: `#${reviewedRow.number} ${reviewedRow.title}` });
    expect(within(article).getByRole("img", { name: /Review pending|Approved|Changes requested/ })).toBeInTheDocument();
  }
  expect(screen.queryByText("Not requested")).not.toBeInTheDocument();
  expect(document.querySelector("[data-testid='reviewer-avatars']")).toBeNull();
});

test("every list command sits on the one toolbar row, the header keeps only the title", async () => {
  const { container } = await show(<Harness rows={rows} />);
  const toolbar = screen.getByRole("toolbar", { name: "List toolbar" });
  const inToolbar = within(toolbar);
  expect(inToolbar.getByRole("checkbox", { name: "Select all" })).toBeInTheDocument();
  expect(inToolbar.getByText("3 open PRs")).toBeInTheDocument();
  expect(inToolbar.getByRole("textbox", { name: "Search" })).toBeInTheDocument();
  expect(inToolbar.getByRole("combobox", { name: "Sort by" })).toBeInTheDocument();
  expect(inToolbar.getByRole("group", { name: "Quick filters" })).toBeInTheDocument();
  expect(inToolbar.getByRole("button", { name: "Filter menu" })).toBeInTheDocument();
  expect(toolbar.className).toContain("flex-nowrap");
  const header = container.querySelector("header") as HTMLElement;
  expect(within(header).getByRole("heading", { name: "My PRs" })).toBeInTheDocument();
  expect(within(header).queryByRole("textbox")).toBeNull();
  expect(within(header).queryByRole("button")).toBeNull();
});
