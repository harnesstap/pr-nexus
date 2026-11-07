import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { Toast } from "../list/model";
import type { Api, BulkCard, BulkPreview, BulkRequest, BulkResult } from "../api";
import { BulkDialog } from "./BulkDialog";

const pr = (pr_id: number, number: number, title: string) => ({ pr_id, number, title, repo: "acme/widgets" });
const platform = { workspace: "acme", channel_id: "C0123456789", channel_name: "platform" };

function card(single: boolean): BulkCard {
  return single
    ? {
        target: platform,
        messages: [{ action: "request", prs: [pr(1, 42, "Add passkeys"), pr(2, 43, "Fix login")], text: "Two PRs please", permalink: null }],
        warnings: [],
        signed_in: true,
        single_top_level: true,
        can_switch: true,
        fingerprint: "fp-single",
      }
    : {
        target: platform,
        messages: [
          { action: "bump", prs: [pr(1, 42, "Add passkeys")], text: "Bump #42", permalink: "https://acme.slack.com/archives/C0123456789/p1" },
          { action: "request", prs: [pr(2, 43, "Fix login")], text: "Please review #43", permalink: null },
        ],
        warnings: ["#42 was bumped 2h ago."],
        signed_in: true,
        single_top_level: false,
        can_switch: true,
        fingerprint: "fp-split",
      };
}

function fake() {
  const calls = { previews: [] as BulkRequest[], sends: [] as [BulkRequest, string[]][] };
  const api = {
    slackWorkspaces: () => Promise.resolve([{ id: 3, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false, team_id: "T0ACME0001", enabled: true, priority: 0 }]),
    bulkPreview: (request: BulkRequest): Promise<BulkPreview> => {
      calls.previews.push(request);
      if (request.target) {
        return Promise.resolve({
          cards: [{ ...card(true), target: request.target, fingerprint: "fp-dm", can_switch: false, single_top_level: false }],
          skipped: [],
          pr_warnings: [],
        });
      }
      return Promise.resolve({
        cards: [card(request.single_top_level.length > 0)],
        skipped: request.pr_ids.includes(3) ? [{ pr: pr(3, 44, "Draft work"), detail: "Not open" }] : [],
        pr_warnings: [],
      });
    },
    sendBulk: (request: BulkRequest, fingerprints: string[]): Promise<BulkResult> => {
      calls.sends.push([request, fingerprints]);
      return Promise.resolve({
        batch_id: "bulk-1",
        results: [
          { pr: pr(3, 44, "Draft work"), target: null, outcome: "skipped", detail: "Not open", permalink: null },
          { pr: pr(1, 42, "Add passkeys"), target: platform, outcome: "sent", detail: "Bumped in the thread", permalink: "https://acme.slack.com/archives/C0123456789/p1" },
          { pr: pr(2, 43, "Fix login"), target: platform, outcome: "failed", detail: "Slack is rate limiting this channel", permalink: null },
        ],
        indicators: [{ pr_id: 1, indicator: null }],
      });
    },
  } as unknown as Api;
  return { api, calls };
}

test("previews every message per channel, switches to one top-level request, and reports per-PR results", async () => {
  const { api, calls } = fake();
  const sent: BulkResult[] = [];
  render(<BulkDialog api={api} prIds={[1, 2, 3]} onClose={() => undefined} onSent={(items) => sent.push(items)} onOpen={() => undefined} />);
  expect(await screen.findByText("Bump #42")).toBeInTheDocument();
  expect(screen.getByText("Please review #43")).toBeInTheDocument();
  expect(screen.getByText("#42 was bumped 2h ago.")).toBeInTheDocument();
  expect(screen.getByText(/Not open/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Send 2 messages" })).toBeEnabled();
  // The card names the channel; its raw id is only in the name's tooltip.
  expect(screen.queryByText("C0123456789")).not.toBeInTheDocument();

  fireEvent.click(screen.getByRole("checkbox", { name: "One top-level request for all" }));
  expect(await screen.findByText("Two PRs please")).toBeInTheDocument();
  expect(calls.previews.at(-1)?.single_top_level).toEqual([{ workspace: "acme", channel_id: "C0123456789" }]);

  fireEvent.click(screen.getByRole("button", { name: "Send 1 message" }));
  expect(await screen.findByText("Bumped in the thread")).toBeInTheDocument();
  expect(calls.sends[0]?.[1]).toEqual(["fp-single"]);
  expect(sent).toHaveLength(1);
  expect(screen.getByText("Slack is rate limiting this channel")).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "Retry failed" }));
  await waitFor(() => expect(calls.previews.at(-1)?.pr_ids).toEqual([2]));
});

test("all PRs can go to one typed DM channel", async () => {
  const { api, calls } = fake();
  render(<BulkDialog api={api} prIds={[1, 2]} onClose={() => undefined} onSent={() => undefined} onOpen={() => undefined} />);
  await screen.findByText("Bump #42");
  fireEvent.click(screen.getByRole("radio", { name: "All to one channel" }));
  fireEvent.click(screen.getByRole("button", { name: "Enter an ID" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Channel, DM or member ID" }), { target: { value: "D0123456789" } });
  fireEvent.change(screen.getByRole("textbox", { name: "Name" }), { target: { value: "me" } });
  fireEvent.click(screen.getByRole("button", { name: "Preview" }));
  expect(await screen.findByRole("region", { name: "DM · me" })).toBeInTheDocument();
  expect(calls.previews.at(-1)?.target).toEqual({ workspace: "", channel_id: "D0123456789", channel_name: "me" });
  fireEvent.click(screen.getByRole("button", { name: "Send 1 message" }));
  await waitFor(() => expect(calls.sends[0]?.[1]).toEqual(["fp-dm"]));
});

test("Escape closes the dialog", async () => {
  const { api } = fake();
  let closed = false;
  render(<BulkDialog api={api} prIds={[1, 2]} onClose={() => (closed = true)} onSent={() => undefined} onOpen={() => undefined} />);
  await screen.findByText("Bump #42");
  fireEvent.keyDown(document, { key: "Escape" });
  expect(closed).toBe(true);
});

test("titles with underscores preview as inline code with no Markdown warning", async () => {
  const { api } = fake();
  const text = "Hello team\nCould someone please review these pull requests?\n• `feat: return location_ip_hash`\nhttps://github.com/acme/widgets/pull/42\n• `fix: keep snake_case keys`\nhttps://github.com/acme/widgets/pull/43\nThanks!\n";
  const preview: BulkPreview = {
    cards: [{ ...card(true), messages: [{ action: "request", prs: [pr(1, 42, "feat: return location_ip_hash"), pr(2, 43, "fix: keep snake_case keys")], text, permalink: null }] }],
    skipped: [],
    pr_warnings: [],
  };
  render(<BulkDialog api={{ ...api, bulkPreview: () => Promise.resolve(preview) }} prIds={[1, 2]} onClose={() => undefined} onSent={() => undefined} onOpen={() => undefined} />);
  expect(await screen.findByLabelText("Message preview")).toHaveTextContent("• `feat: return location_ip_hash`");
  expect(screen.queryByText(/Markdown characters/)).not.toBeInTheDocument();
});

const draft = (pr_id: number, number: number) => ({ kind: "draft" as const, pr_id, number, message: `#${number} is still a draft.` });

test("drafts warn once with a count and a badge each, and can be sent anyway", async () => {
  const { api } = fake();
  const withDrafts: Api = {
    ...api,
    bulkPreview: (request: BulkRequest) => api.bulkPreview(request).then((planned) => ({ ...planned, pr_warnings: [draft(1, 42)] })),
  };
  render(<BulkDialog api={withDrafts} prIds={[1, 2]} onClose={() => undefined} onSent={() => undefined} onOpen={() => undefined} />);
  await screen.findByText("Bump #42");
  expect(screen.getByRole("note", { name: "Draft warning" })).toHaveTextContent("1 of 2 PRs is still a draft: #42.");
  expect(screen.getAllByTestId("draft-badge")).toHaveLength(1);
  expect(screen.getByRole("button", { name: "Send 2 messages anyway" })).toBeEnabled();
});

test("no drafts, no warning", async () => {
  const { api } = fake();
  render(<BulkDialog api={api} prIds={[1, 2]} onClose={() => undefined} onSent={() => undefined} onOpen={() => undefined} />);
  await screen.findByText("Bump #42");
  expect(screen.queryByRole("note", { name: "Draft warning" })).not.toBeInTheDocument();
  expect(screen.queryByTestId("draft-badge")).not.toBeInTheDocument();
});

test("Mark all ready takes every draft out of draft and clears the warning", async () => {
  const { api } = fake();
  const flipped: [number, boolean][] = [];
  const ready: number[] = [];
  const withDrafts: Api = {
    ...api,
    bulkPreview: (request: BulkRequest) => api.bulkPreview(request).then((planned) => ({ ...planned, pr_warnings: [draft(1, 42), draft(2, 43)] })),
    setPrDraft: (id: number, value: boolean) => {
      flipped.push([id, value]);
      return Promise.resolve({} as Awaited<ReturnType<Api["setPrDraft"]>>);
    },
  };
  render(<BulkDialog api={withDrafts} prIds={[1, 2]} onClose={() => undefined} onSent={() => undefined} onOpen={() => undefined} onReady={(id) => ready.push(id)} />);
  await screen.findByText("Bump #42");
  expect(screen.getByRole("note", { name: "Draft warning" })).toHaveTextContent("2 of 2 PRs are still drafts: #42, #43.");
  fireEvent.click(screen.getByRole("button", { name: "Mark all ready" }));
  await waitFor(() => expect(screen.queryByRole("note", { name: "Draft warning" })).not.toBeInTheDocument());
  expect(flipped).toEqual([[1, false], [2, false]]);
  expect(ready).toEqual([1, 2]);
  expect(screen.queryByTestId("draft-badge")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Send 2 messages" })).toBeEnabled();
});

test("Mark all ready runs up to four at once, clears each as it finishes and reports failures", async () => {
  const { api } = fake();
  const ids = [1, 2, 3, 4, 5, 6];
  const pending = new Map<number, { resolve: () => void; reject: (err: Error) => void }>();
  const ready: number[] = [];
  const withDrafts: Api = {
    ...api,
    bulkPreview: (request: BulkRequest) => api.bulkPreview(request).then((planned) => ({ ...planned, pr_warnings: ids.map((id) => draft(id, 40 + id)) })),
    setPrDraft: (id: number) =>
      new Promise<Awaited<ReturnType<Api["setPrDraft"]>>>((resolve, reject) => {
        pending.set(id, { resolve: () => resolve({} as Awaited<ReturnType<Api["setPrDraft"]>>), reject });
      }),
  };
  render(<BulkDialog api={withDrafts} prIds={[1, 2]} onClose={() => undefined} onSent={() => undefined} onOpen={() => undefined} onReady={(id) => ready.push(id)} />);
  await screen.findByText("Bump #42");
  fireEvent.click(screen.getByRole("button", { name: "Mark all ready" }));
  await waitFor(() => expect([...pending.keys()]).toEqual([1, 2, 3, 4]));
  pending.get(2)?.resolve();
  await waitFor(() => expect(ready).toEqual([2]));
  await waitFor(() => expect([...pending.keys()]).toEqual([1, 2, 3, 4, 5]));
  pending.get(1)?.reject(new Error("GitHub said no"));
  await waitFor(() => expect(pending.has(6)).toBe(true));
  for (const id of [3, 4, 5, 6]) {
    pending.get(id)?.resolve();
  }
  await waitFor(() => expect(screen.getByText("Couldn't mark ready for review: #41: GitHub said no")).toBeInTheDocument());
  expect(ready.sort()).toEqual([2, 3, 4, 5, 6]);
});

function planned(text: string, included: ReturnType<typeof pr>[], skipped: { pr: ReturnType<typeof pr>; detail: string }[], warnings: ReturnType<typeof draft>[]): BulkPreview {
  return {
    cards: [
      {
        target: platform,
        messages: [{ action: "request", prs: included, text, permalink: null }],
        warnings: [],
        signed_in: true,
        single_top_level: false,
        can_switch: false,
        fingerprint: "fp-ready",
      },
    ],
    skipped,
    pr_warnings: warnings,
  };
}

test("Mark all ready replans and a draft whose checks now pass joins the message", async () => {
  const { api } = fake();
  let calls = 0;
  const withDrafts: Api = {
    ...api,
    bulkPreview: () => {
      calls += 1;
      const first = calls === 1;
      return Promise.resolve(
        planned(
          first ? "Only the ready one" : "Both PRs please",
          first ? [pr(1, 42, "Add passkeys")] : [pr(1, 42, "Add passkeys"), pr(2, 43, "Fix login")],
          first ? [{ pr: pr(2, 43, "Fix login"), detail: "Blocked: check ci is fail" }] : [],
          first ? [draft(2, 43)] : [],
        ),
      );
    },
    setPrDraft: () => Promise.resolve({} as Awaited<ReturnType<Api["setPrDraft"]>>),
  };
  render(<BulkDialog api={withDrafts} prIds={[1, 2]} onClose={() => undefined} onSent={() => undefined} onOpen={() => undefined} />);
  expect(await screen.findByText("Only the ready one")).toBeInTheDocument();
  expect(screen.getByText(/Blocked: check ci is fail/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Mark ready for review" }));
  expect(await screen.findByText("Both PRs please")).toBeInTheDocument();
  expect(screen.queryByText(/check ci is fail/)).not.toBeInTheDocument();
  expect(screen.queryByRole("note", { name: "Draft warning" })).not.toBeInTheDocument();
  expect(calls).toBe(2);
});

test("a partial mark-ready keeps the failed draft and a real CI block", async () => {
  const { api } = fake();
  let calls = 0;
  const withDrafts: Api = {
    ...api,
    bulkPreview: () => {
      calls += 1;
      const first = calls === 1;
      return Promise.resolve(
        planned(
          first ? "One PR" : "Ready ones",
          first ? [pr(1, 42, "Add passkeys")] : [pr(1, 42, "Add passkeys"), pr(2, 43, "Now ready")],
          [{ pr: pr(3, 44, "Still red"), detail: "Blocked: check ci is fail" }],
          first ? [draft(2, 43), draft(3, 44)] : [draft(3, 44)],
        ),
      );
    },
    setPrDraft: (id: number) =>
      id === 3
        ? Promise.reject(new Error("GitHub said no"))
        : Promise.resolve({} as Awaited<ReturnType<Api["setPrDraft"]>>),
  };
  render(<BulkDialog api={withDrafts} prIds={[1, 2, 3]} onClose={() => undefined} onSent={() => undefined} onOpen={() => undefined} />);
  expect(await screen.findByText("One PR")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Mark all ready" }));
  expect(await screen.findByText("Ready ones")).toBeInTheDocument();
  expect(screen.getByText("Couldn't mark ready for review: #44: GitHub said no")).toBeInTheDocument();
  expect(screen.getByRole("note", { name: "Draft warning" })).toHaveTextContent("#44");
  expect(screen.getByText(/Blocked: check ci is fail/)).toBeInTheDocument();
  expect(calls).toBe(2);
});

test("marking ready does not drop a PR whose checks are still failing", async () => {
  const { api } = fake();
  let calls = 0;
  const withDrafts: Api = {
    ...api,
    bulkPreview: () => {
      calls += 1;
      return Promise.resolve(
        planned(
          "Only green",
          [pr(1, 42, "Add passkeys")],
          [{ pr: pr(2, 43, "Red draft"), detail: "Blocked: check ci is fail" }],
          calls === 1 ? [draft(2, 43)] : [],
        ),
      );
    },
    setPrDraft: () => Promise.resolve({} as Awaited<ReturnType<Api["setPrDraft"]>>),
  };
  render(<BulkDialog api={withDrafts} prIds={[1, 2]} onClose={() => undefined} onSent={() => undefined} onOpen={() => undefined} />);
  expect(await screen.findByText("Only green")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Mark ready for review" }));
  await waitFor(() => expect(screen.queryByRole("note", { name: "Draft warning" })).not.toBeInTheDocument());
  expect(screen.getByText("Only green")).toBeInTheDocument();
  expect(screen.getByText(/Blocked: check ci is fail/)).toBeInTheDocument();
  expect(calls).toBe(2);
});

test("Copy message copies each destination's text without sending, in both modes", async () => {
  const writeText = vi.fn<(text: string) => Promise<void>>(() => Promise.resolve());
  Object.assign(navigator, { clipboard: { writeText } });
  const { api, calls } = fake();
  const notices: Toast[] = [];
  let closed = false;
  render(<BulkDialog api={api} prIds={[1, 2]} onClose={() => (closed = true)} onSent={() => undefined} onOpen={() => undefined} onNotice={(toast) => notices.push(toast)} />);
  await screen.findByText("Bump #42");
  const buttons = screen.getAllByRole("button", { name: "Copy message" });
  expect(buttons).toHaveLength(2);
  fireEvent.click(buttons[1]!);
  await waitFor(() => expect(writeText).toHaveBeenLastCalledWith("Please review #43"));
  expect(notices.at(-1)).toEqual({ text: "Message copied" });
  fireEvent.click(buttons[0]!);
  await waitFor(() => expect(writeText).toHaveBeenLastCalledWith("Bump #42"));

  fireEvent.click(screen.getByRole("radio", { name: "All to one channel" }));
  fireEvent.click(screen.getByRole("button", { name: "Enter an ID" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Channel, DM or member ID" }), { target: { value: "D0123456789" } });
  fireEvent.change(screen.getByRole("textbox", { name: "Name" }), { target: { value: "me" } });
  fireEvent.click(screen.getByRole("button", { name: "Preview" }));
  await screen.findByRole("region", { name: "DM · me" });
  fireEvent.click(screen.getByRole("button", { name: "Copy message" }));
  await waitFor(() => expect(writeText).toHaveBeenLastCalledWith("Two PRs please"));

  expect(calls.sends).toHaveLength(0);
  expect(closed).toBe(false);
  expect(screen.getByRole("dialog")).toBeInTheDocument();
});

test("Copy message shows an error toast when the clipboard fails", async () => {
  Object.assign(navigator, { clipboard: { writeText: vi.fn(() => Promise.reject(new Error("denied"))) } });
  Object.assign(document, { execCommand: vi.fn(() => false) });
  const { api, calls } = fake();
  const notices: Toast[] = [];
  render(<BulkDialog api={api} prIds={[1, 2]} onClose={() => undefined} onSent={() => undefined} onOpen={() => undefined} onNotice={(toast) => notices.push(toast)} />);
  await screen.findByText("Bump #42");
  fireEvent.click(screen.getAllByRole("button", { name: "Copy message" })[0]!);
  await waitFor(() => expect(notices.at(-1)).toEqual({ text: "Couldn't copy to the clipboard: Clipboard is not available", error: true }));
  expect(calls.sends).toHaveLength(0);
});

test("draws the cards from cached names at once, then swaps in Slack's names before Send", async () => {
  const { api } = fake();
  let finishNames: (planned: BulkPreview) => void = () => undefined;
  const requests: BulkRequest[] = [];
  const slow: Api = {
    ...api,
    bulkPreview: (request: BulkRequest) => {
      requests.push(request);
      if (request.cached_names) {
        return Promise.resolve({ cards: [{ ...card(false), target: { ...platform, channel_name: "old-name" } }], skipped: [], pr_warnings: [], names_pending: true });
      }
      return new Promise<BulkPreview>((resolve) => {
        finishNames = resolve;
      });
    },
  };
  render(<BulkDialog api={slow} prIds={[1, 2]} onClose={() => undefined} onSent={() => undefined} onOpen={() => undefined} />);
  // Cards are on screen while Slack is still being asked for the name.
  expect(await screen.findByText("Bump #42")).toBeInTheDocument();
  expect(screen.getByText(/#old-name/)).toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent("Checking channel names on Slack…");
  expect(screen.getByRole("button", { name: "Send 2 messages" })).toBeDisabled();
  expect(requests.map((r) => r.cached_names)).toEqual([true, false]);
  finishNames({ cards: [card(false)], skipped: [], pr_warnings: [] });
  expect(await screen.findByText(/#platform/)).toBeInTheDocument();
  expect(screen.queryByText(/#old-name/)).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Send 2 messages" })).toBeEnabled();
});

test("skips the Slack round when every name is already cached", async () => {
  const { api, calls } = fake();
  render(<BulkDialog api={api} prIds={[1, 2]} onClose={() => undefined} onSent={() => undefined} onOpen={() => undefined} />);
  expect(await screen.findByText("Bump #42")).toBeInTheDocument();
  expect(calls.previews).toHaveLength(1);
  expect(calls.previews[0].cached_names).toBe(true);
});
