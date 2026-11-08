import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { Api, CloseReport } from "../api";
import { CloseDialog, closeButtonLabel, closeDialogTitle } from "./CloseDialog";

function report(partial: Partial<CloseReport> & Pick<CloseReport, "id" | "outcome">): CloseReport {
  return {
    repo: "acme/widgets",
    number: partial.id,
    title: `Change ${partial.id}`,
    comment_posted: false,
    message: null,
    ...partial,
  };
}

function dialog(closePullRequests: Api["closePullRequests"]) {
  const onDismiss = vi.fn();
  const onClosed = vi.fn();
  const onNotice = vi.fn();
  render(
    <CloseDialog
      api={{ closePullRequests } as unknown as Api}
      ids={[7, 8]}
      known={[
        { id: 7, repo: "acme/widgets", number: 7, title: "Tighten the latch" },
        { id: 8, repo: "acme/widgets", number: 8, title: "Replace the hinge" },
      ]}
      onDismiss={onDismiss}
      onClosed={onClosed}
      onNotice={onNotice}
    />,
  );
  return { onDismiss, onClosed, onNotice };
}

test("the labels name one pull request or the count", () => {
  expect(closeDialogTitle(1)).toBe("Close this pull request?");
  expect(closeDialogTitle(3)).toBe("Close 3 pull requests?");
  expect(closeButtonLabel(1)).toBe("Close PR");
  expect(closeButtonLabel(3)).toBe("Close 3 PRs");
});

test("Enter never closes, and an empty or whitespace comment is sent blank", async () => {
  const closePullRequests = vi.fn(() =>
    Promise.resolve([report({ id: 7, outcome: "closed" }), report({ id: 8, outcome: "closed" })]),
  );
  const { onClosed, onDismiss, onNotice } = dialog(closePullRequests);
  const box = screen.getByRole("textbox", { name: "Comment (optional)" });
  expect(box).toHaveFocus();
  fireEvent.keyDown(box, { key: "Enter" });
  fireEvent.keyDown(screen.getByRole("button", { name: "Close 2 PRs" }), { key: "Enter" });
  expect(closePullRequests).not.toHaveBeenCalled();

  fireEvent.change(box, { target: { value: " \n\t " } });
  fireEvent.click(screen.getByRole("button", { name: "Close 2 PRs" }));
  await waitFor(() => expect(closePullRequests).toHaveBeenCalledWith([7, 8], ""));
  expect(onClosed).toHaveBeenCalledWith([7, 8]);
  expect(onNotice).toHaveBeenCalledWith("Closed 2 pull requests.");
  expect(onDismiss).toHaveBeenCalled();
});

test("a comment is sent as typed, and a failed pull request stays for retry", async () => {
  const closePullRequests = vi
    .fn()
    .mockResolvedValueOnce([
      report({ id: 7, outcome: "closed", comment_posted: true }),
      report({
        id: 8,
        outcome: "failed",
        comment_posted: true,
        message: "Comment posted, but the pull request was not closed: You don't have permission to close this pull request.",
      }),
    ])
    .mockResolvedValueOnce([report({ id: 8, outcome: "closed", comment_posted: true })]);
  const { onClosed, onDismiss } = dialog(closePullRequests);
  fireEvent.change(screen.getByRole("textbox", { name: "Comment (optional)" }), {
    target: { value: "No longer needed" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Close 2 PRs" }));
  await screen.findByRole("alert");
  expect(onClosed).toHaveBeenCalledWith([7]);
  expect(onDismiss).not.toHaveBeenCalled();
  expect(screen.getByRole("alert").textContent).toContain(
    "Comment posted, but the pull request was not closed",
  );
  expect(screen.getByRole("heading", { name: "Close this pull request?" })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Close PR" }));
  await waitFor(() => expect(closePullRequests).toHaveBeenLastCalledWith([8], "No longer needed"));
  expect(onClosed).toHaveBeenLastCalledWith([8]);
});

test("an already closed pull request is skipped and leaves the list", async () => {
  const closePullRequests = vi.fn(() =>
    Promise.resolve([
      report({ id: 7, outcome: "skipped", message: "Already closed." }),
      report({ id: 8, outcome: "skipped", message: "Already merged." }),
    ]),
  );
  const { onClosed, onNotice } = dialog(closePullRequests);
  fireEvent.click(screen.getByRole("button", { name: "Close 2 PRs" }));
  await waitFor(() => expect(onClosed).toHaveBeenCalledWith([7, 8]));
  expect(onNotice).toHaveBeenCalledWith("2 already closed or merged.");
});
