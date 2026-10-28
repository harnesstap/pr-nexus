import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import type { Api, ContextThread, SlackWorkspace } from "../api";
import { InlineCode, SlackLine } from "./SlackLine";

const acme: SlackWorkspace = {
  id: 1,
  team_name: "Acme",
  domain: "acme",
  user_name: "ada",
  signed_in: false,
  needs_sign_in: true,
  team_id: "T0ACME0001",
  enabled: true,
  priority: 0,
};
const thread: ContextThread = {
  workspace: "acme",
  channel_id: "C0ACME0001",
  channel_name: "reviews",
  root_ts: "1.0",
  permalink: null,
  status: "active",
  close_reason: null,
  requested_at: "2026-10-27T09:00:00Z",
  last_bumped_at: null,
  bump_count: 0,
  messages: [],
  has_more: false,
  fetched_at: null,
  from_cache: false,
  error: null,
};
const api = (threads: ContextThread[]) => ({ prSlackContext: () => Promise.resolve(threads) }) as unknown as Api;

test("Request review is disabled with a reason when every workspace needs sign-in", async () => {
  render(<SlackLine api={api([])} prId={1} onOpen={() => undefined} onRequest={() => undefined} workspaces={[acme]} />);
  const button = await screen.findByRole("button", { name: /Request review on Slack/ });
  expect(button).toBeDisabled();
  fireEvent.focus(button);
  expect(screen.getAllByRole("tooltip", { hidden: true }).some((tip) => tip.textContent?.includes("Acme Slack needs sign-in"))).toBe(true);
});

test("Bump is disabled for a thread in a workspace needing sign-in, enabled once signed in", async () => {
  const { unmount } = render(<SlackLine api={api([thread])} prId={1} onOpen={() => undefined} onRequest={() => undefined} workspaces={[acme]} />);
  expect(await screen.findByRole("button", { name: /Bump/ })).toBeDisabled();
  unmount();
  render(
    <SlackLine
      api={api([thread])}
      prId={1}
      onOpen={() => undefined}
      onRequest={() => undefined}
      workspaces={[{ ...acme, signed_in: true, needs_sign_in: false }]}
    />,
  );
  expect(await screen.findByRole("button", { name: /Bump/ })).toBeEnabled();
});

test("When sign-in and the bump interval both block Bump, the sign-in reason wins", async () => {
  const recent = { ...thread, requested_at: new Date(Date.now() - 3600_000).toISOString() };
  const { unmount } = render(
    <SlackLine api={api([recent])} prId={1} minBumpHours={24} onOpen={() => undefined} onRequest={() => undefined} workspaces={[acme]} />,
  );
  expect(await screen.findByRole("button", { name: /Bump · last/ })).toBeDisabled();
  fireEvent.focus(screen.getByTestId("bump-disabled"));
  expect(screen.getAllByRole("tooltip", { hidden: true }).some((tip) => tip.textContent?.includes("Acme Slack needs sign-in"))).toBe(true);
  unmount();
  render(
    <SlackLine
      api={api([recent])}
      prId={1}
      minBumpHours={24}
      onOpen={() => undefined}
      onRequest={() => undefined}
      workspaces={[{ ...acme, signed_in: true, needs_sign_in: false }]}
    />,
  );
  expect(await screen.findByRole("button", { name: /Bump · last/ })).toBeDisabled();
  fireEvent.focus(screen.getByTestId("bump-disabled"));
  expect(screen.getAllByRole("tooltip", { hidden: true }).some((tip) => tip.textContent?.includes("Too soon to bump"))).toBe(true);
});

test("the card shows the channel on one line, the message as a quote with inline code, and a muted bump reason", async () => {
  const recent = { ...thread, requested_at: new Date(Date.now() - 3600_000).toISOString() };
  render(
    <SlackLine
      api={api([recent])}
      prId={1}
      minBumpHours={6}
      onOpen={() => undefined}
      onRequest={() => undefined}
      workspaces={[{ ...acme, signed_in: true, needs_sign_in: false }]}
    />,
  );
  expect((await screen.findByTestId("slack-channel")).className).toContain("truncate");
  expect(screen.getByText(/Bump available in 5h/)).toBeTruthy();
});

test("InlineCode renders backticked spans as code", () => {
  const { container } = render(<InlineCode text="Review `Fix tax rounding` please" />);
  expect(container.querySelector("code")?.textContent).toBe("Fix tax rounding");
});
