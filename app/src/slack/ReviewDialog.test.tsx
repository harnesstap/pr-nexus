import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, test } from "vitest";
import type { Api, ListRow, PrDetail, ReviewPreview, ReviewTarget, SlackIndicator, TargetChoice } from "../api";
import { ReviewDialog } from "./ReviewDialog";

const row = {
  id: 7,
  number: 42,
  title: "Add passkeys",
  url: "https://github.com/acme/widgets/pull/42",
  repo: "acme/widgets",
  slack: null,
} as ListRow;

const detail = { mandatory_teams: ["@acme/platform"], candidate_teams: [] } as unknown as PrDetail;
const mapped: TargetChoice = {
  target: { workspace: "acme", channel_id: "C0123456789", channel_name: "platform" },
  teams: ["@acme/platform"],
  source: "mapping",
};

function fake(overrides: Partial<Api> = {}) {
  const calls = { targets: [] as string[][], sent: [] as [ReviewTarget, string[]][] };
  const api = {
    prDetail: () => Promise.resolve(detail),
    slackWorkspaces: () => Promise.resolve([{ id: 3, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false }]),
    reviewTargets: (_id: number, teams: string[]) => {
      calls.targets.push(teams);
      return Promise.resolve([mapped]);
    },
    reviewPreview: (_id: number, target: ReviewTarget): Promise<ReviewPreview> =>
      Promise.resolve({
        target,
        action: "request",
        text: `Review please · #${target.channel_name}`,
        warnings: [],
        signed_in: true,
        bump_count: 0,
        permalink: null,
        broadcast: true,
      }),
    sendReview: (_id: number, target: ReviewTarget, teams: string[]): Promise<SlackIndicator> => {
      calls.sent.push([target, teams]);
      return Promise.resolve({
        requested_at: "2026-10-26T12:00:00Z",
        last_bumped_at: null,
        bump_count: 0,
        workspace: target.workspace,
        channel_id: target.channel_id,
        channel_name: target.channel_name,
        permalink: "https://acme.slack.com/archives/C0123456789/p1",
        root_ts: "1.0",
        source: "app",
      });
    },
    ...overrides,
  } as unknown as Api;
  return { api, calls };
}

test("the mapped channel is previewed for the mandatory teams and Send posts it", async () => {
  const { api, calls } = fake();
  const sent: SlackIndicator[] = [];
  render(<ReviewDialog api={api} row={row} onClose={() => undefined} onOpen={() => undefined} onSent={(item) => sent.push(item)} />);
  expect(await screen.findByLabelText("Message preview")).toHaveTextContent("Review please · #platform");
  expect(calls.targets).toEqual([["@acme/platform"]]);
  fireEvent.click(screen.getByRole("button", { name: "Send request" }));
  await waitFor(() => expect(sent).toHaveLength(1));
  expect(calls.sent).toEqual([[mapped.target, ["@acme/platform"]]]);
});

test("a bump shows the thread link and a signed-out workspace blocks Send", async () => {
  const opened: string[] = [];
  const { api } = fake({
    reviewPreview: (_id, target) =>
      Promise.resolve({
        target,
        action: "bump",
        text: "Friendly bump",
        warnings: ["Last bump was 2 h ago"],
        signed_in: false,
        bump_count: 1,
        permalink: "https://acme.slack.com/archives/C0123456789/p2",
        broadcast: true,
      }),
  });
  render(<ReviewDialog api={api} row={row} onClose={() => undefined} onOpen={(url) => opened.push(url)} onSent={() => undefined} />);
  expect(await screen.findByText(/thread, also sent to the channel \(1 earlier bump\)/)).toBeInTheDocument();
  expect(screen.getByText("Last bump was 2 h ago")).toBeInTheDocument();
  expect(screen.getByText(/Sign in to acme in Settings/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Bump in thread" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Open thread ↗" }));
  expect(opened).toEqual(["https://acme.slack.com/archives/C0123456789/p2"]);
});

test("without a mapping the channel is entered by hand and errors are shown", async () => {
  const { api } = fake({
    reviewTargets: () => Promise.resolve([]),
    reviewPreview: (_id, target) =>
      target.channel_id.length < 9
        ? Promise.reject({ kind: "validation", message: "Enter a Slack channel ID such as C0123456789" })
        : Promise.resolve({ target, action: "request", text: "hand", warnings: [], signed_in: true, bump_count: 0, permalink: null, broadcast: true }),
  });
  render(<ReviewDialog api={api} row={row} onClose={() => undefined} onOpen={() => undefined} onSent={() => undefined} />);
  expect(await screen.findByText(/No mapped channel/)).toBeInTheDocument();
  fireEvent.change(screen.getByRole("textbox", { name: "Channel or DM ID" }), { target: { value: "C1" } });
  fireEvent.click(screen.getByRole("button", { name: "Preview" }));
  expect(await screen.findByText(/such as C0123456789/)).toBeInTheDocument();
  fireEvent.change(screen.getByRole("textbox", { name: "Channel or DM ID" }), { target: { value: "C0123456789" } });
  fireEvent.click(screen.getByRole("button", { name: "Preview" }));
  expect(await screen.findByLabelText("Message preview")).toHaveTextContent("hand");
  expect(screen.getByRole("button", { name: "Send request" })).toBeEnabled();
});

test("a send whose link is unknown is a notice, not an error, and Send stays off", async () => {
  const notice = "Sent to Slack, but the link is unknown. Check the channel before sending again.";
  const { api } = fake({ sendReview: () => Promise.reject({ kind: "sent_unconfirmed", message: notice }) });
  render(<ReviewDialog api={api} row={row} onClose={() => undefined} onOpen={() => undefined} onSent={() => undefined} />);
  await screen.findByLabelText("Message preview");
  fireEvent.click(screen.getByRole("button", { name: "Send request" }));
  expect(await screen.findByRole("status")).toHaveTextContent(notice);
  expect(screen.getByRole("button", { name: "Send request" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Close" })).toBeInTheDocument();
});
