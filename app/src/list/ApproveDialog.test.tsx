import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { Api, ApprovalPreview, ApproveReport } from "../api";
import { ApproveDialog, approveButtonLabel, approveDialogTitle, resultSummary } from "./ApproveDialog";

function preview(partial: Partial<ApprovalPreview> & Pick<ApprovalPreview, "id">): ApprovalPreview {
  return {
    repo: "acme/widgets",
    number: partial.id,
    title: `Bump lib ${partial.id}`,
    action: "approve",
    reason: null,
    message: null,
    head_sha: `sha-${partial.id}`,
    draft: false,
    approved_earlier: false,
    ...partial,
  };
}

function report(partial: Partial<ApproveReport> & Pick<ApproveReport, "id" | "outcome">): ApproveReport {
  return {
    repo: "acme/widgets",
    number: partial.id,
    title: `Bump lib ${partial.id}`,
    reason: null,
    message: null,
    draft: false,
    ...partial,
  };
}

const MIXED = [
  preview({ id: 1 }),
  preview({ id: 2, draft: true }),
  preview({ id: 3, action: "skip", reason: "own_pr", message: "Your own PR. GitHub doesn't let you approve it.", head_sha: "" }),
  preview({ id: 4, action: "skip", reason: "merged", message: "Already merged.", head_sha: "" }),
];

function dialog(api: Partial<Api>) {
  const onDismiss = vi.fn();
  const onApproved = vi.fn();
  const onNotice = vi.fn();
  render(
    <ApproveDialog
      api={api as Api}
      rows={[1, 2, 3, 4].map((id) => ({ id, repo: "acme/widgets", number: id, title: `Bump lib ${id}` }))}
      onDismiss={onDismiss}
      onApproved={onApproved}
      onNotice={onNotice}
    />,
  );
  return { onDismiss, onApproved, onNotice };
}

test("labels name one pull request or the count", () => {
  expect(approveDialogTitle(0)).toBe("Nothing to approve");
  expect(approveDialogTitle(1)).toBe("Approve 1 pull request?");
  expect(approveDialogTitle(3)).toBe("Approve 3 pull requests?");
  expect(approveButtonLabel(0)).toBe("Approve");
  expect(approveButtonLabel(1)).toBe("Approve PR");
  expect(approveButtonLabel(3)).toBe("Approve 3 PRs");
  expect(
    resultSummary([report({ id: 1, outcome: "approved" }), report({ id: 2, outcome: "failed" }), report({ id: 3, outcome: "approved" })]),
  ).toBe("Approved 2 · Failed 1");
});

test("the preview lists what will be approved and why the rest are skipped", async () => {
  const previewApprovals = vi.fn(() => Promise.resolve(MIXED));
  const approvePullRequests = vi.fn(() => Promise.resolve([]));
  dialog({ previewApprovals, approvePullRequests });
  expect(screen.getByRole("heading", { name: "Checking pull requests…" })).toBeTruthy();
  await screen.findByRole("heading", { name: "Approve 2 pull requests?" });
  expect(previewApprovals).toHaveBeenCalledWith([1, 2, 3, 4]);

  const will = screen.getByRole("region", { name: "Will approve" });
  expect(within(will).getByText("Draft")).toBeTruthy();
  expect(within(will).getAllByRole("listitem")).toHaveLength(2);
  const skipped = screen.getByRole("region", { name: "Skipped" });
  expect(within(skipped).getByText(/Your own PR/)).toBeTruthy();
  expect(within(skipped).getByText(/Already merged/)).toBeTruthy();
  expect(screen.getByText(/can't be merged until marked ready/)).toBeTruthy();
  expect(approvePullRequests).not.toHaveBeenCalled();
});

test("Enter never confirms; confirming sends the previewed heads and a trimmed comment", async () => {
  const previewApprovals = vi.fn(() => Promise.resolve([preview({ id: 1 }), preview({ id: 2 })]));
  const approvePullRequests = vi.fn(() =>
    Promise.resolve([report({ id: 1, outcome: "approved" }), report({ id: 2, outcome: "approved" })]),
  );
  const { onApproved, onDismiss, onNotice } = dialog({ previewApprovals, approvePullRequests });
  const button = await screen.findByRole("button", { name: "Approve 2 PRs" });
  const box = screen.getByRole("textbox", { name: "Comment (optional)" });
  expect((box as HTMLTextAreaElement).value).toBe("");
  fireEvent.keyDown(box, { key: "Enter" });
  fireEvent.keyDown(button, { key: "Enter" });
  expect(approvePullRequests).not.toHaveBeenCalled();

  fireEvent.change(box, { target: { value: "  Looks good \n" } });
  fireEvent.click(button);
  await waitFor(() =>
    expect(approvePullRequests).toHaveBeenCalledWith(
      [
        { id: 1, head_sha: "sha-1" },
        { id: 2, head_sha: "sha-2" },
      ],
      "Looks good",
    ),
  );
  expect(onApproved).toHaveBeenCalledWith([1, 2]);
  expect(onNotice).toHaveBeenCalledWith("Approved 2 pull requests.");
  expect(onDismiss).toHaveBeenCalled();
});

test("a partial failure shows one result per pull request and can check the failed ones again", async () => {
  const previewApprovals = vi
    .fn()
    .mockResolvedValueOnce(MIXED)
    .mockResolvedValueOnce([preview({ id: 2, head_sha: "sha-2b" })]);
  const approvePullRequests = vi.fn(() =>
    Promise.resolve([
      report({ id: 1, outcome: "approved" }),
      report({ id: 2, outcome: "failed", message: "New commits were pushed after you checked. Review them, then approve again." }),
    ]),
  );
  const { onApproved, onDismiss } = dialog({ previewApprovals, approvePullRequests });
  fireEvent.click(await screen.findByRole("button", { name: "Approve 2 PRs" }));
  await screen.findByRole("heading", { name: "Approval results" });
  expect(onApproved).toHaveBeenCalledWith([1]);
  expect(onDismiss).not.toHaveBeenCalled();
  expect(screen.getByText("Approved 1 · Skipped 2 · Failed 1")).toBeTruthy();
  const results = within(screen.getByRole("list", { name: "Results" })).getAllByRole("listitem");
  expect(results).toHaveLength(4);
  expect(within(results[1]).getByText("Failed")).toBeTruthy();
  expect(within(results[1]).getByText(/New commits were pushed/)).toBeTruthy();

  fireEvent.click(screen.getByRole("button", { name: "Check failed again" }));
  await screen.findByRole("heading", { name: "Approve 1 pull request?" });
  expect(previewApprovals).toHaveBeenLastCalledWith([2]);
});

test("a preview error is shown and nothing can be approved", async () => {
  const previewApprovals = vi.fn(() => Promise.reject(new Error("GitHub refused the approval. Your token needs write access")));
  dialog({ previewApprovals, approvePullRequests: vi.fn() });
  expect((await screen.findByRole("alert")).textContent).toContain("token needs write access");
  expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
});
