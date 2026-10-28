import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { Api, ContextThread, MentionGroup } from "../api";
import { SlackContextPanel } from "./SlackContextPanel";

const active: ContextThread = {
  workspace: "acme",
  channel_id: "C0123456789",
  channel_name: "eng-web",
  root_ts: "1700000000.000100",
  permalink: "https://acme.slack.com/archives/C0123456789/p1700000000000100",
  status: "active",
  close_reason: null,
  requested_at: "2023-11-14T22:13:20Z",
  last_bumped_at: null,
  bump_count: 0,
  messages: [
    { ts: "1700000000.000100", at: "2023-11-14T22:13:20Z", author: "Ada Acme", user_id: "U0ACME0001", text: "Please review <https://github.com/acme/web/pull/1|acme/web#1> &amp; thanks", permalink: "https://acme.slack.com/archives/C0123456789/p1700000000000100", is_bot: false },
    { ts: "1700000300.000200", at: "2023-11-14T22:18:20Z", author: "Bob Acme", user_id: "U0ACME0002", text: "On it <@U0ACME0001>", permalink: "https://acme.slack.com/archives/C0123456789/p1700000300000200?thread_ts=1700000000.000100", is_bot: false },
  ],
  has_more: false,
  fetched_at: "2023-11-14T22:20:00Z",
  from_cache: false,
  error: null,
};

const closed: ContextThread = {
  ...active,
  channel_id: "D0123456789",
  channel_name: "",
  root_ts: "1690000000.000100",
  status: "closed",
  close_reason: "Superseded",
  messages: [],
  fetched_at: null,
  error: "Slack sign-in expired. Reconnect in Settings → Slack.",
};

test("shows known threads with replies, close reasons, errors, and refreshes past the cache", async () => {
  const prSlackContext = vi.fn((_id: number, refresh: boolean) => Promise.resolve(refresh ? [{ ...active, from_cache: false }] : [active, closed]));
  const onOpen = vi.fn();
  render(<SlackContextPanel api={{ prSlackContext } as unknown as Api} prId={7} mentions="off" onOpen={onOpen} />);
  const thread = await screen.findByRole("article", { name: "#eng-web thread" });
  expect(within(thread).getByText(/Requested · .* · 1 reply/)).toBeInTheDocument();
  expect(within(thread).getByText("Please review acme/web#1 & thanks")).toBeInTheDocument();
  expect(within(thread).getByText("On it @U0ACME0001")).toBeInTheDocument();
  const dm = screen.getByRole("article", { name: "DM thread" });
  expect(within(dm).getByText(/Closed: Superseded/)).toBeInTheDocument();
  expect(within(dm).getByText(/Slack sign-in expired/)).toBeInTheDocument();
  fireEvent.click(within(thread).getAllByRole("button", { name: "Open ↗" })[0]);
  expect(onOpen).toHaveBeenCalledWith(active.permalink);
  expect(screen.queryByRole("region", { name: "Mentions" })).not.toBeInTheDocument();
  expect(prSlackContext).toHaveBeenCalledWith(7, false);
  fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
  await waitFor(() => expect(prSlackContext).toHaveBeenLastCalledWith(7, true));
  await waitFor(() => expect(screen.queryByRole("article", { name: "DM thread" })).not.toBeInTheDocument());
});

test("signed out, a known thread still shows its stored last action and permalink", async () => {
  const bumped: ContextThread = {
    ...active,
    channel_id: "D0123456789",
    channel_name: "",
    requested_at: new Date(Date.now() - 30 * 60 * 60 * 1000).toISOString(),
    last_bumped_at: new Date(Date.now() - 9 * 60 * 60 * 1000).toISOString(),
    bump_count: 1,
    messages: [],
    fetched_at: null,
    error: "Slack sign-in expired. Reconnect in Settings → Slack.",
  };
  const onOpen = vi.fn();
  render(<SlackContextPanel api={{ prSlackContext: () => Promise.resolve([bumped]) } as unknown as Api} prId={7} mentions="off" onOpen={onOpen} />);
  const dm = await screen.findByRole("article", { name: "DM thread" });
  const status = within(dm).getByText(/Bumped · 9h ago/);
  expect(status).toHaveAccessibleDescription("requested 1d ago · 1 bump");
  expect(within(dm).getByText(/Open the thread in Slack to read the replies/)).toBeInTheDocument();
  fireEvent.click(within(dm).getByRole("button", { name: "Open ↗" }));
  expect(onOpen).toHaveBeenCalledWith(active.permalink);
});

test("an unrequested PR says so; mentions search runs on click when enabled", async () => {
  const groups: MentionGroup[] = [
    {
      workspace: "acme",
      hits: [{ channel_id: "C0123456780", channel_name: "deploys", ts: "1700000400.000100", author: "Devel", text: "Deployed <https://github.com/acme/web/pull/1>", permalink: "https://acme.slack.com/archives/C0123456780/p1700000400000100", is_bot: true }],
      error: null,
    },
    { workspace: "acme-ops", hits: [], error: "Slack is rate limiting. Try again after 10:00 UTC." },
  ];
  const prSlackMentions = vi.fn(() => Promise.resolve(groups));
  const api = { prSlackContext: () => Promise.resolve([]), prSlackMentions } as unknown as Api;
  render(<SlackContextPanel api={api} prId={7} mentions="public" onOpen={() => undefined} />);
  expect(await screen.findByText("No Slack request for this PR yet.")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /Find mentions \(public channels\)/ }));
  expect(await screen.findByText("Deployed https://github.com/acme/web/pull/1")).toBeInTheDocument();
  expect(screen.getByText(/acme-ops: Slack is rate limiting/)).toBeInTheDocument();
  expect(prSlackMentions).toHaveBeenCalledWith(7);
});

test("a panel load failure shows the error", async () => {
  const api = { prSlackContext: () => Promise.reject(new Error("store locked")) } as unknown as Api;
  render(<SlackContextPanel api={api} prId={7} mentions="off" onOpen={() => undefined} />);
  expect(await screen.findByText(/store locked/)).toBeInTheDocument();
});
