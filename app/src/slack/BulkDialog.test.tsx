import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, test } from "vitest";
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
    slackWorkspaces: () => Promise.resolve([{ id: 3, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false }]),
    bulkPreview: (request: BulkRequest): Promise<BulkPreview> => {
      calls.previews.push(request);
      if (request.target) {
        return Promise.resolve({
          cards: [{ ...card(true), target: request.target, fingerprint: "fp-dm", can_switch: false, single_top_level: false }],
          skipped: [],
        });
      }
      return Promise.resolve({
        cards: [card(request.single_top_level.length > 0)],
        skipped: request.pr_ids.includes(3) ? [{ pr: pr(3, 44, "Draft work"), detail: "Not open" }] : [],
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
  expect(await screen.findByRole("region", { name: "acme DM · me" })).toBeInTheDocument();
  expect(calls.previews.at(-1)?.target).toEqual({ workspace: "acme", channel_id: "D0123456789", channel_name: "me" });
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
