import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { Api, BulkPreview, BulkRequest, DueBump } from "../api";
import { BumpQueue } from "./BumpQueue";

function due(id: number, number: number, title: string): DueBump {
  return {
    pr_id: id,
    number,
    title,
    repo: "acme/widgets",
    asked_at: "2026-11-06T16:00:00Z",
    quiet_hours: 50,
    bump_count: 1,
  };
}

function preview(channelId: string, name: string, text: string): BulkPreview {
  return {
    cards: [
      {
        target: { workspace: "acme", workspace_name: "Acme", channel_id: channelId, channel_name: name },
        messages: [
          {
            action: "bump",
            prs: [{ pr_id: 1, number: 142, title: "Migrate login", repo: "acme/widgets" }],
            text,
            permalink: "https://acme.slack.com/archives/C0ACME0001/p1700000000000100",
          },
        ],
        warnings: [],
        signed_in: true,
        single_top_level: false,
        can_switch: false,
        fingerprint: `fp-${channelId}-${text}`,
        notes: [],
      },
    ],
    skipped: [],
    pr_warnings: [{ kind: "checks", pr_id: 1, number: 142, message: "#142 has failing checks." }],
  };
}

const shown = preview("C0ACME0001", "team-reviews", "Please take another look\n• `Migrate login`");

function harness(bulkPreview: Api["bulkPreview"], extra: Partial<Api> = {}) {
  const sendBulk = vi.fn(() => Promise.resolve({ batch_id: "bulk-1", results: [], indicators: [] }));
  const snoozeBump = vi.fn(() => Promise.resolve([]));
  const api = {
    dueBumps: () => Promise.resolve([due(1, 142, "Migrate login")]),
    bulkPreview,
    sendBulk,
    snoozeBump,
    skipBump: vi.fn(() => Promise.resolve([])),
    ...extra,
  } as unknown as Api;
  return { api, sendBulk, snoozeBump };
}

test("shows the preview channel and does not send until Send is clicked", async () => {
  const { api, sendBulk } = harness((request) => Promise.resolve(request.cached_names === false ? shown : shown));
  render(<BumpQueue api={api} onOpen={() => undefined} />);
  expect(await screen.findByRole("region", { name: "#team-reviews" })).toHaveAttribute("data-channel-id", "C0ACME0001");
  expect(screen.getByText(/Please take another look/)).toBeInTheDocument();
  expect(screen.getByRole("note", { name: "Failing checks" })).toHaveTextContent("#142 has failing checks.");
  expect(sendBulk).not.toHaveBeenCalled();

  fireEvent.click(screen.getByRole("button", { name: "Send all" }));
  await waitFor(() => expect(sendBulk).toHaveBeenCalledTimes(1));
  const [body, fingerprints] = sendBulk.mock.calls[0] as unknown as [BulkRequest, string[]];
  expect(body.pr_ids).toEqual([1]);
  expect(fingerprints).toEqual([shown.cards[0]!.fingerprint]);
});

test("a changed destination is shown and not sent", async () => {
  const moved = preview("C0ACME0002", "payments", "Please take another look\n• `Migrate login`");
  const { api, sendBulk } = harness((request) => Promise.resolve(request.cached_names === false ? moved : shown));
  render(<BumpQueue api={api} onOpen={() => undefined} />);
  expect(await screen.findByRole("region", { name: "#team-reviews" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Send all" }));
  expect(await screen.findByRole("region", { name: "#payments" })).toHaveAttribute("data-channel-id", "C0ACME0002");
  expect(screen.getByRole("status")).toHaveTextContent("The destination or the message changed");
  expect(sendBulk).not.toHaveBeenCalled();
});

test("snooze does not send", async () => {
  const { api, sendBulk, snoozeBump } = harness(() => Promise.resolve(shown));
  render(<BumpQueue api={api} onOpen={() => undefined} />);
  fireEvent.click(await screen.findByRole("button", { name: "Snooze" }));
  await waitFor(() => expect(snoozeBump).toHaveBeenCalledWith(1));
  expect(sendBulk).not.toHaveBeenCalled();
});

test("a note is part of the preview request and is not sent by typing it", async () => {
  const calls: BulkRequest[] = [];
  const { api, sendBulk } = harness((request) => {
    calls.push(request);
    const note = request.notes?.find((item) => item.pr_id === 1)?.text;
    const text = note ? `${shown.cards[0]!.messages[0]!.text}\n\n${note}\n` : shown.cards[0]!.messages[0]!.text;
    return Promise.resolve(preview("C0ACME0001", "team-reviews", text));
  });
  render(<BumpQueue api={api} onOpen={() => undefined} />);
  const note = await screen.findByRole("textbox", { name: "Note for acme/widgets#142" });
  fireEvent.change(note, { target: { value: "Start with the login flow" } });
  await waitFor(() => expect(calls.some((request) => request.notes?.[0]?.text === "Start with the login flow")).toBe(true));
  expect(await screen.findByLabelText("Message preview")).toHaveTextContent("Start with the login flow");
  expect(sendBulk).not.toHaveBeenCalled();
});
