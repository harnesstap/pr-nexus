import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import type { Api, ContextThread, ListRow, PrOverview } from "../api";
import { DetailPanel, WIDTH_KEY, clampWidth } from "./DetailPanel";
import { mockDetail, mockOverview } from "./mock";
import { bumpView, latestThread } from "./SlackLine";
import { draftView } from "./draft";
import { peopleGroups, PeopleRow, personLine, requiredSeats } from "./People";

afterEach(() => window.localStorage.clear());

const row = {
  id: 7,
  number: 42,
  title: "Split the checkout into steps",
  url: "https://github.com/acme/web/pull/42",
  owner: "acme",
  repo: "acme/web",
  head_ref: "feature/checkout-steps",
  author_login: "ada",
  is_draft: false,
  state: "open",
  slack: null,
} as unknown as ListRow;

const thread: ContextThread = {
  workspace: "acme",
  channel_id: "C0123456789",
  channel_name: "eng-web",
  root_ts: "1700000000.000100",
  permalink: "https://acme.slack.com/archives/C0123456789/p1700000000000100",
  status: "active",
  close_reason: null,
  requested_at: "2023-11-14T22:13:20Z",
  last_bumped_at: null,
  bump_count: 1,
  messages: [
    {
      ts: "1",
      at: "2023-11-14T22:13:20Z",
      author: "Ada Acme",
      user_id: null,
      text: "Could someone review this?",
      permalink: null,
      is_bot: false,
    },
    {
      ts: "2",
      at: "2023-11-14T22:14:20Z",
      author: "Kim Lee",
      user_id: null,
      text: "Looking now",
      permalink: null,
      is_bot: false,
    },
    {
      ts: "3",
      at: "2023-11-14T22:15:20Z",
      author: "Sam Park",
      user_id: null,
      text: "Left two notes",
      permalink: null,
      is_bot: false,
    },
    {
      ts: "4",
      at: "2023-11-14T22:16:20Z",
      author: "Kim Lee",
      user_id: null,
      text: "Approved",
      permalink: null,
      is_bot: false,
    },
  ],
  has_more: false,
  fetched_at: null,
  from_cache: false,
  error: null,
};

function fakeApi(overrides: Partial<Api> = {}): Api {
  return {
    prOverview: vi.fn(() => Promise.resolve(mockOverview())),
    prNote: vi.fn((prId: number) => Promise.resolve({ pr_id: prId, note: "", updated_at: null })),
    setPrNote: vi.fn((prId: number, note: string) =>
      Promise.resolve({
        pr_id: prId,
        note,
        updated_at: "2026-01-02T10:00:00Z",
      }),
    ),
    prDetail: vi.fn((id: number) => Promise.resolve(mockDetail(id))),
    prSlackContext: vi.fn(() => Promise.resolve([])),
    setAutoMerge: vi.fn(() => Promise.resolve(mockOverview())),
    setPrDraft: vi.fn((_id: number, draft: boolean) => Promise.resolve(mockOverview({ is_draft: draft }))),
    mergePr: vi.fn(() => Promise.resolve(mockOverview({ state: "merged" }))),
    loadPreferences: vi.fn(() => Promise.resolve({ min_bump_interval_hours: 0 })),
    ...overrides,
  } as unknown as Api;
}

function renderPanel(api: Api, props: Partial<Parameters<typeof DetailPanel>[0]> = {}) {
  const handlers = {
    onClose: vi.fn(),
    onOpen: vi.fn(),
    onRequestReview: vi.fn(),
    onNotice: vi.fn(),
  };
  render(<DetailPanel api={api} row={row} version={0} {...handlers} {...props} />);
  return handlers;
}

test("shows a skeleton, then the cards, verdict and people from the overview", async () => {
  let resolve: (value: PrOverview) => void = () => undefined;
  const api = fakeApi({
    prOverview: vi.fn((_id: number, refresh: boolean) => (refresh ? new Promise<PrOverview>((done) => (resolve = done)) : Promise.resolve(null))),
  });
  renderPanel(api);
  expect(screen.getByRole("status", { name: "Loading pull request" })).toBeTruthy();
  await act(async () => resolve(mockOverview()));
  expect(screen.queryByRole("status", { name: "Loading pull request" })).toBeNull();
  expect(screen.getByRole("heading", { name: /#42 Split the checkout into steps/ })).toBeTruthy();
  expect(screen.getByText("Open")).toBeTruthy();
  expect(screen.getByText("main")).toBeTruthy();
  expect(screen.getByRole("button", { name: /Checks: 2\/4 passed, 1 failing/ })).toBeTruthy();
  expect(screen.getByLabelText("Comments: 8, 3 on the PR · 5 in code")).toBeTruthy();
  expect(screen.getByLabelText("Approvals: 1/2, Changes requested")).toBeTruthy();
  const verdict = screen.getByTestId("merge-verdict");
  expect(within(verdict).getByText("You can't merge")).toBeTruthy();
  // Every blocker is listed, not just the first.
  expect(
    within(within(verdict).getByRole("list", { name: "What's in the way" }))
      .getAllByRole("listitem")
      .map((item) => item.textContent),
  ).toEqual(["1 check failing: unit tests.", "A reviewer requested changes.", "It needs 1 more approval."]);
  expect(screen.getByLabelText(/^Conflicts: None/)).toBeTruthy();
  const reviewers = screen.getByRole("group", { name: "Reviewers" });
  expect(reviewers.querySelectorAll("[data-login]")).toHaveLength(3);
  expect(reviewers.querySelector('[data-state="changes_requested"]')?.getAttribute("data-login")).toBe("sam");
  // The viewer (ada) wrote it, so no Author group; order is fixed.
  const groups = [...screen.getByTestId("people-row").querySelectorAll("[data-group]")].map((group) => group.getAttribute("data-group"));
  expect(groups).toEqual(["reviewers", "assignees", "commenters"]);
  expect(screen.queryByText("Participants")).toBeNull();
});

test("the checks card lists the checks that need attention", async () => {
  const api = fakeApi();
  const { onOpen } = renderPanel(api);
  const card = await screen.findByRole("button", { name: /Checks:/ });
  expect(screen.queryByRole("list", { name: "Checks needing attention" })).toBeNull();
  fireEvent.click(card);
  const list = screen.getByRole("list", { name: "Checks needing attention" });
  expect(
    within(list)
      .getAllByRole("listitem")
      .map((item) => item.textContent),
  ).toEqual([expect.stringContaining("unit tests"), expect.stringContaining("preview deploy")]);
  fireEvent.click(within(list).getByRole("button", { name: "Logs ↗" }));
  expect(onOpen).toHaveBeenCalledWith("https://example.test/runs/1");
});

test("cached overview shows first and a failed refresh keeps it with a note", async () => {
  const cached = mockOverview({ issue_comments: 9, review_comments: 0 });
  const api = fakeApi({
    prOverview: vi.fn((_id: number, refresh: boolean) => (refresh ? Promise.reject(new Error("GitHub is down")) : Promise.resolve(cached))),
  });
  renderPanel(api);
  expect(await screen.findByLabelText("Comments: 9, 9 on the PR · 0 in code")).toBeTruthy();
  expect(await screen.findByText(/refreshing failed: GitHub is down/)).toBeTruthy();
});

test("a sync refreshes the overview", async () => {
  const api = fakeApi();
  const { rerender } = render(<DetailPanel api={api} row={row} version={0} onClose={vi.fn()} onOpen={vi.fn()} onNotice={vi.fn()} />);
  await screen.findByTestId("merge-verdict");
  rerender(<DetailPanel api={api} row={row} version={1} onClose={vi.fn()} onOpen={vi.fn()} onNotice={vi.fn()} />);
  await waitFor(() => expect(vi.mocked(api.prOverview).mock.calls.filter(([, refresh]) => refresh)).toHaveLength(2));
});

test("with no Slack request, one big button asks for review, right under the verdict", async () => {
  const api = fakeApi();
  const { onRequestReview } = renderPanel(api);
  const slot = await screen.findByTestId("slack-empty");
  const button = within(slot).getByRole("button", {
    name: "Request review on Slack",
  });
  // The header no longer carries a small Slack button.
  expect(screen.getAllByRole("button", { name: /on Slack/ })).toEqual([button]);
  expect(screen.getByTestId("merge-verdict").compareDocumentPosition(slot) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  fireEvent.click(button);
  expect(onRequestReview).toHaveBeenCalled();
  expect(screen.queryByRole("region", { name: "Slack request" })).toBeNull();
});

test("an existing request shows its summary with a Bump button in the same spot", async () => {
  const api = fakeApi({
    prSlackContext: vi.fn(() => Promise.resolve([thread])),
  });
  const { onRequestReview } = renderPanel(api);
  const region = await screen.findByRole("region", { name: "Slack request" });
  expect(screen.queryByTestId("slack-empty")).toBeNull();
  fireEvent.click(within(region).getByRole("button", { name: /^Bump/ }));
  expect(onRequestReview).toHaveBeenCalled();
});

test("a draft shows Mark ready for review on its blocker line and flips optimistically", async () => {
  let finish: (value: PrOverview) => void = () => undefined;
  const api = fakeApi({
    prOverview: vi.fn(() => Promise.resolve(mockOverview({ is_draft: true, merge_state: "draft" }))),
    setPrDraft: vi.fn(() => new Promise<PrOverview>((done) => (finish = done))),
  });
  const onDraftChange = vi.fn();
  const { onNotice } = renderPanel(api, { onDraftChange });
  const verdict = await screen.findByTestId("merge-verdict");
  expect(screen.getByTestId("state-badge").textContent).toBe("Draft");
  // The header holds no toggle: the action lives in the merge card.
  expect(
    within(screen.getByRole("banner")).queryByRole("button", {
      name: /ready|draft/i,
    }),
  ).toBeNull();
  const line = within(verdict).getByText("It is still a draft.");
  const ready = within(line.closest("li, p") as HTMLElement).getByRole("button", { name: "Mark ready for review" });
  fireEvent.click(ready);
  expect(api.setPrDraft).toHaveBeenCalledWith(7, false);
  expect(onDraftChange).toHaveBeenCalledWith(7, false);
  expect(within(verdict).getByTestId("draft-action").textContent).toContain("Saving");
  await act(async () => finish(mockOverview({ is_draft: false })));
  expect(onDraftChange).toHaveBeenLastCalledWith(7, "saved");
  expect(onNotice).toHaveBeenCalledWith("#42 is ready for review");
  expect(screen.getByTestId("state-badge").textContent).toBe("Open");
  expect(within(verdict).getByRole("button", { name: "Convert to draft" })).toBeTruthy();
});

test("a failed draft flip rolls back and explains the plan limit", async () => {
  const api = fakeApi({
    setPrDraft: vi.fn(() => Promise.reject(new Error("draft pull requests aren't available on this repository's plan"))),
  });
  const onDraftChange = vi.fn();
  const { onNotice } = renderPanel(api, { onDraftChange });
  const verdict = await screen.findByTestId("merge-verdict");
  fireEvent.click(within(verdict).getByRole("button", { name: "Convert to draft" }));
  expect(onDraftChange).toHaveBeenCalledWith(7, true);
  await waitFor(() => expect(onDraftChange).toHaveBeenLastCalledWith(7, null));
  expect(screen.getByTestId("state-badge").textContent).toBe("Open");
  expect(onNotice).toHaveBeenCalledWith("Couldn't convert to draft: draft pull requests aren't available on this repository's plan");
});

test("the draft toggle is off, with a reason, without access", async () => {
  const api = fakeApi({
    prOverview: vi.fn(() =>
      Promise.resolve(
        mockOverview({
          viewer: {
            ...mockOverview().viewer,
            login: "lee",
            permission: "read",
          },
        }),
      ),
    ),
  });
  renderPanel(api);
  await screen.findByTestId("merge-verdict");
  expect((screen.getByTestId("draft-action") as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByTestId("draft-action-disabled")).toBeTruthy();
});

test("the four summary cards share one frame", async () => {
  renderPanel(fakeApi());
  const grid = await screen.findByTestId("stat-cards");
  expect(grid.className).toContain("auto-rows-fr");
  const frames = Array.from(grid.children).map(
    (child) => (child.matches("button, div[aria-label]") ? child : child.querySelector("div[aria-label]")) as HTMLElement,
  );
  expect(frames).toHaveLength(4);
  for (const frame of frames) {
    expect(frame.className).toContain("h-full");
    expect(frame.className).toContain("w-full");
    expect(frame.querySelector("p.min-h-8")).toBeTruthy();
  }
});

test("a Slack request shows compactly with its latest replies", async () => {
  const older = {
    ...thread,
    root_ts: "1600000000.000100",
    requested_at: "2023-01-01T00:00:00Z",
    channel_name: "old-reviews",
  };
  const api = fakeApi({
    prSlackContext: vi.fn(() => Promise.resolve([older, thread])),
  });
  renderPanel(api);
  const region = await screen.findByRole("region", { name: "Slack request" });
  expect(within(region).getByText("#eng-web")).toBeTruthy();
  expect(region.textContent).toContain("bumped 1×");
  expect(region.textContent).toContain("2 threads");
  expect(within(region).getByText("Could someone review this?")).toBeTruthy();
  expect(within(region).getByText("+1 earlier reply")).toBeTruthy();
  expect(within(region).getByText("Left two notes")).toBeTruthy();
  expect(within(region).getByText("Approved")).toBeTruthy();
  expect(within(region).queryByText("Looking now")).toBeNull();
  expect(screen.queryByTestId("slack-empty")).toBeNull();
});

test("files and details start collapsed and open with plain sentences", async () => {
  const api = fakeApi();
  renderPanel(api);
  const files = await screen.findByRole("button", {
    name: /^Files changed \(3\)\s*with code owners$/,
  });
  const details = screen.getByRole("button", { name: "Details" });
  expect(files.getAttribute("aria-expanded")).toBe("false");
  expect(details.getAttribute("aria-expanded")).toBe("false");
  expect(screen.queryByText("src/checkout/Steps.tsx")).toBeNull();

  fireEvent.click(files);
  const list = screen.getByRole("list", { name: "Changed files" });
  expect(within(list).getAllByText("@acme/web")).toHaveLength(2);
  expect(within(list).getByText("no owner")).toBeTruthy();

  fireEvent.click(details);
  const lines = within(screen.getByRole("list", { name: "Details" }))
    .getAllByRole("listitem")
    .map((item) => item.textContent);
  expect(lines).toEqual([
    "Check “unit tests” failed.",
    "@acme/web still has to approve: it owns changed files.",
    "Branch protection requires code-owner reviews.",
  ]);
  // No internal keys leak into the sentences.
  expect(lines.join(" ")).not.toMatch(/_|BLOCKED|UNKNOWN/);
  fireEvent.click(details);
  expect(screen.queryByRole("list", { name: "Details" })).toBeNull();
});

test("auto-merge turns on with the chosen method and shows a toast", async () => {
  const enabled = mockOverview({
    auto_merge: { method: "merge", enabled_by: "ada", enabled_at: null },
  });
  const api = fakeApi({ setAutoMerge: vi.fn(() => Promise.resolve(enabled)) });
  const { onNotice } = renderPanel(api);
  const method = await screen.findByRole("combobox", { name: "Merge method" });
  expect((method as HTMLSelectElement).value).toBe("squash");
  expect(
    within(method)
      .getAllByRole("option")
      .map((option) => option.textContent),
  ).toEqual(["Squash and merge", "Merge commit"]);
  fireEvent.change(method, { target: { value: "merge" } });
  fireEvent.click(screen.getByRole("button", { name: "Enable auto-merge" }));
  await screen.findByText(/Auto-merge on · Merge commit · by @ada/);
  expect(api.setAutoMerge).toHaveBeenCalledWith(7, "merge");
  expect(onNotice).toHaveBeenCalledWith("Auto-merge on: it will merge commit when ready.");
});

test("auto-merge errors become a toast and unavailable controls explain why", async () => {
  const api = fakeApi({
    setAutoMerge: vi.fn(() => Promise.reject(new Error("Pull request is in clean status"))),
  });
  const { onNotice } = renderPanel(api);
  fireEvent.click(await screen.findByRole("button", { name: "Enable auto-merge" }));
  await waitFor(() => expect(onNotice).toHaveBeenCalledWith("Could not change auto-merge: Pull request is in clean status"));

  const draft = fakeApi({ prOverview: vi.fn(() => Promise.resolve(mockOverview({ is_draft: true }))) });
  cleanup();
  renderPanel(draft);
  const wrapper = await screen.findByTestId("auto-merge-disabled");
  expect(
    (
      within(wrapper).getByRole("button", {
        name: "Enable auto-merge",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(true);
  fireEvent.focus(wrapper);
  expect(await screen.findByText(/Drafts can't auto-merge/)).toBeTruthy();
});

test("Esc and the close button close it; Esc waits for a dialog on top", async () => {
  const api = fakeApi();
  const { onClose } = renderPanel(api);
  await screen.findByTestId("merge-verdict");
  const modal = document.createElement("div");
  modal.setAttribute("role", "dialog");
  modal.setAttribute("aria-modal", "true");
  document.body.appendChild(modal);
  fireEvent.keyDown(document.body, { key: "Escape" });
  expect(onClose).not.toHaveBeenCalled();
  modal.remove();
  fireEvent.keyDown(document.body, { key: "Escape" });
  expect(onClose).toHaveBeenCalledTimes(1);
  fireEvent.click(screen.getByRole("button", { name: "Close details" }));
  expect(onClose).toHaveBeenCalledTimes(2);
});

test("the width is a remembered share of the window, within limits", async () => {
  window.localStorage.setItem(WIDTH_KEY, "0.5");
  renderPanel(fakeApi());
  const separator = screen.getByRole("separator", {
    name: "Resize the detail panel",
  });
  expect(separator.getAttribute("aria-valuenow")).toBe("50");
  fireEvent.keyDown(separator, { key: "ArrowLeft" });
  expect(window.localStorage.getItem(WIDTH_KEY)).toBe("0.52");
  expect(clampWidth(0.95)).toBe(0.7);
  expect(clampWidth(0.1)).toBe(0.28);
  expect(clampWidth(Number.NaN)).toBe(0.4);
  await screen.findByTestId("merge-verdict");
});

test("helpers: newest Slack thread", () => {
  expect(latestThread([])).toBeNull();
  expect(latestThread([{ ...thread, requested_at: null, root_ts: "9" }, thread])?.root_ts).toBe(thread.root_ts);
});

test("draftView: authors and writers may flip; closed, merged and readers may not", () => {
  const open = { ...row, author_login: "ada" } as ListRow;
  expect(draftView(open, mockOverview())).toMatchObject({
    available: true,
    action: "Convert to draft",
  });
  expect(draftView(open, mockOverview({ is_draft: true }))).toMatchObject({
    available: true,
    action: "Mark ready for review",
  });
  const reader = mockOverview({
    viewer: { ...mockOverview().viewer, login: "lee", permission: "read" },
  });
  expect(draftView(open, reader).reason).toMatch(/author or someone with write access/);
  expect(draftView({ ...open, author_login: "lee" } as ListRow, reader).available).toBe(true);
  expect(draftView(open, mockOverview({ state: "merged" })).reason).toBe("It is already merged.");
  expect(draftView(open, mockOverview({ state: "closed" })).reason).toBe("The pull request is closed.");
  expect(draftView(open, null).available).toBe(false);
});

const user = (login: string) => ({
  kind: "user" as const,
  login,
  name: null,
  avatar_url: null,
});

test("code-owner teams not yet requested show as pending reviewers", () => {
  const seats = requiredSeats(
    [
      {
        kind: "team",
        login: "acme/web",
        name: null,
        avatar_url: null,
        state: "pending",
      },
    ],
    ["@acme/web", "@acme/api", "acme/api", "@acme/docs"],
    ["acme/docs"],
  );
  expect(seats.map((seat) => seat.login)).toEqual(["acme/api"]);
  const groups = peopleGroups(mockOverview({ reviewers: [] }), seats);
  expect(groups[0]).toMatchObject({ key: "reviewers" });
  expect(personLine(groups[0].people[0])).toBe("acme/api · Code owner · waiting");
});

test("people groups: author hidden for the viewer, shown otherwise, in a fixed order", () => {
  const mine = mockOverview({
    viewer: { ...mockOverview().viewer, login: "ADA" },
  });
  expect(peopleGroups(mine).map((group) => group.key)).toEqual(["reviewers", "assignees", "commenters"]);
  const theirs = mockOverview({ author: user("grace") });
  expect(peopleGroups(theirs).map((group) => group.key)).toEqual(["author", "reviewers", "assignees", "commenters"]);
});

test("people groups: empty groups are dropped and the row disappears when all are", () => {
  const none = mockOverview({
    author: null,
    reviewers: [],
    assignees: [],
    commenters: [],
  });
  expect(peopleGroups(none)).toEqual([]);
  const { container } = render(<PeopleRow groups={[]} />);
  expect(container.innerHTML).toBe("");
  render(<PeopleRow groups={peopleGroups(mockOverview({ reviewers: [], assignees: [], commenters: [] }))} />);
  expect(screen.queryByRole("group", { name: "Reviewers" })).toBeNull();
  expect(screen.queryByRole("group", { name: "Assignees" })).toBeNull();
  expect(screen.queryByRole("group", { name: "Commenters" })).toBeNull();
});

test("with no reviewers, the row says how many approvals are missing", () => {
  render(<PeopleRow groups={[]} missing={1} />);
  expect(screen.getByText(/1 approval needed/)).toBeTruthy();
});

test("people groups dedupe within a group; commenters skip the viewer, author and reviewers", () => {
  const overview = mockOverview({
    author: user("grace"),
    assignees: [user("noor"), user("NOOR")],
    commenters: [user("ada"), user("grace"), user("Kim"), user("lee"), user("LEE"), { kind: "bot", login: "ci-bot", name: null, avatar_url: null }],
  });
  const byKey = Object.fromEntries(peopleGroups(overview).map((group) => [group.key, group.people.map((person) => person.login)]));
  expect(byKey.assignees).toEqual(["noor"]);
  expect(byKey.commenters).toEqual(["lee", "ci-bot"]);
});

test("bots keep their marker; groups past four avatars fold into +N", () => {
  const many = Array.from({ length: 6 }, (_, index) => user(`u${index}`));
  render(
    <PeopleRow
      groups={[
        {
          key: "commenters",
          label: "Commenters",
          people: [{ kind: "bot", login: "ci-bot", name: null, avatar_url: null }, ...many],
        },
      ]}
    />,
  );
  expect(screen.getByTestId("commenters-more").textContent).toBe("+3");
  expect(personLine({ kind: "bot", login: "ci-bot", name: null, avatar_url: null })).toBe("@ci-bot · bot");
});

const mergeable = () =>
  mockOverview({
    merge_state: "clean",
    review_decision: "approved",
    approvals: 2,
    checks: [],
  });

test("when it can merge now, Merge asks to confirm, then merges with the chosen method", async () => {
  const api = fakeApi({
    prOverview: vi.fn(() => Promise.resolve(mergeable())),
  });
  const { onNotice } = renderPanel(api);
  const verdict = await screen.findByTestId("merge-verdict");
  expect(within(verdict).getByText("You can merge")).toBeTruthy();
  fireEvent.change(within(verdict).getByRole("combobox", { name: "Merge method" }), { target: { value: "merge" } });
  fireEvent.click(within(verdict).getByRole("button", { name: "Merge" }));
  // The first click only asks.
  expect(api.mergePr).not.toHaveBeenCalled();
  const confirm = within(verdict).getByRole("group", { name: "Confirm merge" });
  expect(confirm.textContent).toContain("Merge commit now?");
  fireEvent.click(within(confirm).getByRole("button", { name: "Cancel" }));
  expect(within(verdict).queryByRole("group", { name: "Confirm merge" })).toBeNull();
  fireEvent.click(within(verdict).getByRole("button", { name: "Merge" }));
  fireEvent.click(within(verdict).getByRole("button", { name: "Confirm merge" }));
  await waitFor(() => expect(api.mergePr).toHaveBeenCalledWith(7, "merge"));
  await waitFor(() => expect(onNotice).toHaveBeenCalledWith("Merged: merge commit done."));
  expect(within(verdict).queryByRole("button", { name: "Merge" })).toBeNull();
});

test("no Merge button while something blocks it, and merge errors become a toast", async () => {
  renderPanel(fakeApi());
  const verdict = await screen.findByTestId("merge-verdict");
  expect(within(verdict).queryByRole("button", { name: "Merge" })).toBeNull();

  cleanup();
  const failing = fakeApi({ prOverview: vi.fn(() => Promise.resolve(mergeable())), mergePr: vi.fn(() => Promise.reject(new Error("Base branch was modified"))) });
  const { onNotice } = renderPanel(failing);
  fireEvent.click(await screen.findByRole("button", { name: "Merge" }));
  fireEvent.click(screen.getByRole("button", { name: "Confirm merge" }));
  await waitFor(() => expect(onNotice).toHaveBeenCalledWith("Could not merge: Base branch was modified"));
});

test("Bump says when it was last poked and waits for the minimum interval from settings", async () => {
  const recent = {
    ...thread,
    last_bumped_at: new Date(Date.now() - 2 * 3_600_000).toISOString(),
  };
  const api = fakeApi({
    prSlackContext: vi.fn(() => Promise.resolve([recent])),
    loadPreferences: vi.fn(() => Promise.resolve({ min_bump_interval_hours: 6 })),
  } as unknown as Partial<Api>);
  const { onRequestReview } = renderPanel(api);
  const region = await screen.findByRole("region", { name: "Slack request" });
  const bump = within(region).getByRole("button", {
    name: "Bump · last 2h ago",
  }) as HTMLButtonElement;
  await waitFor(() => expect(bump.disabled).toBe(true));
  fireEvent.click(bump);
  expect(onRequestReview).not.toHaveBeenCalled();
  fireEvent.focus(screen.getByTestId("bump-disabled"));
  expect(await screen.findByText(/one bump every 6h\. Try again in about 4h/)).toBeTruthy();
  expect(within(region).getByRole("button", { name: "Open in Slack ↗" })).toBeTruthy();
});

test("bumpView: no limit, too soon, and never requested", () => {
  const now = new Date("2024-01-01T12:00:00Z");
  expect(bumpView({ last_bumped_at: null, requested_at: "2024-01-01T06:00:00Z" }, 0, now)).toEqual({ last: "6h ago", tooSoon: null });
  expect(
    bumpView(
      {
        last_bumped_at: "2024-01-01T11:00:00Z",
        requested_at: "2024-01-01T06:00:00Z",
      },
      24,
      now,
    ).tooSoon,
  ).toMatch(/about 23h/);
  expect(bumpView({ last_bumped_at: "2024-01-01T11:00:00Z", requested_at: null }, 1, now).tooSoon).toBeNull();
  expect(bumpView({ last_bumped_at: null, requested_at: null }, 6, now)).toEqual({ last: null, tooSoon: null });
});

test("people groups: the viewer is hidden from assignees too", () => {
  const overview = mockOverview({ assignees: [user("ada"), user("noor")] });
  const byKey = Object.fromEntries(peopleGroups(overview).map((group) => [group.key, group.people.map((person) => person.login)]));
  expect(byKey.assignees).toEqual(["noor"]);
  expect(peopleGroups(mockOverview({ assignees: [user("ADA")] })).map((group) => group.key)).not.toContain("assignees");
});
