import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import type { Api, BranchStatus, ListRow, PrOverview } from "../api";
import { BranchCard, BranchError, CHECKING_DELAYS_MS, RebaseConfirm, useBranch } from "./BranchCard";
import { DetailPanel } from "./DetailPanel";
import { mockBranch, mockDetail, mockOverview } from "./mock";

afterEach(() => vi.useRealTimers());

function fakeApi(branch: BranchStatus, overrides: Partial<Api> = {}): Api {
  return {
    prOverview: vi.fn(() => Promise.resolve(mockOverview())),
    prBranchStatus: vi.fn(() => Promise.resolve(branch)),
    updatePrBranch: vi.fn(() => Promise.resolve({ overview: mockOverview(), branch: mockBranch() })),
    ...overrides,
  } as unknown as Api;
}

function Harness({ api, overview, onOpen, onNotice }: { api: Api; overview: PrOverview; onOpen: (url: string) => void; onNotice: (text: string) => void }) {
  const branch = useBranch({ api, prId: 7, overview, fresh: true, onOverview: () => undefined, onNotice });
  return (
    <>
      <BranchCard branch={branch} onOpen={onOpen} />
      <BranchError branch={branch} />
      <RebaseConfirm branch={branch} headRef={overview.head_ref} />
    </>
  );
}

function renderCard(branch: BranchStatus, overview: PrOverview = mockOverview(), overrides: Partial<Api> = {}) {
  const api = fakeApi(branch, overrides);
  const onOpen = vi.fn();
  const onNotice = vi.fn();
  render(<Harness api={api} overview={overview} onOpen={onOpen} onNotice={onNotice} />);
  return { api, onOpen, onNotice, card: screen.getByTestId("branch-card") };
}

const icon = (card: HTMLElement) => card.querySelector("[data-icon]")?.getAttribute("data-icon");

test("up to date: a green check and the base, never the word None", async () => {
  const { card } = renderCard(mockBranch());
  await waitFor(() => expect(card.dataset.branchState).toBe("up_to_date"));
  expect(card.textContent).toContain("Up to date with main");
  expect(card.textContent).not.toContain("None");
  expect(icon(card)).toBe("check");
  expect(within(card).queryByRole("button")).toBeNull();
});

test("behind but optional: a ringed check, the count, and Update branch merges by default", async () => {
  const { card, api, onNotice } = renderCard(mockBranch({ state: "behind", behind_by: 3 }));
  await waitFor(() => expect(card.dataset.branchState).toBe("behind"));
  expect(card.textContent).toContain("3 commits behind main");
  expect(icon(card)).toBe("check-ring");
  fireEvent.click(within(card).getByRole("button", { name: "Update branch options" }));
  const menu = screen.getByRole("menu", { name: "Update branch" });
  expect(
    within(menu)
      .getAllByRole("menuitem")
      .map((item) => item.textContent),
  ).toEqual([expect.stringContaining("Merge main into branch"), expect.stringContaining("Rebase")]);
  fireEvent.click(within(card).getByRole("button", { name: "Update branch" }));
  await waitFor(() => expect(api.updatePrBranch).toHaveBeenCalledWith(7, "merge"));
  await waitFor(() => expect(onNotice).toHaveBeenCalledWith("Merged main into the branch."));
});

test("update required: a warning and the count, from the strict merge state", async () => {
  const { card } = renderCard(mockBranch({ state: "update_required", behind_by: 2 }), mockOverview({ merge_state: "behind" }));
  await waitFor(() => expect(card.textContent).toContain("2 commits behind · update required"));
  expect(card.dataset.branchState).toBe("update_required");
  expect(icon(card)).toBe("warning");
  expect(within(card).getByRole("button", { name: "Update branch" })).toBeTruthy();
});

test("conflicts: a red X and Resolve on GitHub opens the conflicts page", async () => {
  const url = "https://github.com/acme/web/pull/42/conflicts";
  const { card, onOpen, api } = renderCard(
    mockBranch({ state: "conflicts", behind_by: null, conflicts_url: url }),
    mockOverview({ mergeable: "conflicting", merge_state: "dirty" }),
  );
  await waitFor(() => expect(api.prBranchStatus).toHaveBeenCalled());
  expect(card.dataset.branchState).toBe("conflicts");
  expect(card.textContent).toContain("Conflicts with main");
  expect(icon(card)).toBe("x");
  fireEvent.click(await within(card).findByRole("button", { name: /Resolve on GitHub/ }));
  expect(onOpen).toHaveBeenCalledWith(url);
});

test("checking: a spinner, and the overview is re-polled with backoff", async () => {
  vi.useFakeTimers();
  const checking = mockOverview({ mergeable: "unknown", merge_state: "unknown" });
  const prOverview = vi.fn(() => Promise.resolve(checking));
  const { card } = renderCard(mockBranch({ state: "checking", behind_by: null }), checking, { prOverview } as Partial<Api>);
  expect(card.dataset.branchState).toBe("checking");
  expect(card.textContent).toContain("Checking…");
  expect(icon(card)).toBe("spinner");
  await act(async () => vi.advanceTimersByTimeAsync(CHECKING_DELAYS_MS[0] - 1));
  expect(prOverview).not.toHaveBeenCalled();
  await act(async () => vi.advanceTimersByTimeAsync(1));
  expect(prOverview).toHaveBeenCalledTimes(1);
  expect(prOverview).toHaveBeenCalledWith(7, true);
  await act(async () => vi.advanceTimersByTimeAsync(CHECKING_DELAYS_MS[1]));
  expect(prOverview).toHaveBeenCalledTimes(2);
  for (const delay of CHECKING_DELAYS_MS.slice(2)) {
    await act(async () => vi.advanceTimersByTimeAsync(delay));
  }
  await act(async () => vi.advanceTimersByTimeAsync(60_000));
  expect(prOverview).toHaveBeenCalledTimes(CHECKING_DELAYS_MS.length);
});

test("rebase asks first, explains the history rewrite, and warns about other authors", async () => {
  const { card, api } = renderCard(mockBranch({ state: "behind", behind_by: 1, other_authors: ["grace"] }));
  await waitFor(() => expect(card.dataset.branchState).toBe("behind"));
  fireEvent.click(within(card).getByRole("button", { name: "Update branch options" }));
  fireEvent.click(screen.getByRole("menuitem", { name: /Rebase/ }));
  const dialog = screen.getByRole("dialog", { name: /Rebase feature\/checkout-steps onto main/ });
  expect(dialog.textContent).toMatch(/rewrites the branch history/);
  expect(within(dialog).getByRole("alert").textContent).toContain("@grace");
  expect(api.updatePrBranch).not.toHaveBeenCalled();
  fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  fireEvent.click(within(card).getByRole("button", { name: "Update branch options" }));
  fireEvent.click(screen.getByRole("menuitem", { name: /Rebase/ }));
  fireEvent.click(screen.getByRole("button", { name: "Rebase branch" }));
  await waitFor(() => expect(api.updatePrBranch).toHaveBeenCalledWith(7, "rebase"));
});

test("no extra warning when only you committed on the branch", async () => {
  const { card } = renderCard(mockBranch({ state: "behind", behind_by: 1 }));
  await waitFor(() => expect(card.dataset.branchState).toBe("behind"));
  fireEvent.click(within(card).getByRole("button", { name: "Update branch options" }));
  fireEvent.click(screen.getByRole("menuitem", { name: /Rebase/ }));
  expect(within(screen.getByRole("dialog")).queryByRole("alert")).toBeNull();
});

test("a failed update shows the error banner", async () => {
  const { card } = renderCard(mockBranch({ state: "behind", behind_by: 1 }), mockOverview(), {
    updatePrBranch: vi.fn(() => Promise.reject({ kind: "head_moved", message: "The branch moved since you loaded it. Refresh and try again." })),
  } as Partial<Api>);
  await waitFor(() => expect(card.dataset.branchState).toBe("behind"));
  fireEvent.click(within(card).getByRole("button", { name: "Update branch" }));
  expect((await screen.findByRole("alert")).textContent).toContain("Couldn't update the branch: The branch moved");
});

test("the update-required line also sits in the blocker list, with the same action", async () => {
  const row = {
    id: 7,
    number: 42,
    title: "Split the checkout",
    url: "https://github.com/acme/web/pull/42",
    owner: "acme",
    repo: "acme/web",
    head_ref: "feature/checkout-steps",
    author_login: "ada",
    is_draft: false,
    state: "open",
    slack: null,
  } as unknown as ListRow;
  const api = {
    ...fakeApi(mockBranch({ state: "update_required", behind_by: 4 }), { prOverview: vi.fn(() => Promise.resolve(mockOverview({ merge_state: "behind" }))) }),
    prDetail: vi.fn((id: number) => Promise.resolve(mockDetail(id))),
    prSlackContext: vi.fn(() => Promise.resolve([])),
    loadPreferences: vi.fn(() => Promise.resolve({ min_bump_interval_hours: 0 })),
    prNote: vi.fn((prId: number) => Promise.resolve({ pr_id: prId, note: "", updated_at: null })),
    setPrStarred: vi.fn(() => Promise.resolve(true)),
  } as unknown as Api;
  render(<DetailPanel api={api} row={row} version={0} onClose={vi.fn()} onOpen={vi.fn()} onNotice={vi.fn()} />);
  const verdict = await screen.findByTestId("merge-verdict");
  await waitFor(() => expect(verdict.textContent).toContain("4 commits behind · update required."));
  const line = within(verdict).getByText("4 commits behind · update required.").closest("li") as HTMLElement;
  expect(within(line).getByRole("button", { name: "Update branch" })).toBeTruthy();
});
